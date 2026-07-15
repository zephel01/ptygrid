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

export type TermHandle = {
  term: XTerm;
  /** Mount (or re-mount) the terminal element into a container. */
  attach(container: HTMLElement): void;
  /** Remove the terminal element from `container` if it is still there. */
  detach(container: HTMLElement): void;
  write(data: string): void;
  /** fit() the terminal to its container and sync the PTY size (debounce is the caller's job). */
  fitAndSync(): void;
  dispose(): void;
};

const handles = new Map<number, TermHandle>();
const pending = new Map<number, Promise<TermHandle>>();

export function getTermHandle(id: number): TermHandle | undefined {
  return handles.get(id);
}

/** Write text locally into a session's terminal (exit banners, restart dividers). */
export function writeToTerm(id: number, data: string): void {
  handles.get(id)?.write(data);
}

export function disposeTermHandle(id: number): void {
  handles.get(id)?.dispose();
  pending.delete(id);
}

export async function ensureTermHandle(id: number): Promise<TermHandle> {
  const existing = handles.get(id);
  if (existing) return existing;
  const inFlight = pending.get(id);
  if (inFlight) return inFlight;
  const creation = createTermHandle(id).then((handle) => {
    handles.set(id, handle);
    pending.delete(id);
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
  });
  const fit = new FitAddon();
  term.loadAddon(fit);

  let unlistenOutput: (() => void) | undefined;
  let disposed = false;

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
      unlistenOutput?.();
      unlistenOutput = undefined;
      term.dispose();
      handles.delete(id);
    },
  };

  return handle;
}
