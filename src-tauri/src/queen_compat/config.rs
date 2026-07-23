// Phase 5.5.0: hot-swappable feature flags for the MCP RC-compat router.
// This is the *resolved* shape (plain bool/usize, defaults already applied),
// distinct from `crate::config::McpConfig` (the raw `Option<T>` ptygrid.yml
// shape) — mirrors the `QueenConfig` -> `effective_*()` split already used
// for the `queen:` block.

// TODO(track-b 5.5.0): remove this allow when the compat middleware is wired
// into queen.rs (McpCompatHandle gets constructed there); until then every
// item here is staged-but-unreferenced by design.
#![allow(dead_code)]

use std::sync::Arc;

use arc_swap::ArcSwap;

/// Resolved `mcp:` block. Read per-request by the compat middleware via
/// [`McpCompatHandle::get`] — a lock-free read, so a config edit never adds
/// latency to `/mcp` traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpCompatConfig {
    pub rc_2026_07_28: bool,
    pub legacy_2025_06: bool,
    pub max_body_bytes: usize,
    pub legacy_capabilities: LegacyCapabilities,
}

/// Resolved `mcp.legacy_capabilities:` — per-capability no-op vs
/// `-32601 method_not_found` policy for the deprecated `sampling/*`,
/// `resources/roots`, `logging/setLevel` methods (spec-phase5-5.md §3.1,
/// design pin "design-5.5.0"). See `capabilities::classify`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyCapabilities {
    pub sampling: bool,
    pub roots: bool,
    pub logging: bool,
}

impl Default for LegacyCapabilities {
    fn default() -> Self {
        LegacyCapabilities {
            sampling: false,
            roots: false,
            logging: true,
        }
    }
}

impl Default for McpCompatConfig {
    fn default() -> Self {
        McpCompatConfig {
            rc_2026_07_28: true,
            legacy_2025_06: true,
            max_body_bytes: 1_048_576,
            legacy_capabilities: LegacyCapabilities::default(),
        }
    }
}

impl From<&crate::config::McpConfig> for McpCompatConfig {
    fn from(raw: &crate::config::McpConfig) -> Self {
        McpCompatConfig {
            rc_2026_07_28: raw.effective_rc_2026_07_28(),
            legacy_2025_06: raw.effective_legacy_2025_06(),
            max_body_bytes: raw.effective_max_body_bytes(),
            legacy_capabilities: LegacyCapabilities {
                sampling: raw.effective_legacy_capabilities_sampling(),
                roots: raw.effective_legacy_capabilities_roots(),
                logging: raw.effective_legacy_capabilities_logging(),
            },
        }
    }
}

/// Live, shareable, hot-swappable [`McpCompatConfig`]. Cloning shares the same
/// underlying `Arc<ArcSwap<_>>`, so a [`McpCompatHandle::set`] made through one
/// clone (a config reload) is immediately observed by every other clone —
/// including the copy captured by the already-bound `/mcp` compat middleware.
/// Parallels [`crate::token_store::TokenHandle`]'s role for the auth token,
/// but `ArcSwap` instead of `Mutex`: this is read on every `/mcp` request.
#[derive(Clone)]
pub struct McpCompatHandle(Arc<ArcSwap<McpCompatConfig>>);

impl McpCompatHandle {
    pub fn new(initial: McpCompatConfig) -> Self {
        McpCompatHandle(Arc::new(ArcSwap::from_pointee(initial)))
    }

    /// Current value (lock-free read).
    pub fn get(&self) -> McpCompatConfig {
        *self.0.load_full()
    }

    /// Replace the value in place (config reload). Observed by all clones.
    pub fn set(&self, value: McpCompatConfig) {
        self.0.store(Arc::new(value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_design_pin() {
        let cfg = McpCompatConfig::default();
        assert!(cfg.rc_2026_07_28);
        assert!(cfg.legacy_2025_06);
        assert_eq!(cfg.max_body_bytes, 1_048_576);
    }

    #[test]
    fn handle_shares_updates_across_clones() {
        let a = McpCompatHandle::new(McpCompatConfig::default());
        let b = a.clone();
        assert!(b.get().rc_2026_07_28);
        a.set(McpCompatConfig {
            rc_2026_07_28: false,
            ..McpCompatConfig::default()
        });
        assert!(!b.get().rc_2026_07_28, "clone observes the hot-swap");
    }
}
