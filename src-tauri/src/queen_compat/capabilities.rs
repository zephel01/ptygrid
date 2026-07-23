// Phase 5.5.0: legacy-capability no-op vs `-32601 method_not_found` policy for
// the deprecated `sampling/*`, `resources/roots`, `logging/setLevel` methods
// (spec-phase5-5.md §3.1, design pin "design-5.5.0"). Pure function — no I/O.

use super::config::LegacyCapabilities;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    /// Not a legacy-capability method — forward to rmcp unchanged.
    Passthrough,
    /// A legacy-capability method the config keeps answering with a 200 no-op.
    NoOp200,
    /// A legacy-capability method the config has turned off — real
    /// `-32601 method_not_found`.
    MethodNotFound,
}

/// Classify `body.method` against the resolved `legacy_capabilities` flags.
/// `sampling/*` (any method under that prefix), `resources/roots`, and
/// `logging/setLevel` are the only methods this ever touches — anything else
/// is `Passthrough`.
pub fn classify(method: &str, caps: LegacyCapabilities) -> Classification {
    let enabled = if method.starts_with("sampling/") {
        caps.sampling
    } else if method == "resources/roots" {
        caps.roots
    } else if method == "logging/setLevel" {
        caps.logging
    } else {
        return Classification::Passthrough;
    };
    if enabled {
        Classification::NoOp200
    } else {
        Classification::MethodNotFound
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(sampling: bool, roots: bool, logging: bool) -> LegacyCapabilities {
        LegacyCapabilities {
            sampling,
            roots,
            logging,
        }
    }

    #[test]
    fn passthrough_for_unrelated_methods() {
        assert_eq!(
            classify("tools/call", caps(true, true, true)),
            Classification::Passthrough
        );
        assert_eq!(
            classify("ping", caps(false, false, false)),
            Classification::Passthrough
        );
    }

    #[test]
    fn sampling_prefix_matches_any_suffix() {
        assert_eq!(
            classify("sampling/createMessage", caps(true, false, false)),
            Classification::NoOp200
        );
        assert_eq!(
            classify("sampling/createMessage", caps(false, false, false)),
            Classification::MethodNotFound
        );
    }

    #[test]
    fn resources_roots_is_an_exact_match() {
        assert_eq!(
            classify("resources/roots", caps(false, true, false)),
            Classification::NoOp200
        );
        assert_eq!(
            classify("resources/roots", caps(false, false, false)),
            Classification::MethodNotFound
        );
        // A sibling under the same namespace is NOT roots and passes through.
        assert_eq!(
            classify("resources/list", caps(false, true, false)),
            Classification::Passthrough
        );
    }

    #[test]
    fn logging_set_level_is_an_exact_match() {
        assert_eq!(
            classify("logging/setLevel", caps(false, false, true)),
            Classification::NoOp200
        );
        assert_eq!(
            classify("logging/setLevel", caps(false, false, false)),
            Classification::MethodNotFound
        );
    }

    #[test]
    fn defaults_from_legacy_capabilities_default() {
        // sampling/roots off, logging on (LegacyCapabilities::default()).
        let d = LegacyCapabilities::default();
        assert_eq!(
            classify("sampling/createMessage", d),
            Classification::MethodNotFound
        );
        assert_eq!(
            classify("resources/roots", d),
            Classification::MethodNotFound
        );
        assert_eq!(classify("logging/setLevel", d), Classification::NoOp200);
    }
}
