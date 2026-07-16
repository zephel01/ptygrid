//! ptygrid teams pane-backend (Phase 4.x groundwork, not yet wired into the
//! app crate).
//!
//! Implements the pane-backend socket protocol modeled on the
//! CustomPaneBackend proposal (anthropics/claude-code#26572): a JSON-RPC 2.0
//! NDJSON server over a Unix domain socket, a `PaneHost` trait the ptygrid
//! session manager will implement, and the `ptygrid-tmux-shim` binary that
//! translates the tmux subcommand surface driven by Claude Code's
//! split-pane teammate mode into those RPCs.
//!
//! If the proposal is adopted upstream, the same socket can be advertised
//! directly via `CLAUDE_PANE_BACKEND_SOCKET` and the shim becomes optional.

pub mod host;
pub mod mock;
pub mod protocol;
#[cfg(unix)]
pub mod server;
pub mod shim;
#[cfg(unix)]
pub mod shim_client;
