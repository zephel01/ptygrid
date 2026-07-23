**English** · [日本語 (Japanese)](design.md)

# ptygrid Design and Architecture

Last updated: 2026-07-16 / Implementation baseline: Phase 3.9

This document summarizes the current implementation structure and the design decisions to uphold when making changes. For the initial research and competitive positioning, see [competitive-landscape.en.md](competitive-landscape.en.md); for the exact IPC types and constraints, see [CONTRACT.md](../CONTRACT.md); and for how to operate the app, see [userguide.en.md](userguide.en.md).

## 1. Goals and Boundaries

ptygrid is a macOS desktop application that runs multiple AI CLIs and development processes in parallel across up to nine PTY panes, letting them read, write, and coordinate with one another through the built-in MCP server, Queen. The same runtime path is also implemented for Linux, but as of Phase 3.9 that support is experimental (beta).

The design rests on four central principles:

- Treat any interactive CLI as an ordinary PTY, without depending on a specific vendor's protocol.
- Draw clear per-project boundaries around sessions, Git, and persistent data.
- Never resolve duplicate agent names or concurrent updates by guessing; reject misdirected sends and lost updates instead.
- Keep the structure such that each Phase 3 feature can be released independently.

A full IDE, remote execution, reconnecting to PTY processes after an OS restart, and automatic deletion of dirty worktrees are all out of scope for now.

## 2. Current Structure

```text
┌─ Frontend: Svelte 5 / TypeScript / Tauri WebView ──────────────┐
│ App.svelte                                                     │
│ ├ Terminal.svelte × up to 9 (xterm.js + fit addon)             │
│ ├ GitPanel.svelte (status/diff/stage/unstage/commit)            │
│ └ stores.svelte.ts (session/layout/resource/state)              │
└────────────────── Tauri commands / batched events ─────────────┘
                              │
┌─ Backend: Rust ─────────────┴───────────────────────────────────┐
│ commands.rs          IPC boundary                               │
│ session.rs / pty.rs  PTY lifecycle, output ring, name resolution│
│ config.rs            ptygrid.yml parse/watch                      │
│ git_service.rs       installed git invocation                   │
│ worktree.rs          opt-in linked worktree creation/reuse      │
│ project_state.rs     versioned logical session persistence      │
│ resource_monitor.rs  shared process-tree CPU/RSS sampler        │
│ queen.rs             rmcp Streamable HTTP server / 18 tools     │
│ queen_store.rs       SQLite pins/notes/inbox with transactions  │
└─────────────────────────────────────────────────────────────────┘
```

`lib.rs` is kept to nothing more than initializing services and registering the Tauri commands. The wire format lives in `commands.rs`, while the process and storage implementations are split into separate modules, so that Git and SQLite work never gets mixed into the session hot path.

## 3. Technology Stack

| Layer | Choice | Rationale |
|---|---|---|
| desktop | Tauri v2 | Rust backend with a lightweight system WebView |
| frontend | Svelte 5 + TypeScript + Vite | Small reactive state and rendering of multiple terminals |
| terminal | `@xterm/xterm` + `portable-pty` | ANSI terminal UI and native PTY processes |
| async / server | Tokio + Axum + rmcp | Queen's Streamable HTTP transport |
| config | serde_norway + notify | YAML parsing and change events for explicit reloads |
| Git | installed `git` executable | Preserves native hooks, signing, config, and worktree semantics |
| process monitor | sysinfo | A single sampler aggregates every session's process tree |
| Queen storage | rusqlite + bundled SQLite | Transactions, schema versioning, per-project isolation |
| GUI PATH recovery | fix-path-env-rs | Resolves user-installed CLIs even for desktop launches on Linux/macOS |

We do not use `git2` for Git. Rather than passing command strings to a shell, we launch the installed `git` with structured arguments. This makes the user's Git hooks and signing configuration behave exactly as they do with ordinary Git.

### Naming / packaging compatibility

The display name, window title, Rust crate/binary, and npm package have all been unified under `ptygrid`. The bundle icon is the terminal prompt + grid motif in `src-tauri/icons/`. The bundle identifier `com.zephel01.multiterminal`, on the other hand, is deliberately kept so that existing users retain their Tauri app-data. The config filename has already been changed to `ptygrid.yml`. As a migration measure, the old `mterm.yml` is still read as well (when both exist, `ptygrid.yml` takes precedence), and the watcher tracks whichever file was actually loaded. Do not rename the bundle identifier without a migration design.

## 4. Session / PTY Model

A single session holds a backend-assigned `u32` ID, a PTY master/slave pair, a child process, a writer, a 256 KiB output ring, its launch spec, and a generation.

- `spawn_shell` / `spawn_agent` create the PTY and emit `pty-output` from a blocking reader thread.
- Frontend input goes through `write_pty`, and resizes go through `resize_pty`.
- restart/autorestart keep the same session ID, and the generation invalidates events from stale readers.
- A manual kill does not trigger autorestart.
- `on-failure` / `always` restart after one second and stop after five consecutive failures.

After the app exits, it does not reconnect to PTYs. A logical resume resolves the saved definition reference against the current `ptygrid.yml` (legacy: `mterm.yml`) and launches a new PTY process. If a `resume` command is present it is used to resume; otherwise the normal `cmd` is used.

## 5. Session Addressing

The pane header shows defined sessions as `codex #3` and adhoc sessions as `shell #3`. A CLI started manually inside a shell is detected separately as a foreground process.

Queen resolves an `agent` in the following order:

1. Exact specification via `#<id>`.
2. Exact match against a unique definition/session name.
3. Exact match against a unique foreground process name.

If there is more than one candidate, it returns an ambiguous error listing the candidates, such as `#3, #5`. It does not guess the latest ID or anything similar and send to it. Session IDs are valid only for the current run of the app; after an app restart, obtain them again from `list_agents` or the pane header.

## 6. Queen

Queen is an rmcp Streamable HTTP server that binds only to `127.0.0.1`. The default port is 39237, and if it is in use, it tries the next ports in order up to 39246. It passes the actual URL to every session as `QUEEN_URL`.

The 18 tools as of Phase 3.8:

- live session: `list_agents`, `read_output`, `send_message`, `spawn_agent`, `notify`
- durable pins: `set_pin`, `list_pins`, `delete_pin`
- durable notes: `create_note`, `list_notes`, `get_note`, `update_note`, `delete_note`
- durable inbox: `send_inbox`, `list_inbox`, `ack_inbox`, `reply_inbox`
- wait: `await`

`spawn_agent` only permits names defined in the current `ptygrid.yml`. Pins/Notes/Inbox are scoped to the loaded canonical config directory and are unavailable when no project is loaded.

The Inbox is an append-only channel separate from `send_message`, which writes to a live PTY. It uses stable mailbox names rather than session `#id`s, which change across app restarts. A reply swaps the original sender and recipient and inherits the root ID to form a thread. Creating a reply happens in the same transaction as acknowledging the original message.

`await` subscribes to the Inbox generation via `tokio::sync::watch` before its first query, so it never misses a message that arrives between the query and the start of the sleep. While waiting, it does not hold the DB mutex; when a message is committed, the notification triggers a short query to run again. It handles the rmcp request cancellation token, a maximum five-minute deadline, and the immediate return of existing messages all within the same select loop.

## 7. Persistence and Concurrent Updates

Runtime-managed data lives under the Tauri app-data directory, so no management files are created in the user's repository.

```text
app-data/
├ project-state/           versioned JSON / last project pointer
├ worktrees/               opt-in linked worktrees
└ queen/queen.sqlite3      project-scoped pins, notes and inbox
```

The Queen Store serializes in-process access through a single `Mutex<Connection>` and performs mutations inside `BEGIN IMMEDIATE` transactions. Each pin/note carries a monotonically increasing `revision`, and updates and deletes succeed only when `expectedRevision` matches. If multiple agents write from the same revision at once, only the first commits and the rest roll back as conflicts.

The DB uses WAL, a busy timeout of 5 seconds, and `PRAGMA user_version = 2`. The v1-to-v2 migration is transactional, and an unknown newer schema version is not opened silently.

## 8. Git / Worktree Safety

The Git panel provides status, working/staged diffs, staging/unstaging of explicit paths, and commits that target only the current index. It performs no implicit staging on file selection or commit. Diffs are subject to size and file-count limits, and paths are restricted to within the repository.

Worktree isolation is opt-in per agent definition. ptygrid creates and locks a linked worktree and a `ptygrid/<agent>/...` branch under the app-data directory. On restart it reuses the same worktree and runs setup only the first time. Even on failure it retains the worktree and does not automatically delete dirty content.

## 9. Resource Monitoring

A single sysinfo sampler is shared across all sessions and refreshes process information once per second. Taking each PTY child as the root, it sums the CPU, resident memory, and process count of all descendants and sends them to the frontend as one `session-resources` event.

CPU treats one core as 100%, so multi-core workloads exceed 100%. Memory is the sum of each process's RSS, with no deduplication of shared pages. The sum of the pane values is shown in the toolbar; no additional sampling is done to compute the total.

## 10. Release Status

Phases 0 through 2.1 and Phases 3.0 through 3.9 are implemented. Phase 3.9 added, as Linux test support, native builds, Ubuntu CI, `.deb` / AppImage packaging, and PATH recovery on GUI launch. For the detailed gates, see [inside/phase3.md](inside/phase3.md).

## 11. Principles for Making Changes

- Document IPC or MCP schema changes in [CONTRACT.md](../CONTRACT.md) first.
- Do not implicitly create project data in the repository.
- Do not perform destructive Git/worktree operations on a guess.
- Do not automatically select or overwrite same-named sessions or stale revisions.
- The backend must pass `cargo test` / `cargo check`, and the frontend must pass `svelte-check` / a production build.
- Keep both macOS and Linux beta CI green, and keep platform-specific process retrieval able to fall back.
