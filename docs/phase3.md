# Phase 3 staged delivery plan

Phase 3 is delivered as a sequence of independently releasable changes. Each
release must preserve the Phase 0–2.1 IPC contract unless its own contract
section explicitly extends it, and must pass both Rust and frontend checks.

## Release sequence

| Release | Scope | Completion gate |
|---|---|---|
| 3.0 ✅ | IPC boundary extraction and deterministic process-resolution tests | No wire-format or UI behavior changes; all existing checks pass |
| 3.1 ✅ | Read-only Git status and diff | Works for normal repos and linked worktrees; never changes index/worktree |
| 3.2 ✅ | Untracked diff plus explicit stage, unstage, and inline commit | Hooks/errors are surfaced; no implicit staging |
| 3.3 ✅ | Optional per-agent worktree isolation | Safe naming/locking; dirty worktrees are never silently removed |
| 3.4 ✅ | Versioned project state and logical session resume | No expanded environment values or secrets are persisted |
| 3.5 ✅ | Per-session process-tree CPU/memory monitoring | One shared sampler; batched frontend updates |
| 3.6 | Durable Queen pins and notes | Project-scoped transactional storage and CRUD tools |
| 3.7 | Durable Queen inbox and reply | Stable message IDs, acknowledgement, and reply correlation |
| 3.8 | Cancellable Queen `await` | Cursor-based wait, bounded timeout, MCP cancellation support |

## Fixed design decisions

- Git integration uses the installed `git` executable with structured
  arguments, not a shell command string. This preserves native Git hooks,
  signing, configuration, and worktree behavior.
- Worktree isolation is opt-in. The default remains the shared-workspace
  collaboration model.
- Phase 3 resume is a logical resume: restore project/layout/session metadata
  and relaunch through an optional config-defined resume command. Reattaching
  to a PTY after the owning app process exits is out of scope.
- Runtime state and Queen data live under the Tauri app-data directory. They
  do not create tracked or untracked files in the user's repository.
- Resource usage is aggregated across the PTY child process tree, not just
  the shell process.
- Queen inbox data is separate from `send_message`: the former is durable and
  cooperative, while the latter writes directly to a live PTY.

## Release discipline

For every release:

1. Add the precise backend/frontend contract to `CONTRACT.md`.
2. Keep new service logic outside `lib.rs` and the session hot path.
3. Add pure unit tests for parsing/state transitions and focused integration
   tests for external process behavior.
4. Run `cargo test`, `cargo check`, `npm run check`, and `npm run build`.
5. Update the user guide only for behavior included in that release.
