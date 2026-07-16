// Pure helpers for the toolbar "bulk cd" feature (send `cd <dir>` + Enter to
// several panes at once). No Svelte runes / Tauri imports here so the target
// selection and command-string building can be reasoned about and unit-tested
// in isolation from the UI.

import type { SessionInfo } from "./types";

/**
 * Foreground process base-names treated as an interactive shell.
 * Matched case-insensitively after stripping any directory part and a trailing
 * `.exe`. Extend this list rather than loosening the match.
 */
const SHELL_NAMES = new Set([
  "sh",
  "bash",
  "zsh",
  "fish",
  "dash",
  "ash",
  "ksh",
  "mksh",
  "tcsh",
  "csh",
  "nu",
  "xonsh",
  "elvish",
  "pwsh",
  "powershell",
  "cmd",
]);

/**
 * True when a foreground process name looks like an interactive shell.
 *
 * The name is lower-cased, reduced to its base name (any `/` or `\` directory
 * part dropped) and stripped of a trailing `.exe`, then matched against
 * {@link SHELL_NAMES}.
 *
 * IMPORTANT: an **undefined / empty** foreground is treated as a shell. The
 * backend only fills `SessionInfo.foreground` from list_sessions/list_agents
 * (session-state events omit it), so a pane we have no foreground info for must
 * not be silently dropped from the default (shell-only) target set — otherwise
 * a freshly spawned shell would be excluded just because its foreground has not
 * been sampled yet.
 */
export function isShellForeground(foreground: string | undefined): boolean {
  if (!foreground) return true;
  const trimmed = foreground.trim().toLowerCase();
  if (trimmed === "") return true;
  const base = trimmed.split(/[\\/]/).pop() ?? "";
  const name = base.endsWith(".exe") ? base.slice(0, -4) : base;
  return SHELL_NAMES.has(name);
}

/**
 * Choose which open panes should receive a bulk `cd`, preserving input order.
 *
 * Always excluded:
 *  - transcript panes (`kind === "transcript"`): read-only, cannot take input.
 *  - panes not currently `running` (starting / restarting / exited).
 *
 * When `includeNonShell` is `false` (the default): additionally keep only panes
 * whose foreground is a shell per {@link isShellForeground} (unknown foreground
 * counts as a shell — see that function).
 *
 * When `includeNonShell` is `true`: keep every running pty pane regardless of
 * foreground (this is the "also send to panes running a CLI" opt-in). Transcript
 * panes are still excluded.
 *
 * @param sessions ordered SessionInfo for the currently open panes
 * @param includeNonShell whether to also target running non-shell pty panes
 */
export function selectCdTargets(
  sessions: SessionInfo[],
  includeNonShell: boolean,
): SessionInfo[] {
  return sessions.filter((s) => {
    // transcript panes are always excluded (kind defaults to "pty")
    if ((s.kind ?? "pty") !== "pty") return false;
    // only a running pty can accept `cd` cleanly
    if (s.state !== "running") return false;
    if (includeNonShell) return true;
    return isShellForeground(s.foreground);
  });
}

/**
 * Build a POSIX `cd` command line for `dir`, safely quoted.
 *
 * The directory is wrapped in single quotes so spaces and shell metacharacters
 * (`$`, `*`, `;`, quotes, …) are passed literally; each embedded single quote
 * is emitted as the classic `'\''` idiom (close-quote, escaped quote, reopen).
 *
 * A leading `~` or `~/` is kept OUTSIDE the quotes so the shell still expands
 * the home directory (inside single quotes `~` is a literal character); the
 * remainder is still quoted. A `~name` form (named home) is not special-cased
 * and is quoted whole. No trailing newline is added — the caller appends `\r`.
 *
 * Examples:
 *   buildCdCommand("/a b/it's")  => cd '/a b/it'\''s'
 *   buildCdCommand("~/works/x")  => cd ~'/works/x'
 *   buildCdCommand("~")          => cd ~
 */
/**
 * Resolve a bulk-cd directory input against an optional projects root.
 *
 * Rules (applied after trimming leading/trailing whitespace):
 *  - An **absolute** path (`/...`) or a **home-relative** path (`~` / `~/...`)
 *    is returned unchanged — the user asked for an explicit location.
 *  - Any other value is treated as a **bare project name** relative to `root`:
 *    when `root` is a non-empty string the result is `<root>/<input>`
 *    (a single trailing `/` on `root` is not doubled); when `root` is null or
 *    empty the input is returned as-is (best effort — the shell resolves it
 *    against its own cwd).
 *  - An empty input returns `""` regardless of `root`.
 *
 * The returned string is a path, not a shell command; feed it to
 * {@link buildCdCommand} (which handles the leading `~` and quoting). So a
 * `~`-prefixed root flows through untouched: `resolveCdInput("notemake",
 * "~/works/project")` => `~/works/project/notemake`.
 *
 * Examples:
 *   resolveCdInput("notemake", "~/works/project") => "~/works/project/notemake"
 *   resolveCdInput("/etc",     "~/works/project") => "/etc"
 *   resolveCdInput("~/tmp",    "~/works/project") => "~/tmp"
 *   resolveCdInput("x",        null)              => "x"
 */
export function resolveCdInput(input: string, root: string | null): string {
  const trimmed = input.trim();
  if (trimmed === "") return "";
  if (trimmed.startsWith("/") || trimmed === "~" || trimmed.startsWith("~/")) {
    return trimmed;
  }
  const base = (root ?? "").trim();
  if (base === "") return trimmed;
  return `${base.replace(/\/+$/, "")}/${trimmed}`;
}

export function buildCdCommand(dir: string): string {
  const quote = (s: string): string => "'" + s.replace(/'/g, "'\\''") + "'";
  // Split off a leading ~ that is followed by end-of-string or a path separator.
  if (/^~(?=$|\/)/.test(dir)) {
    const rest = dir.slice(1);
    return `cd ~${rest ? quote(rest) : ""}`;
  }
  return `cd ${quote(dir)}`;
}
