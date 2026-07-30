// Registry of live xterm instances, keyed by session id.
//
// Terminal instances are created once per session and survive pane layout
// changes (moving between grid rows re-parents term.element instead of
// recreating the terminal, so scrollback is preserved). Disposal happens
// only when a pane is explicitly closed (disposeTermHandle) — never on
// component unmount, and kill_pty is never called from here.

import { Terminal as XTerm } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { isTauri } from "./tauri";
import type { PtyOutputPayload } from "./types";

// Exact font stack per CONTRACT.md (Nerd Font glyph fallback chain).
const FONT_FAMILY =
  "'MesloLGS NF','Hack Nerd Font Mono','JetBrainsMono Nerd Font Mono','Symbols Nerd Font Mono',Menlo,monospace";

// Platform detection for the copy/paste key bindings.
//
// It has to be synchronous (attachCustomKeyEventHandler must decide before the
// event is dispatched), which rules out Tauri's async platform APIs
// (@tauri-apps/plugin-os is not a dependency here, and `invoke` is a promise).
// `navigator.userAgentData` is not implemented in WebKit (macOS WKWebView /
// Linux WebKitGTK), so the classic `navigator.platform` ("MacIntel" on macOS)
// is the only reliable synchronous signal; the UA string is the fallback for
// the day WebKit finally drops `platform`.
const IS_MAC = (() => {
  if (typeof navigator === "undefined") return false;
  const platform = navigator.platform || "";
  if (platform) return /mac/i.test(platform);
  return /mac/i.test(navigator.userAgent || "");
})();

/** Shortcut hints for the pane context menu. Symbols/ASCII — not translated. */
export const COPY_SHORTCUT = IS_MAC ? "⌘C" : "Ctrl+Shift+C";
export const PASTE_SHORTCUT = IS_MAC ? "⌘V" : "Ctrl+Shift+V";

/** Layout-tolerant letter match: physical key first, produced character as fallback
 * (with Shift held, `key` is "C"/"V", hence the toLowerCase). */
function isLetter(ev: KeyboardEvent, letter: "c" | "v"): boolean {
  const code = letter === "c" ? "KeyC" : "KeyV";
  return ev.code === code || ev.key.toLowerCase() === letter;
}

export type TermHandle = {
  term: XTerm;
  /** Mount (or re-mount) the terminal element into a container. */
  attach(container: HTMLElement): void;
  /** Remove the terminal element from `container` if it is still there. */
  detach(container: HTMLElement): void;
  write(data: string): void;
  /** fit() the terminal to its container and sync the PTY size (debounce is the caller's job). */
  fitAndSync(): void;
  /** True when the pane currently holds a non-empty selection (drives the context menu). */
  hasSelection(): boolean;
  /** Copy the selection to the clipboard. Resolves false when nothing was selected. Rejects on clipboard failure. */
  copySelection(): Promise<boolean>;
  /** Read the clipboard and feed it to the PTY. Rejects on clipboard failure. */
  pasteFromClipboard(): Promise<void>;
  dispose(): void;
};

const handles = new Map<number, TermHandle>();
const pending = new Map<number, Promise<TermHandle>>();
// Ids whose TermHandle creation is still in flight but which were disposed
// before the creation resolved. The resolved handle must be disposed (not
// registered), otherwise a closed pane's xterm + pty-output listener leak.
const canceledPending = new Set<number>();

export function getTermHandle(id: number): TermHandle | undefined {
  return handles.get(id);
}

/** Write text locally into a session's terminal (exit banners, restart dividers). */
export function writeToTerm(id: number, data: string): void {
  handles.get(id)?.write(data);
}

export function disposeTermHandle(id: number): void {
  const existing = handles.get(id);
  if (existing) {
    existing.dispose(); // dispose() removes it from `handles`
  } else if (pending.has(id)) {
    // Creation still in flight: flag it so the resolved handle is disposed
    // instead of registered (prevents a resurrected leak — see BUG-1).
    canceledPending.add(id);
  }
  pending.delete(id);
}

export async function ensureTermHandle(id: number): Promise<TermHandle> {
  const existing = handles.get(id);
  if (existing) return existing;
  const inFlight = pending.get(id);
  if (inFlight) return inFlight;
  const creation = createTermHandle(id).then((handle) => {
    pending.delete(id);
    // If the pane was closed while this creation was in flight, dispose the
    // freshly built handle instead of registering it.
    if (canceledPending.has(id)) {
      canceledPending.delete(id);
      handle.dispose();
      return handle;
    }
    handles.set(id, handle);
    return handle;
  });
  pending.set(id, creation);
  return creation;
}

async function createTermHandle(id: number): Promise<TermHandle> {
  const term = new XTerm({
    theme: {
      background: "#1e1e1e",
      foreground: "#d4d4d4",
      cursor: "#d4d4d4",
    },
    fontFamily: FONT_FAMILY,
    fontSize: 13,
    cursorBlink: true,
    scrollback: 5000,
    // Let Option(Alt)+drag select text even while a TUI has mouse reporting on
    // (tmux/vim/claude). Option name verified in
    // node_modules/@xterm/xterm/typings/xterm.d.ts:198.
    // Caveat worth knowing: xterm's SelectionService.shouldForceSelection()
    // only consults this flag on macOS (`isMac ? altKey && option : shiftKey`),
    // so on Linux/Windows the escape hatch is *Shift*+drag and is hard-coded —
    // there is no option to move it to Alt.
    macOptionClickForcesSelection: true,
    // rightClickSelectsWord is left at its default (false) on purpose: the
    // pane's right-click opens our own copy/paste menu, and auto-selecting the
    // word under the cursor would make "Copy" never show its disabled state.
  });
  const fit = new FitAddon();
  term.loadAddon(fit);

  let unlistenOutput: (() => void) | undefined;
  let disposed = false;

  // ---- clipboard ------------------------------------------------------
  //
  // Copy writes through `navigator.clipboard.writeText` (the app's existing
  // write path): the Tauri capability grants clipboard-manager:allow-read-text
  // only, so the plugin cannot write.
  async function copySelection(): Promise<boolean> {
    if (disposed) return false;
    const text = term.getSelection();
    if (!text) return false;
    await navigator.clipboard.writeText(text);
    return true;
  }

  // Paste goes through `term.paste()` rather than invoking write_pty with the
  // raw string. Two reasons:
  //   1. bracketed paste — xterm's paste path runs prepareTextForTerminal
  //      (\r\n → \r) and wraps the text in \x1b[200~ … \x1b[201~ *only* when
  //      the running app enabled DEC 2004. Wrapping it ourselves would either
  //      double-wrap or send the markers to a shell that never asked for them.
  //      `ignoreBracketedPasteMode` stays at its default (false) so a
  //      bracketed-paste-aware shell keeps a multi-line paste inert until the
  //      user presses Enter.
  //   2. it still ends up on the existing PTY path: term.paste() fires
  //      term.onData(), i.e. invoke("write_pty", { id, data }) below.
  async function pasteFromClipboard(): Promise<void> {
    if (disposed) return;
    let text: string;
    if (isTauri()) {
      const { readText } = await import("@tauri-apps/plugin-clipboard-manager");
      text = await readText();
    } else {
      // Plain-browser (`vite dev`) fallback — the Tauri plugin is unavailable.
      text = await navigator.clipboard.readText();
    }
    if (!text || disposed) return;
    term.paste(text);
  }

  // Copy/paste key bindings. The handler runs before xterm's own key handling;
  // returning false makes xterm skip the event entirely *without* calling
  // preventDefault, so the WebView's native handling still runs.
  term.attachCustomKeyEventHandler((ev) => {
    // The handler is also called for keypress/keyup; act once, on keydown.
    if (ev.type !== "keydown") return true;

    const isCopyChord = IS_MAC
      ? ev.metaKey && !ev.ctrlKey && !ev.altKey && isLetter(ev, "c")
      : ev.ctrlKey && ev.shiftKey && !ev.metaKey && !ev.altKey && isLetter(ev, "c");
    if (isCopyChord) {
      // With an empty selection the chord must reach the PTY untouched so
      // Ctrl+C keeps sending SIGINT (and Cmd+C stays a no-op for the shell).
      if (!term.hasSelection()) return true;
      // macOS note: the native Edit▸Copy accelerator may swallow Cmd+C before
      // the WebView reports keydown (this handler then never runs) or fire in
      // addition to it. Unlike paste that is harmless — both paths copy
      // term.getSelection(), so the clipboard ends up with the same text.
      ev.preventDefault();
      copySelection().catch((err) => {
        console.error("copy to clipboard failed", err);
      });
      return false;
    }

    const isPasteChord = IS_MAC
      ? ev.metaKey && !ev.ctrlKey && !ev.altKey && isLetter(ev, "v")
      : ev.ctrlKey && ev.shiftKey && !ev.metaKey && !ev.altKey && isLetter(ev, "v");
    if (isPasteChord) {
      if (IS_MAC) {
        // macOS: do NOT paste here. The native Edit▸Paste accelerator makes
        // the WebView fire a real `paste` DOM event, and xterm already listens
        // for it (addDisposableDomListener(textarea, "paste", …) →
        // handlePasteEvent → coreService data event → write_pty). Reading the
        // clipboard here as well would insert the text twice — and we cannot
        // suppress the native path reliably either, because the menu
        // accelerator can consume Cmd+V before the WebView reports keydown, in
        // which case this handler never even runs. So the native path is the
        // single source of truth on macOS; here we only step aside (return
        // false, no preventDefault).
        return false;
      }
      // Linux/Windows: Ctrl+Shift+V produces no native paste event, so this is
      // the only paste path — no double-insert risk.
      ev.preventDefault();
      pasteFromClipboard().catch((err) => {
        console.error("paste from clipboard failed", err);
      });
      return false;
    }

    return true;
  });

  if (isTauri()) {
    const { invoke } = await import("@tauri-apps/api/core");
    const { listen } = await import("@tauri-apps/api/event");

    unlistenOutput = await listen<PtyOutputPayload>("pty-output", (event) => {
      if (event.payload.id === id) {
        term.write(event.payload.data);
      }
    });

    term.onData((data) => {
      invoke("write_pty", { id, data }).catch((err) => {
        console.error("write_pty failed", err);
      });
    });
  } else {
    // Plain-browser demo: local echo so `vite dev` alone shows something.
    term.writeln(
      `\x1b[1;33mNo Tauri runtime — local echo demo (pane #${id}).\x1b[0m`,
    );
    term.writeln("Type something and press Enter; it will be echoed back.\r\n");
    term.write("$ ");
    let line = "";
    term.onData((data) => {
      for (const ch of data) {
        if (ch === "\r") {
          term.write("\r\n");
          term.writeln(line);
          line = "";
          term.write("$ ");
        } else if (ch === "\x7f" || ch === "\b") {
          if (line.length > 0) {
            line = line.slice(0, -1);
            term.write("\b \b");
          }
        } else {
          line += ch;
          term.write(ch);
        }
      }
    });
  }

  const handle: TermHandle = {
    term,
    attach(container) {
      if (disposed) return;
      if (!term.element) {
        term.open(container);
      } else {
        container.appendChild(term.element);
      }
      requestAnimationFrame(() => handle.fitAndSync());
    },
    detach(container) {
      if (term.element && term.element.parentElement === container) {
        container.removeChild(term.element);
      }
    },
    write(data) {
      if (!disposed) term.write(data);
    },
    hasSelection() {
      return !disposed && term.hasSelection();
    },
    copySelection,
    pasteFromClipboard,
    fitAndSync() {
      if (disposed) return;
      const container = term.element?.parentElement;
      if (!container || container.clientWidth < 20 || container.clientHeight < 20) {
        return; // hidden (e.g. another pane is maximized) — skip
      }
      fit.fit();
      if (isTauri()) {
        import("@tauri-apps/api/core")
          .then(({ invoke }) =>
            invoke("resize_pty", { id, cols: term.cols, rows: term.rows }),
          )
          .catch((err) => {
            console.error("resize_pty failed", err);
          });
      }
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      // The clipboard work added no listener of its own that outlives the
      // terminal: the custom key handler lives on `term` (xterm has no detach
      // API for it) and dies with term.dispose() below, and the copy/paste
      // helpers bail out on `disposed`. The pty-output listener stays the only
      // thing that must be unhooked by hand (BUG-1).
      unlistenOutput?.();
      unlistenOutput = undefined;
      term.dispose();
      handles.delete(id);
    },
  };

  return handle;
}
