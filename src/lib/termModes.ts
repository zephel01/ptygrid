// Stale terminal-mode recovery (pure helpers; the side effects live in
// terminals.ts / stores.svelte.ts).
//
// A full-screen program (tmux, vim, Claude Code, …) switches xterm into the
// alternate screen (DEC 1049) and often turns on mouse tracking. It is
// expected to undo both on exit. When it cannot — killed, crashed, or an
// `ssh host` running tmux whose link dropped — the pane stays in the
// alternate screen with the shell prompt drawn on top of it. The alternate
// screen has no scrollback, so xterm turns the wheel into ↑/↓ keys and the
// operator sees their shell history cycling instead of scrolling (the
// reported symptom; `tput rmcup` fixed it by hand).
//
// ptygrid can see the two moments at which such state is certainly stale:
//   - the PTY process is gone (exit / restart / managed-ssh reconnect);
//   - the pane's foreground process went from a program back to a SHELL.
//     A shell itself never uses the alternate screen or mouse tracking, so
//     a shell in the foreground with either still on is leftover state.
// At those moments the pane's xterm (never the PTY) is sent the "off"
// sequences. Nothing is written to the running program.

/** Why the modes are being reset. */
export type ModeResetScope =
  /** The PTY process exited or is being respawned. */
  | "processGone"
  /** The foreground returned from a program to the shell. */
  | "shellReturned";

/** Subset of xterm state the decision needs (keeps this module xterm-free). */
export type TermModeSnapshot = {
  altBuffer: boolean;
  /** xterm `term.modes.mouseTrackingMode` ("none" | "x10" | "vt200" | "drag" | "any"). */
  mouseTracking: string;
};

export const LEAVE_ALT_SCREEN = "\x1b[?1049l";
export const MOUSE_TRACKING_OFF =
  "\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1015l";
export const SHOW_CURSOR = "\x1b[?25h";
/** Application cursor keys off + bracketed paste off. Only when the process
 * is gone: a live shell (zsh zle) may have just enabled these itself. */
export const INPUT_MODES_OFF = "\x1b[?1l\x1b[?2004l";

/**
 * Escape sequence to write into the pane's xterm, or "" when nothing is
 * stale. "shellReturned" acts only on real evidence (alt screen or mouse
 * tracking still on) and leaves input modes alone; "processGone" also clears
 * input modes since no process owns them any more.
 */
export function modeResetSequence(
  scope: ModeResetScope,
  state: TermModeSnapshot,
): string {
  const mouseOn = state.mouseTracking !== "none";
  if (scope === "shellReturned" && !state.altBuffer && !mouseOn) return "";
  let seq = "";
  if (state.altBuffer) seq += LEAVE_ALT_SCREEN;
  if (mouseOn) seq += MOUSE_TRACKING_OFF;
  if (scope === "processGone") seq += INPUT_MODES_OFF;
  if (seq !== "" || scope === "processGone") seq += SHOW_CURSOR;
  return seq;
}

// Interactive shells as reported by the foreground sampler (process name,
// possibly a login shell's "-zsh" or a full path).
const SHELLS = new Set([
  "sh",
  "bash",
  "zsh",
  "fish",
  "dash",
  "ksh",
  "mksh",
  "tcsh",
  "csh",
  "nu",
  "elvish",
  "xonsh",
  "pwsh",
  "powershell",
  "powershell.exe",
  "pwsh.exe",
  "cmd.exe",
]);

export function isShellName(name: string | undefined | null): boolean {
  if (!name) return false;
  const base = name.replace(/^-/, "").split(/[\\/]/).pop() ?? "";
  return SHELLS.has(base.toLowerCase());
}

/** True on the edge "a non-shell program was in the foreground → a shell is". */
export function returnedToShell(
  prev: string | undefined,
  next: string | undefined,
): boolean {
  return !!prev && !!next && !isShellName(prev) && isShellName(next);
}
