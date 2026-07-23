// Phase 5.5.0: MCP 2026-07-28 RC-compat router (WIP — agent mid-implementation).
//
// This mod.rs was scaffolded by the human integrator to unblock the build
// after the implementing agent stopped between `mod queen_compat;` (lib.rs)
// and creating this file. Only `config` is compiled for now — it is
// self-contained (ArcSwap handle + From<&crate::config::McpConfig>).
//
// TODO(sonnet-coder / track-b 5.5.0): when wiring the middleware, uncomment
// the submodules below, connect them in queen.rs, and delete this note.
//
// pub mod header;
// pub mod route;

pub mod config;

// Re-export the handle so future call sites read naturally
// (`queen_compat::McpCompatHandle`).
#[allow(unused_imports)]
pub use config::{McpCompatConfig, McpCompatHandle};
