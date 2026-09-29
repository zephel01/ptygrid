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
import { modeResetSequence, type ModeResetScope } from "./termModes";

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
export const SCROLL_BOTTOM_SHORTCUT = IS_MAC ? "⌘End" : "Ctrl+Shift+End";

/** Layout-tolerant letter match: physical key first, produced character as fallback
 * (with Shift held, `key` is "C"/"V", hence the toLowerCase). */
function isLetter(ev: KeyboardEvent, letter: "c" | "v"): boolean {
  const code = letter === "c" ? "KeyC" : "KeyV";
  return ev.code === code || ev.key.toLowerCase() === letter;
}

// ---- scrollback preservation -----------------------------------------
//
// `ESC[3J` (ED 3, "erase saved lines") wipes xterm's scrollback. Claude Code
// and `clear` emit it, which is exactly when an operator loses the log they
// wanted to read back. With this on (default), ED 3 is swallowed; ED 0/1/2
// still work, so the visible screen clears as usual. Toggle without a UI:
//   localStorage.setItem("ptygrid.preserveScrollback", "0")  // then reload
const PRESERVE_SCROLLBACK_KEY = "ptygrid.preserveScrollback";

function preserveScrollback(): boolean {
  try {
    return localStorage.getItem(PRESERVE_SCROLLBACK_KEY) !== "0";
  } catch {
    return true; // storage unavailable → keep the default
  }
}

/** Buffer/scroll state a pane needs for its "jump to latest" affordance. */
export type ScrollState = {
  /** Viewport is above the live bottom of the normal buffer. */
  scrolledUp: boolean;
  /** The alternate screen (tmux/vim/full-screen TUIs) is active — no xterm scrollback. */
  altBuffer: boolean;
};

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
  /** Current scroll state (see ScrollState). */
  scrollState(): ScrollState;
  /** Subscribe to scroll-state changes. Returns an unsubscribe function. */
  onScrollState(listener: (state: ScrollState) => void): () => void;
  /** Jump the viewport to the live bottom. */
  scrollToBottom(): void;
  /** Undo stale alt-screen / mouse / input modes (see termModes.ts). Returns true when anything was written. */
  resetModes(scope: ModeResetScope): boolean;
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

/** Undo stale terminal modes in a session's xterm (see termModes.ts). */
export function resetTermModes(id: number, scope: ModeResetScope): boolean {
  return handles.get(id)?.resetModes(scope) ?? false;
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

  // ---- scrollback ------------------------------------------------------
  //
  // ED 3 guard (see preserveScrollback above). Returning true marks the
  // sequence handled, so xterm's own ED handler (which would drop the saved
  // lines) never runs; any other ED falls through to the default.
  term.parser.registerCsiHandler({ final: "J" }, (params) => {
    return params.length > 0 && params[0] === 3 && preserveScrollback();
  });

  // Wheel → scrollLines, driven here instead of by xterm's Viewport.
  // xterm scrolls the normal buffer by moving the native scrollTop of
  // `.xterm-viewport` and deriving the buffer position from it; in the macOS
  // WKWebView the panes did not scroll at all, even with plenty of
  // scrollback. term.scrollLines() moves the buffer position directly (the
  // viewport's scrollTop is then synced from it), so it does not depend on
  // the DOM scroll path. Scope is deliberately narrow:
  //   - only the NORMAL buffer with scrollback; the alternate screen (tmux,
  //     vim, full-screen TUIs) keeps xterm's default (wheel → ↑/↓, or mouse
  //     reports when the program enabled mouse tracking — xterm never calls
  //     this handler in that case);
  //   - Shift+wheel is left alone (xterm treats it as horizontal).
  let wheelPartial = 0;
  function rowHeightPx(): number {
    const screen = term.element?.querySelector<HTMLElement>(".xterm-screen");
    const h = screen && term.rows > 0 ? screen.clientHeight / term.rows : 0;
    return h > 0 ? h : 16;
  }
  term.attachCustomWheelEventHandler((ev) => {
    if (disposed) return true;
    const buf = term.buffer.active;
    if (buf.type !== "normal" || buf.baseY === 0) return true;
    if (ev.deltaY === 0 || ev.shiftKey) return true;
    let lines: number;
    if (ev.deltaMode === WheelEvent.DOM_DELTA_LINE) {
      lines = Math.round(ev.deltaY);
    } else if (ev.deltaMode === WheelEvent.DOM_DELTA_PAGE) {
      lines = Math.round(ev.deltaY) * term.rows;
    } else {
      wheelPartial += ev.deltaY / rowHeightPx();
      lines = Math.trunc(wheelPartial);
      wheelPartial -= lines;
    }
    if (lines !== 0) term.scrollLines(lines);
    ev.preventDefault();
    return false;
  });

  // Keyboard scrolling for keyboards without PageUp/Home/End (MacBook).
  // Shift+PageUp/PageDown already work (xterm built-in). Normal buffer only:
  // in the alternate screen these chords reach the program as before.
  //   macOS:        ⌘↑ / ⌘↓ page,  ⌘Home / ⌘End (fn+⌘←/→) top / bottom
  //   Linux/Win:    Ctrl+Shift+↑/↓ page, Ctrl+Shift+Home/End top / bottom
  function handleScrollKey(ev: KeyboardEvent): boolean {
    const chord = IS_MAC
      ? ev.metaKey && !ev.ctrlKey && !ev.altKey && !ev.shiftKey
      : ev.ctrlKey && ev.shiftKey && !ev.metaKey && !ev.altKey;
    if (!chord || term.buffer.active.type !== "normal") return false;
    switch (ev.key) {
      case "ArrowUp":
        term.scrollPages(-1);
        return true;
      case "ArrowDown":
        term.scrollPages(1);
        return true;
      case "Home":
        term.scrollToTop();
        return true;
      case "End":
        term.scrollToBottom();
        return true;
      default:
        return false;
    }
  }

  // Scroll-state notifications for the pane's "jump to latest" button.
  // term.onScroll does not fire for scrolls that start in the DOM viewport
  // (xterm suppresses it there), so the viewport's native `scroll` event is
  // observed too (wired in attach(), once the element exists).
  const scrollListeners = new Set<(state: ScrollState) => void>();
  let lastScrolledUp = false;
  let lastAlt = false;
  function computeScrollState(): ScrollState {
    const buf = term.buffer.active;
    const altBuffer = buf.type !== "normal";
    return { scrolledUp: !altBuffer && buf.viewportY < buf.baseY, altBuffer };
  }
  function emitScrollState(): void {
    if (disposed) return;
    const st = computeScrollState();
    if (st.scrolledUp === lastScrolledUp && st.altBuffer === lastAlt) return;
    lastScrolledUp = st.scrolledUp;
    lastAlt = st.altBuffer;
    for (const l of scrollListeners) l(st);
  }
  term.onScroll(emitScrollState);
  term.onWriteParsed(emitScrollState);
  term.buffer.onBufferChange(emitScrollState);
  let viewportScrollWired = false;

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

    if (handleScrollKey(ev)) {
      ev.preventDefault();
      return false;
    }

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
      if (!viewportScrollWired && term.element) {
        // Capture phase: `scroll` does not bubble, but it does reach an
        // ancestor's capturing listener. term.element survives re-parenting,
        // so this is wired exactly once and dies with term.dispose().
        term.element.addEventListener("scroll", emitScrollState, {
          capture: true,
          passive: true,
        });
        viewportScrollWired = true;
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
    scrollState: computeScrollState,
    onScrollState(listener) {
      scrollListeners.add(listener);
      return () => scrollListeners.delete(listener);
    },
    scrollToBottom() {
      if (!disposed) term.scrollToBottom();
    },
    resetModes(scope) {
      if (disposed) return false;
      const seq = modeResetSequence(scope, {
        altBuffer: term.buffer.active.type === "alternate",
        mouseTracking: term.modes.mouseTrackingMode,
      });
      if (seq === "") return false;
      term.write(seq);
      return true;
    },
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
      scrollListeners.clear();
      term.dispose();
      handles.delete(id);
    },
  };

  return handle;
}
