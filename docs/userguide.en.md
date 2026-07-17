**English** · [日本語 (Japanese)](userguide.md)

# ptygrid User Guide

This guide walks you through everything from installing ptygrid, to writing your `ptygrid.yml`,
to coordinating agents with Queen (the built-in MCP server).

## Table of Contents

1. [What is ptygrid](#what-is-ptygrid)
2. [Installing and Launching](#installing-and-launching)
3. [Understanding the Screen](#understanding-the-screen)
4. [Working with Panes](#working-with-panes)
5. [Git status / diff](#git-status--diff)
6. [ptygrid.yml Reference](#ptygridyml-reference)
7. [Worktree Isolation](#worktree-isolation)
8. [Session Restore](#session-restore)
9. [Setting Up Queen](#setting-up-queen)
10. [Teammates (receiving hooks)](#teammates-receiving-hooks)
11. [Queen Tool Reference](#queen-tool-reference)
12. [Team Presets (team_presets)](#team-presets-team_presets)
13. [Practical Recipes: Agent Coordination](#practical-recipes-agent-coordination)
14. [Stored Data and Safety](#stored-data-and-safety)
15. [Getting Help](#getting-help)

---

## What is ptygrid

ptygrid is an integrated terminal that runs multiple AI agent CLIs (Claude Code / Codex / Grok, and
others) side by side in split panes. But it does more than just line them up: through its built-in MCP
server, **Queen**, the agents running inside the panes can themselves "read other panes, send them
instructions, and spawn new agents."

## Installing and Launching

Prerequisites:

- Rust (install via rustup)
- Node.js 20+
- Git
- macOS: Xcode Command Line Tools
- Linux: Tauri system dependencies such as WebKitGTK 4.1

### Linux (Ubuntu / Debian, test support)

The baseline is Ubuntu 22.04 or Debian 12 and later. Install the dependencies needed for development
and building:

> The Linux build is in test support (beta) as of Phase 3.9. Builds and package generation are verified
> in CI, but we are still validating stable operation on real hardware across desktop environments and
> distributions.

```bash
sudo apt update
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

Launching for normal development is the same as on macOS. To build a Linux package, run the following:

```bash
npm install
npm run tauri dev
npm run bundle:linux   # .deb + AppImage
```

The artifacts are written to `src-tauri/target/release/bundle/deb/` and `appimage/`.
Even when launched from a desktop launcher, ptygrid restores the login-shell `PATH` at startup, so the
Claude Code / Codex / Grok / Git tools you installed can be launched from the PTY.

```bash
git clone https://github.com/zephel01/ptygrid.git
cd ptygrid
npm install
npm run tauri dev    # the first run takes a few minutes to build Rust
```

A window opens with `$SHELL` (zsh, etc.) running in a single pane.

> If you open it in a browser alone (`npm run dev`), there is no PTY, so you get a local-echo demo display.

## Understanding the Screen

- **Toolbar, left side**: the "+ Shell" button (adds a pane), the **working folder** input field plus a
  "Load" button (e.g. `~/works/hoge`; a leading `~` is allowed), and after loading, a badge showing where
  the config file came from (`Config: In-project / Launch folder / ~/.ptygrid / Default`) along with chips
  for the agents defined in ptygrid.yml (click to launch). On a successful load, the open shell panes
  automatically `cd` into the working folder.
- **Toolbar, right side**: the Git panel button, the combined CPU/memory total across all panes, the
  "● Queen :39237" badge, and the pane count.
  - 🟢 green = running / 🔴 red = stopped / ⚪ gray = disabled (`queen.enabled: false`)
  - Click it to copy the registration command for Claude Code (including the auth token) to the clipboard.
    The token is saved and stays valid after a restart, so you only need to register **once** (re-register
    only when you regenerate the token).

### Working-folder suggestions

To help prevent typos, the **working folder** input field offers suggestions (via `<datalist>`). The
suggestions list each folder directly under your **projects root** (where your projects live), formatted
as `<root>/<folder name>`. The separate "cd…" button and bulk-cd popover that used to exist have been
removed; "Load" now handles both confirming the working folder and doing the bulk `cd` (see "Load = cd"
below).

- **Automatic root memorization**: When "Load" succeeds, the **parent directory** of the working folder
  you loaded is automatically saved as the projects root (in `app-settings.json`; it persists even when
  you switch projects). If the parent is `/` or your home directory itself, it is too broad to serve as a
  project location, so it is not saved. Saving is best-effort — if it fails, you get no toast and nothing
  is disrupted.
- **Showing suggestions**: If a root is set, ptygrid fetches the **non-hidden folders** directly under the
  root when the app starts and when you focus the input field, and offers `<root>/<folder name>` (a leading
  `~` is kept as-is) as suggestions (sorted by name, up to 200 entries). If no root is set, no suggestions
  appear (and this is not an error).
- Pick a suggestion, or type a path directly like `~/works/hoge`, and press "Load." That loads the working
  folder and automatically `cd`s the open shell panes into the same folder.
- **Each pane**: the header shows `<name> #<id>` (ad-hoc panes show `shell #<id>`), a status dot, the CPU
  and memory usage of the entire process tree, and restart / close / maximize buttons.
- **Toast notifications**: appear in the top-right corner (auto-dismissing after 5 seconds) for things like
  detected changes to ptygrid.yml (Reload) and Queen's `notify` tool calls.

## Working with Panes

| Action | How |
|---|---|
| Add a shell pane | The "+ Shell" button in the toolbar |
| Launch an agent | Click an agent chip in the toolbar (or set `autostart: true` in ptygrid.yml) |
| Restart | The restart button in the pane header. Restarts with the same config **while keeping the pane and session ID** |
| Close | The close button in the pane header |
| Maximize / restore | The maximize button in the pane header |

- You can have **up to 9 panes**. Sessions launched via Queen's `spawn_agent` also get a pane added
  automatically (when the limit is reached, you get a banner notification, and the session itself keeps
  running).
- The session ID identifies a session within the current run of the app. After you quit the app and do a
  logical resume, a new ID is assigned, so re-check it via the header or `list_agents`.
- Output is stored per session in a ring buffer (256 KiB) and stays continuous across restarts.
- The CPU/memory display updates every second. CPU is summed with one core counted as 100%, so a session
  using multiple cores can exceed 100%. Memory is the total resident memory of the PTY child and all of its
  descendants. The `Σ CPU` display on the right side of the toolbar is the sum across all running sessions
  currently being monitored.

## Git status / diff

Press "Git" on the right side of the toolbar to show the current project's changed files and their unified
diff in the right-hand panel. Select a file, and you can toggle between `Working tree` and `Staged`.

- If a `ptygrid.yml` has been loaded, ptygrid uses the repository in the directory containing that file.
- If nothing has been loaded, it uses the current directory from which ptygrid was launched.
- It does not run external diff, textconv, or a pager.
- The diff display is truncated at 2 MiB, and the status display at 10,000 files.
- Selecting an untracked file also shows the new-file diff.

To stage or unstage, check the box for the target file and press `Stage` or `Unstage`. Simply expanding a
file row does not change the index.

Enter a message in the commit field and press `Commit staged changes` to commit only the currently staged
changes. Unstaged files are never added implicitly. The repository's pre-commit / commit-msg and other
hooks, along with signing settings, are applied just as with a normal `git commit`; if they fail, the Git
error is shown in the panel.

## ptygrid.yml Reference

Enter the folder you want to work on in the toolbar's **"working folder" field** (e.g. `~/works/hoge`; a
leading `~` expands to your home directory) and press "Load." The config file `ptygrid.yml` does not have
to live inside that working folder — it is searched for in the following order:

1. **Inside the working folder** — `<working folder>/ptygrid.yml` (if absent, the old name
   `<working folder>/mterm.yml`; the legacy-name fallback applies only inside the working folder)
2. **The app launch folder** — the `ptygrid.yml` in the folder ptygrid was launched from (for example,
   where you ran `npm run tauri dev`)
3. **Global config** — `~/.ptygrid/ptygrid.yml`

The first file found is loaded (if both exist, the `ptygrid.yml` inside the working folder takes highest
priority). After loading, a badge appears next to the Load button indicating **where it was read from**
(`Config: In-project / Launch folder / ~/.ptygrid / Default`); hover over it to see the actual path and the
working folder. The working folder is the **project boundary** that anchors cwd resolution, the Git panel,
Queen's project scope, and session restore — and no matter where the config file is read from, the working
folder is always what is used.

**"Load" behaves the same as cd**: When you press "Load" and it succeeds, the open shell panes are
automatically `cd`'d into the specified working folder (a `cd '<working folder>'` is sent). This targets
**only the running shell panes** (where `kind` is pty, the state is running, and the foreground is sh/bash/
zsh/fish, etc.; a pane whose foreground name cannot be determined is treated as a shell), and does not send
to panes running a CLI or to transcript (read-only) panes. Afterward a toast appears reading "Working
folder: … / cd sent to N panes." It is not an error if there are no panes, or if all panes are running a
CLI.

**It opens even without a config file**: Even if no `ptygrid.yml` exists in any of the three locations,
"Load" does not error — it succeeds with the **built-in default config** (no agent definitions; Queen
enabled), and the badge shows `Config: Default`. Even in this state, the `cd` into the working folder still
happens. If you later create `<working folder>/ptygrid.yml`, it is detected by the file watcher and can be
loaded from the "Reload" toast (the chips for the definitions you created then line up in the toolbar).

If you want to reuse a common set of definitions across multiple projects, put them in
`~/.ptygrid/ptygrid.yml` and just switch the working folder — you can target a different folder with the
same config.

### Trust confirmation (auto-start guard for unconfirmed folders)

A `ptygrid.yml` can run commands via `cmd` / `resume` / `worktree.setup`. The first time you load a
`ptygrid.yml` from someone else's repository (originating from the working folder or launch folder = the
`In-project` / `Launch folder` badge), ptygrid **holds back auto-starting any `autostart: true`
definitions** so that unintended commands don't launch on their own, and shows the following confirmation
banner:

> The config for this folder (&lt;working folder&gt;) has not been confirmed. Do you want to auto-start the
> defined commands?

- Pressing **"Trust and launch"** remembers that folder as trusted (in app-data's `trusted-folders.json`)
  and launches the `autostart` definitions that were held back. From then on, no confirmation appears for
  that same folder.
- Pressing **"Later"** auto-starts nothing. You can still browse the config contents and view panes as
  usual, and **manual launch via the ▶ on an agent chip works without a confirmation** (manual actions are
  not subject to the gate).
- Your own global config `~/.ptygrid/ptygrid.yml` (the `~/.ptygrid` badge) and the built-in default used
  when there is no config file (the `Default` badge) are **always treated as trusted**, and no confirmation
  appears.

Loaded files are watched (the global config watches `~/.ptygrid`; a launch-folder config watches that
folder), and when you change them, you can reload from the "Reload" toast.
Samples: [ptygrid.example.yml](../ptygrid.example.yml) (annotated) / [example/](../example/README.md) (by use case)

```yaml
project: my-app

queen:            # optional (omit the whole block for default behavior)
  enabled: true   # default true. false stops Queen
  port: 39237     # default 39237. if in use, tries +1 up to 39246

agents:           # interactive AI CLIs
  - name: claude
    cmd: "claude"
    cwd: "."                                   # relative paths (based on the directory containing ptygrid.yml) are allowed
    env:
      ANTHROPIC_API_KEY: "${ANTHROPIC_API_KEY}"  # ${VAR} expands host environment variables
    autostart: false
    autorestart: never                          # never | on-failure | always

processes:        # ordinary long-running processes (dev servers, etc.). the fields are the same as agents
  - name: web
    cmd: "npm run dev"
    autorestart: on-failure
```

### Field list

| Field | Required | Default | Description |
|---|---|---|---|
| `project` | - | - | Project name (for display) |
| `queen.enabled` | - | `true` | Enable/disable Queen (the built-in MCP server) |
| `queen.port` | - | `39237` | Queen's listening port. If in use, automatically tries +1 up to 39246 |
| `agents[].name` / `processes[].name` | ✅ | - | Display name. Also serves as the Queen destination name and the `spawn_agent` allowlist |
| `.cmd` | ✅ | - | Launch command |
| `.cwd` | - | location of ptygrid.yml | Working directory. Relative paths are resolved against ptygrid.yml |
| `.env` | - | - | Environment variables. `${VAR}` in a value expands from the host environment (undefined becomes an empty string) |
| `.autostart` | - | `false` | Auto-start when the config is loaded |
| `.autorestart` | - | `never` | `never` / `on-failure` / `always`. Gives up after 5 consecutive failures |
| `.resume` | - | `.cmd` | Command used for logical resume after an app restart |
| `.worktree.enabled` | - | `false` | Create a linked worktree and dedicated branch each time the definition launches |
| `.worktree.base` | - | `HEAD` | The branch/tag/commit the worktree branch starts from |
| `.worktree.setup` | - | - | A setup command run once in the agent cwd after the worktree is created |

> Every session has the environment variable `QUEEN_URL` injected (e.g.
> `http://127.0.0.1:39237/mcp?token=<token>`; it includes the auth token). To check the connection target
> from inside a pane, run `echo $QUEEN_URL`.

## Worktree Isolation

When multiple agents editing the same repository at once would conflict, you can enable worktree isolation
per definition. It is disabled by default, and as before, all agents share the same workspace.

```yaml
agents:
  - name: codex
    cmd: codex
    cwd: packages/app
    worktree:
      enabled: true
      base: HEAD
      setup: npm install
```

When you launch a definition that has it enabled, ptygrid creates a unique linked worktree under app-data
along with a `ptygrid/codex/...` branch, and displays the branch name in the pane header. If `cwd` is a
subdirectory inside the repository, the launch happens from the same relative position inside the worktree.
Restart and autorestart reuse the same worktree. You can select a running worktree from `Workspace` at the
top of the Git panel to review that branch's diff and commit.

Worktrees are locked to avoid Git's automatic pruning, and ptygrid never deletes them automatically. Once
you have collected and committed your work, clean up worktrees you no longer need explicitly with the usual
Git commands. `<path>` and `<branch>` can be checked in the pane's branch display and its tooltip.

```bash
git worktree unlock <path>
git worktree remove <path>   # Git refuses if it is dirty
git branch -d <branch>
```

If setup or the agent launch fails, the worktree is kept, and the path is shown in the error. Do not delete
it with `--force` without checking its contents first.

## Session Restore

ptygrid automatically saves the last open project, pane order, column layout, and maximized state to
app-data. On the next launch, it re-reads the current `ptygrid.yml` and restarts the config definitions as
new PTYs. If an AI CLI has a command for resuming its conversation, you can specify it via `resume`.

```yaml
agents:
  - name: codex
    cmd: codex
    resume: codex resume --last
  - name: claude
    cmd: claude
    resume: claude --continue
```

A definition that omits `resume` re-runs `cmd`. For a normal launch or a pane restart, the command that
launched that session is used — not `resume`.

This feature is not a reconnection to an already-terminated process. Ad-hoc shells are also re-opened as a
fresh default shell, and their previous scrollback is not restored. A worktree session is reused after
confirming that the saved path is a valid linked worktree of the same repository, and its setup command is
not re-run.

The saved JSON does not include commands, terminal output, or environment variables. If the state file is
corrupted, the project directory has moved, or a definition has been removed, a restore error is shown on
screen.

## Agent status badge (agent_status)

For panes that are running, ptygrid infers the **semantic state of the agent** from the pane's terminal
output. This is a **separate layer** — an "inference" — from the existing status dot that represents
process liveness (starting up / running / exited), and it is overlaid on top of the live PTY (it does not
overwrite the status dot).

- 🔴 **blocked** — stopped waiting for approval or input (only when it matches a known approval/permission/
  selection UI; the detection is conservative to avoid false positives).
- 🟡 **working** — running (`esc to interrupt`, `Thinking`, etc.).
- 🔵 **done** — just after the most recent work finished (automatically decays to idle after a few seconds).
- 🟢 **idle** — alive but waiting (matches none of the patterns).
- ⚪ **unknown** — there is no rule set to infer the state (badge hidden).

Detection starts from built-in default patterns (`claude` / `codex` / `grok` / `aider`) and selects a rule
set based on each pane's agent definition name or its foreground process name. A `claude` / `codex` you
started by hand is also picked up by its foreground name. **The built-in patterns can become outdated as
each CLI changes its UI**, so override them in `ptygrid.yml` as needed (changes take effect immediately on
config reload).

```yaml
agent_status:
  enabled: true          # default true. false stops detection
  tail_lines: 24         # number of trailing lines used for detection (4..200)
  debounce_ms: 250       # evaluation interval (100..2000). doesn't overload even with bursty output
  done_linger_ms: 6000   # how long to hold done before decaying to idle (0..60000; 0 disables done)
  patterns:
    claude:              # by default, "appends (merges)" onto the built-in rules
      blocked:
        - 'Do you want to proceed\?'
      working:
        - 'esc to interrupt'
    codex:
      replace: true      # discard the built-in and fully replace
      blocked:
        - '\[y/N\]'
    "*":                 # a generic rule to apply to unassigned panes as well, only if you want it (opt-in)
      blocked:
        - '\[y/N\]\s*$'
```

By default patterns are case-insensitive and evaluated as multi-line partial matches (you can override this
individually with inline flags such as `(?-i)`). An invalid regular expression skips **just that one
pattern**, leaving the others in effect.

> Note: The badge UI itself is being rolled out incrementally, starting with the header display in this
> release (the status-list sidebar and approval-pending notifications come later). The `agent_status`
> settings are effective as of this release.

## Setting Up Queen

Queen is an MCP server that runs continuously inside the app (streamable HTTP, bound to 127.0.0.1 only).
Register it as an MCP server with each agent CLI, and that agent gains access to
[18 tools](#queen-tool-reference).

> 🔑 **About the auth token (important)**
> Queen is restricted to 127.0.0.1, but to prevent unauthorized access from other processes on the same
> host or from a web page that has performed DNS rebinding, `/mcp` is protected by an **auth token +
> Host/Origin verification**. The registration URL includes `?token=<token>`.
> **This token is saved in app-data and does not change even when you restart the app. You only need to
> register once.** Re-registration is required only when you regenerate the token (you can rotate it in the
> event of a leak via "Regenerate Queen token" in the Teammates panel). Always get the actual URL by
> clicking the "● Queen" badge in the toolbar to copy it (the `<token>` in the commands below is a
> placeholder).

### Claude Code

```bash
# <token> and <port> are replaced with real values when you copy from the badge
claude mcp add -s user --transport http queen "http://127.0.0.1:39237/mcp?token=<token>"
```

> ⚠️ **Always include `-s user`.** The default local scope registers "for the directory where the command
> was run only," so if you register somewhere other than the pane's working directory, Claude Code won't be
> able to see Queen (this has actually happened). If you want to share it per project, you can also use
> `-s project` (which creates a `.mcp.json` in the repository).
>
> The token stays valid after a restart, so normally no re-registration is needed. Only when you regenerate
> the token do you need to `claude mcp remove queen` and register again, or register over it.

### Codex CLI

Add to `~/.codex/config.toml` (include the token in the URL):

```toml
[mcp_servers.queen]
url = "http://127.0.0.1:39237/mcp?token=<token>"
```

### Grok CLI

```bash
grok mcp add -s user -t http queen "http://127.0.0.1:39237/mcp?token=<token>"
grok mcp doctor    # verify the connection (success shows handshake OK / 18 tools discovered)
```

> ℹ️ Because the token is passed as a URL query, no additional CLI settings such as `--header` are needed.
> If you really want to pass it via a header, `Authorization: Bearer <token>` is also accepted.

### About the port

If 39237 is in use, Queen automatically tries +1 up to 39246. If it falls back, you need to adjust the
registration URL accordingly (the toolbar badge shows the actual port). To pin it, specify `queen.port` in
your `ptygrid.yml`.

## Teammates (receiving hooks)

The **Teammates badge** on the right side of the toolbar is the entry point for receiving the teammate
lifecycle hooks that Claude Code and others fire (subagent start/stop, idle, task creation/completion). The
receiving endpoint is `/hooks/v1/*` on the same 127.0.0.1 server as Queen; it requires
`Authorization: Bearer <token>` and is non-blocking (always returns `200 {"decision":"allow"}`).

### Enabling

Add a global `teammates:` block to your `ptygrid.yml` (everything is optional):

```yaml
teammates:
  enabled: true             # default false. true enables hook-receipt notifications
  hook_notifications: true  # default true. whether to toast on receipt
  global_max_panes: 6       # default 6 (1..9). used in Phase 4.1
  hooks_scope: user         # "user" | "project". default "user"
```

Even while `enabled: false` (the default), token verification is still performed, but no event
notifications are shown. The badge is green when enabled and gray when disabled.

### Registering hooks

Clicking the badge opens the settings panel:

- **Copy snippet**: copies the hooks-definition JSON with the token embedded to the clipboard. Paste it
  into the `hooks` section of Claude Code's `settings.json`.
- **Register into settings.json (user)**: automatically merges the hooks definition into
  `~/.claude/settings.json` (existing content is preserved, a `settings.json.ptygrid-backup-<unix seconds>`
  is created before writing, and nothing is written if the content is identical).
- **Regenerate hook token / Regenerate Queen token**: for rotating a leaked token. Regenerates the target
  token and immediately applies it to the running auth layer (no Queen server restart needed). After
  regeneration, the settings.json / MCP registrations still hold the old token, so re-registration is
  required (the panel tells you).
- **Recent events**: shows up to the 10 most recent teammate-lifecycle events received.

> ✅ The token is saved in app-data (`auth-tokens.json`, permissions 0600 on Unix) and does not change even
> when you restart the app. You only need to register **once**. Only when you regenerate the token do you
> need to re-copy the snippet or re-register into settings.json.

### observe: read-only transcript panes (Phase 4.1)

When a lead (parent agent) launches a subagent, its transcript can be automatically added as a **read-only
pane**. To enable it, just add a `teams:` block to the lead's definition:

```yaml
teammates:
  enabled: true       # the global enable (above) is also required
agents:
  - name: claude
    cmd: claude
    cwd: "."
    teams:
      enabled: true         # turn transcript panes on for this lead
      mode: observe         # observe | host (host becomes a real PTY in Phase 4.2; see below)
      max_panes: 3          # cap on transcript panes this lead creates (default 3)
      transcript_tail: true # if false, notify only and create no pane (default true)
```

Usage and behavior:

- When Claude Code's `SubagentStart` hook fires, a `claude·sub #<id> ▸<role> 📖RO` pane appears that tails
  the subagent transcript under `~/.claude/`. The parent lead is shown alongside as `↳#<id>`.
- The pane is **read-only**. Rather than xterm, it is a scroll view that displays `role: text` in
  chronological order and summarizes tool calls to a single line. You cannot type into it (Queen's
  `send_message` is also rejected).
- The status dot is active (running) / stopped (subagent finished). When `SubagentStop` is received it
  becomes stopped, and the pane remains (showing its final state until you close it yourself).
- **Closing the pane does not affect the subagent** (it just stops the tail). It cannot be restarted.
- If a cap is exceeded (the per-lead `max_panes`, the overall `teammates.global_max_panes`, or the 9-pane
  grid), no pane is created and you get a banner notification in Japanese.
- For safety, only absolute paths under `$HOME/.claude/` are tailed. Anything else, or a path that cannot
  be determined, results in a status display only. Transcript sessions are not subject to session restore
  (resume).
- **How to launch**: Launching from the ▶ chip is reliable (as a named lead, its `teams:` config applies
  directly), but observe also works with a `claude` **typed by hand** into a shell pane. When there is not
  a single named lead with `teams.enabled`, a running pane whose foreground is `claude` (the default;
  configurable via `teammates.teammate_binaries`) is picked up as an **implicit observe lead** (this
  requires the global `teammates.enabled: true`; it is observe-only and never becomes a host). If a named
  lead exists, that one takes priority.
- If a subagent could not be attributed to a lead and no pane could be created, then when
  `teammates.enabled: true`, a banner notifies you that "a subagent was detected but no teams-enabled lead
  was found" (prompting you to launch from the ▶ chip or check `teammates.enabled`).

### host: real-PTY teammate panes (Phase 4.2, experimental, off by default)

With `mode: host`, ptygrid hosts Claude Code's split-pane teammates (independent `claude` processes) as
ptygrid's own **native interactive PTY panes**. Unlike read-only observe, you can type directly into the
teammate pane, and it is treated on par with a normal pane all the way through resize, scrollback, and
Queen connectivity. **This is an opt-in experimental feature, off by default.**

```yaml
teammates:
  enabled: true             # note: host is per-agent opt-in, so it does not depend on the global enabled
agents:
  - name: claude
    cmd: claude
    cwd: "."
    teams:
      enabled: true
      mode: host                 # observe | host. host provides real-PTY hosting
      max_panes: 3               # cap on this lead's teammate panes (1..9)
      teammate_binaries:         # argv0 basenames allowed to spawn a PTY via split-window (default ["claude"])
        - claude
      fallback_to_observe: true  # auto-downgrade to observe when host goes unused (default true)
```

Enabling and how it works:

- The condition for enabling is a lead with `enabled: true` **and** `mode: host` only. Without opt-in, no
  env injection, no socket server startup, and no shim placement happen **at all**. host is Unix-only (on
  Windows it launches as a normal session).
- When the lead launches, ptygrid **automatically places a tmux-compatible shim and a per-lead Unix socket
  server**, and auto-injects the necessary environment variables (`TMUX` / `TMUX_PANE` /
  `PTYGRID_TEAMS_SOCK` / `PTYGRID_TEAMS_TOKEN` / prepending the shim to `PATH`) into the lead PTY. **ptygrid
  also auto-injects `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1`**, so you don't have to set it manually. The
  principle is config-as-code; there is no temporary enabling from the UI.
- When Claude Code launches a teammate via split-window, an interactive PTY pane with a
  `claude·team #<id> ▸<role>` header appears. The parent lead is shown alongside as `↳#<id>`. The status dot
  is running / exited (+ exit code), the same as a normal PTY. It can be restarted (⟳) and maximized (⤢).
- **Closing a teammate pane kills the real process (a destructive action)**, so a confirmation ("Stop the
  teammate?") is inserted.
- **Fallback**: If no split-window RPC comes through the shim within 2 seconds of detecting the teammate,
  ptygrid concludes that Claude Code fell back to in-process (the shim was not used). If
  `fallback_to_observe: true`, it automatically downgrades to observe (a read-only transcript pane) and
  notifies you with a toast. During this time the Teammates badge shows "host: falling back."
- **Exceeding the cap**: Even when any of `teams.max_panes` / `teammates.global_max_panes` / the 9-pane
  grid is reached, host still creates the teammate session itself (so work isn't halted). However it is not
  placed on the grid — it is paneless — and you get a banner notification in Japanese. You can promote it
  via "Show on grid" from the list in the Teammates panel.
- **Orphaned teammates**: When a lead exits, its host teammate PTYs can be orphaned. The Teammates panel
  lists them as "lead exited (orphaned teammate)," and you can clean them up with the "Stop" button.
- Teammate spawns do not go through Queen's allowlist (`spawn_agent`); they are protected by three stages:
  (1) the config opt-in, (2) the socket token handshake, and (3) argv0-basename verification against
  `teammate_binaries` (default `["claude"]`). Teammate sessions are not subject to session restore
  (resume).

## Queen Tool Reference

| Tool | Arguments | Description |
|---|---|---|
| `list_agents` | none | A list of running sessions and ptygrid.yml definitions (with state and foreground process name) |
| `read_output` | `agent`, `lines?` (default 100, 1..1000), `raw?` (default false) | The most recent output of the given pane. By default, reconstructs ANSI cursor movement, screen clears, and the alternate screen to match the pane dimensions. Use `raw: true` for raw output |
| `send_message` | `agent`, `text`, `submit?` (default true) | Writes to the given pane's stdin. `submit: true` appends an Enter at the end |
| `spawn_agent` | `name` | Can launch **only names defined in ptygrid.yml** (allowlist approach) |
| `spawn_team` | `preset` | Launch a team declared under `team_presets:` in one call (see [Team Presets](#team-presets-team_presets)). Returns a launch report |
| `notify` | `title`, `message` | Shows an in-app toast notification |
| `set_pin` | `key`, `value`, `expectedRevision?` | Creates a short shared value within the project, or updates it safely. Updating an existing value requires the current revision |
| `list_pins` | none | Lists the project's pins and revisions by key order |
| `delete_pin` | `key`, `expectedRevision` | Deletes a pin only if the revision matches |
| `create_note` | `title`, `body`, `tags?` | Creates a durable note within the project |
| `list_notes` | `query?`, `limit?` (default 50, max 200) | Searches and lists notes, newest updated first |
| `get_note` | `id` | Fetches a single note by its stable ID |
| `update_note` | `id`, `expectedRevision`, `title?`, `body?`, `tags?` | Updates only the specified fields of a note whose revision matches |
| `delete_note` | `id`, `expectedRevision` | Deletes a note only if the revision matches |
| `send_inbox` | `sender`, `recipient`, `subject`, `body` | Sends a durable message to a stable mailbox. Does not type into a live PTY |
| `list_inbox` | `mailbox`, `afterId?`, `includeAcknowledged?`, `limit?` | Reads the inbox in ascending ID order. The default is unacknowledged only |
| `ack_inbox` | `id`, `recipient` | Idempotently acknowledges a message whose recipient matches |
| `reply_inbox` | `id`, `sender`, `body` | Sends a correlated reply from the original recipient and acknowledges the original message |
| `await` | `mailbox`, `afterId?`, `includeAcknowledged?`, `limit?`, `timeoutMs?` | Waits for inbox arrivals after a cursor, until timeout/cancel |

### Resolving the destination name (`agent`)

A pane launched from a definition displays a session ID like `codex #4`, and an ad-hoc shell like
`shell #5`. Even if you launch Codex manually inside a shell, the header name stays `shell`, but `codex`
appears in the `foreground` of `list_agents`. Check the current ID first, then specify the destination for
`read_output` / `send_message` using these rules:

1. **`#<id>`** — an exact session ID (e.g. `"#4"`). Recommended when there are multiple panes
2. **A ptygrid.yml definition name / session name** — when it matches exactly and there is only one running
   candidate
3. **A foreground process name** — when it matches exactly and there is only one candidate. This can also
   identify a `codex` / `claude` / `grok` you started manually inside a shell

When there are multiple panes with the same name or the same foreground process, ptygrid does not guess the
latest pane and send to it — instead it returns candidate IDs like `use one of: #2, #4`. For example, if
there are three Codex panes, tell it `agent: "#4"` rather than `agent: "codex"`. Giving definable panes role
names like `codex-impl`, `codex-review`, or `claude-test` makes the intent clear to both humans and agents.
Even when nothing is found, the error includes a list of running sessions (with foreground process names).

For example, if Codex is on two panes, `#3` and `#5`, you would make a request like this:

> Send "review the changes" to `#3`, and read the reply.

In the MCP tool arguments, that is `{ "agent": "#3", ... }`. `agent: "codex"` produces an ambiguity error,
so misdelivery to an unintended pane doesn't happen.

If you ask Claude Code to "`work in grok #2`" or "`ask codex #3 to review`," then as long as Queen is
connected, it treats these as references to existing panes: it checks the IDs with `list_agents` before
using `read_output` / `send_message`. It does not mean launching a new Grok/Codex process.

### Concurrent editing of Pins / Notes

Pins and Notes are separated by the directory of the loaded `ptygrid.yml` and persisted to a SQLite
database inside app-data. No management files are created inside the repository. Each record has a
monotonically increasing `revision`, and updates or deletes of an existing record are committed only when
the `expectedRevision` you fetched just beforehand matches.

If multiple agents update the same revision at once, only the first one to succeed wins. The rest get a
`conflict` and do not overwrite the new content or delete anything. Re-read the latest version with
`list_pins` / `get_note`, merge the content, then retry with the new revision. Different keys or note IDs
can be updated independently.

Recommended update procedure:

1. Read the value and `revision` with `list_pins` or `get_note`
2. Update the content and pass the fetched revision as `expectedRevision`
3. On a `conflict`, re-fetch the latest version, merge your change, and retry

For example, a pin sharing which pane is responsible for something is created without a revision on the
first call:

```json
{ "key": "task/owner", "value": "#3" }
```

If `set_pin` returns `revision: 1`, then to change it you use
`{ "key": "task/owner", "value": "#5", "expectedRevision": 1 }`. Save design decisions and long context in
`create_note` rather than a pin, and share the stable ID it returns.

### Inbox / Reply

The Inbox serves a different purpose from `send_message`. `send_message` types directly into a currently
running PTY, whereas the Inbox appends a project-scoped durable message to SQLite that the recipient can
read later.

For the Inbox's `sender` / `recipient` / `mailbox`, use stable role names like `codex-review` or
`claude-impl`. Session IDs like `#3` that change on app restart are rejected.

```json
{
  "sender": "claude-impl",
  "recipient": "codex-review",
  "subject": "Review request",
  "body": "Please check commit 71a483b"
}
```

The receiving side fetches unacknowledged messages with `list_inbox`. To reply, specify the message ID.

```json
{ "id": 12, "sender": "codex-review", "body": "No problems" }
```

`reply_inbox` sends the reply to the original sender and, while maintaining the thread via `inReplyToId`
and `rootMessageId`, acknowledges the original message in the same transaction. If you are not replying,
use `ack_inbox`. Re-running the same ack does not corrupt state. The default `list_inbox` excludes
acknowledged messages, so specify `includeAcknowledged: true` only when you need the history.

As of Phase 3.7, the MCP client is not authenticated-bound to a mailbox, so sender/recipient are explicit
values. Queen is localhost-only, but do not treat mailbox names as an access-control boundary. The limits
are 256 bytes for a subject, 64 KiB for a body, and 50,000 messages per project. Updating or deleting a
message body is not provided, so make corrections by sending a new message or reply.

### Waiting on the Inbox (`await`)

Instead of repeating `list_inbox` at short intervals, you can wait for a new message to arrive with
`await`.

```json
{
  "mailbox": "codex-review",
  "afterId": 12,
  "timeoutMs": 30000
}
```

- If a matching message after ID 12 already exists, it returns immediately
- On arrival, it returns `messages`, the maximum ID as `nextCursor`, and `timedOut: false`
- At the deadline, it returns normally with empty `messages`, the input cursor, and `timedOut: true`
- The default timeout is 30 seconds; the allowed range is 1 ms to 5 minutes
- If the MCP client cancels the request, it terminates immediately with a cancellation error, leaving the
  Inbox unchanged

On the next call, pass the previously returned `nextCursor` as `afterId`. `await` itself does not
acknowledge a message, so after you finish processing, call `ack_inbox` or `reply_inbox`.

## Team Presets (team_presets)

Declare a **named team composition** in `ptygrid.yml` and launch it in a single action
(Phase 4.3). Members are **references to `agents:` definitions only**, so a preset can never
launch anything the `spawn_agent` allowlist would not allow.

```yaml
team_presets:
  daily:                        # preset name (shown as a 👥 chip in the toolbar)
    lead: local                 # optional: kickoff recipient; default = first non-standby member
    members:
      - agent: local            # reference to an agents: definition (processes: not allowed)
        instructions: >-        # optional: role instructions delivered via the inbox at launch
          Primary worker. When stuck, spawn_agent "opus" and ask it via the inbox.
      - agent: opus
        standby: true           # optional (default false): declared only, not launched at team start
        instructions: "Hard problems only."
      - agent: grok
        standby: true
    kickoff: "Read the pinned task list and get started."   # optional: sent to the lead
```

### How to launch

- **Toolbar**: when the loaded config has `team_presets:`, 👥 chips appear. Click ▶ to launch;
  a toast summarizes the result (started / existing / failed / standby counts).
- **Queen tool**: agents themselves can assemble a team with `spawn_team {preset: "daily"}`.
  Both paths run the same backend function and return the same JSON report.

### Launch semantics

- Non-standby members launch **sequentially in declaration order**. A member whose session is
  already alive is **skipped instead of duplicated**, so clicking 👥 repeatedly is safe (idempotent).
- Members beyond the 9-pane limit are not spawned and are reported as failed ("pane limit") —
  a partial launch; whatever did start keeps working.
- `instructions` and `kickoff` are delivered via the **durable Queen inbox** (mailbox =
  definition name, sender = `queen:preset/<preset name>`). Standby members receive their
  instructions too, so an agent launched later can read its role with `list_inbox`.
  Delivery only happens **when the call actually started at least one member**, so re-clicking
  👥 on a running team re-sends nothing.

### Validation errors

`team_presets:` is validated at config load. These fail the load: referencing a name not under
`agents:` (`processes:` entries are not allowed), an empty or all-standby member list, a standby
member as `lead`, and declaring the same agent twice in one preset.

### Intended pattern: local-LLM primary + cloud standby (cost tiering)

Reproduce "local LLM for everyday work, Claude Opus / Grok only for hard problems" in one click.
Keep the Claude Code CLI and point the local member at llama.cpp / ollama through
claude-code-router (routing is decided by **per-process env**, so `agents[].env` with
`ANTHROPIC_BASE_URL` is all it takes), and declare the cloud members `standby: true`.
Escalation is a **convention in the instructions**, not a mechanism. Do NOT write a
self-judged trigger like "when you feel stuck" — local models answer hard questions
confidently and never report being stuck, so it simply won't fire. Use **objective
conditions** instead:

> Example instruction for the primary: "Escalate whenever any of these holds: (1) tests or
> the build failed twice for the same cause, (2) the change touches public APIs, stored
> data, or security boundaries (then an opus review is required before you report done),
> (3) a human says 'ask opus'. Procedure: spawn_agent \"opus\", send an inbox message with
> a summary and what you tried, and await the reply."

See [example/team-preset/ptygrid.yml](../example/team-preset/ptygrid.yml) for the full sample.
Even though both panes run the same `claude` binary, ptygrid distinguishes them by definition
name, and one `-s user` Queen MCP registration covers every pane.

> [!WARNING]
> **settings.json interference**: besides the env vars ptygrid passes via `agents[].env`,
> Claude Code also reads the `env` block of `~/.claude/settings.json` (user) and
> `.claude/settings.json` (project) — and **on some versions the settings side wins over
> the process environment**. To make local routing stick reliably, give the local agent a
> per-agent settings file via `cmd: "claude --settings router.settings.json"` (CLI-argument
> scope outranks both project and user settings). Do NOT put the base URL in a project-level
> `.claude/settings.json`: it would also apply to the cloud panes running in the same working
> folder. See A-2b / R1 in verify-team-preset.md (Japanese) for the sample settings file and
> the "did it actually hit the router" check.

## Practical Recipes: Agent Coordination

### Delegating a task to another agent

Just ask, in a Claude Code pane:

> Send "review src/session.rs" to `#3`, and summarize the reply when it comes.

Claude Code carries this out using Queen's `send_message` → `read_output` (polling).

### Checking the recipient's state before sending (recommended)

An interactive TUI's composer may have **unsent text left in it** (for example, when an update-check dialog
consumes the Enter). Before `send_message`, check the state with `read_output`, and if unsent text remains,
**send just an Enter with `text: ""` + `submit: true`** to push it through.

### Judging when a reply is complete

`read_output` only returns "the current screen," so wait for long tasks by polling. A reliable way to
judge: consider it **complete once the output stops changing for two consecutive reads (10–15 seconds
apart)**. A TUI's spinner keeps updating the elapsed seconds, so once the output goes static, you can judge
that the response is complete.

Even for TUIs like Grok that frequently redraw the whole screen, `read_output` reflects cursor movement and
clears, so it does not simply concatenate past redraws. Still, while the TUI keeps updating what it
displays, the completion judgment can be delayed. If `sent to #<id>` appears, the send itself succeeded.
When waiting a long time, also check whether the deliverables have been updated. See
`docs/troubleshooting.md` for details.

### Operating from a script, externally

To hit Queen from outside a ptygrid pane (a normal terminal or CI), you can use the bundled
`scripts/queen-send.py`:

```bash
python3 scripts/queen-send.py '#3' "run the tests"    # send → wait for completion → show output
python3 scripts/queen-send.py '#3' --read --lines 50    # read only
python3 scripts/queen-send.py '#3' --enter              # send just an Enter
```

Since `#` starts a comment in the shell, wrap the arguments in single quotes.

## Stored Data and Safety

ptygrid saves its runtime management data to Tauri's app-data directory, and does not add management files
to the project repository.

| Data | Location (under app-data) | Contents |
|---|---|---|
| logical session state | `project-state/` | project, layout, pane order, definition names, worktree references |
| linked worktree | `worktrees/` | opt-in worktrees and branches that were created |
| Queen Pins / Notes / Inbox | `queen/queen.sqlite3` | shared data per canonical project directory |
| auth tokens | `auth-tokens.json` | Queen `/mcp` token and the hook Bearer token (versioned, Unix permissions 0600). Stays valid after a restart |

Terminal output, expanded environment variables, `QUEEN_URL`, and launch command bodies are not saved to
the session state. Queen binds to localhost (`127.0.0.1`) only, and `spawn_agent` permits only the
definition names in the loaded `ptygrid.yml`. Since there is no authentication, disable Queen with
`queen.enabled: false` in environments where untrusted local processes run.

## Getting Help

- The traps discovered through real dogfooding (registration scope, the sandbox's localhost restriction,
  how to read TUI output, composer double-input, and so on) are collected in
  [troubleshooting.md](troubleshooting.en.md).
- For the design background and architecture, see [design.md](design.en.md).
