// Phase 5.5.0: legacy-route Deprecation/Sunset/Link trio (spec-phase5-5.md
// §3.6, design pin "design-5.5.0"). The spec §3.6 example carries a
// day-of-week typo (verified against a real calendar: 2026-07-28 is a
// Tuesday, 2027-07-28 a Wednesday) — the values below are the pin's
// corrected, calendar-accurate ones.

use std::sync::{Mutex, OnceLock};

use axum::http::{HeaderMap, HeaderName, HeaderValue};
use chrono::NaiveDate;

pub const DEPRECATION_VALUE: &str = "Tue, 28 Jul 2026 00:00:00 GMT";
pub const SUNSET_VALUE: &str = "Wed, 28 Jul 2027 00:00:00 GMT";
pub const LINK_VALUE: &str = "<https://modelcontextprotocol.io/spec/2026-07-28>; rel=\"deprecation\"";

/// Attach the Deprecation/Sunset/Link trio to a legacy-route response (also
/// used, per `capabilities::classify`, on an RC response answering a
/// deprecated-capability method). `Deprecation`/`Sunset` are single-valued
/// (`insert`); `Link` is HTTP's multi-valued header, so it's `append`ed —
/// a response that already carries an unrelated `Link` value must not have
/// it clobbered.
pub fn attach(headers: &mut HeaderMap) {
    headers.insert(
        HeaderName::from_static("deprecation"),
        HeaderValue::from_static(DEPRECATION_VALUE),
    );
    headers.insert(
        HeaderName::from_static("sunset"),
        HeaderValue::from_static(SUNSET_VALUE),
    );
    headers.append(
        HeaderName::from_static("link"),
        HeaderValue::from_static(LINK_VALUE),
    );
}

fn last_logged_day() -> &'static Mutex<Option<NaiveDate>> {
    static LAST: OnceLock<Mutex<Option<NaiveDate>>> = OnceLock::new();
    LAST.get_or_init(|| Mutex::new(None))
}

/// Should a "deprecated route in use" warning log, given the last-logged day
/// and today? Pure — separated from `log_deprecated_route`'s process-wide
/// static so the dedupe decision itself is unit-testable.
fn should_log(last: &mut Option<NaiveDate>, today: NaiveDate) -> bool {
    if *last == Some(today) {
        return false;
    }
    *last = Some(today);
    true
}

/// Emit at most one warning per calendar day (process-wide), so a chatty
/// legacy client can't flood the log.
pub fn log_deprecated_route() {
    let mut last = last_logged_day()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if should_log(&mut last, chrono::Utc::now().date_naive()) {
        eprintln!(
            "queen_compat: deprecated MCP route in use (2025-06 legacy transport or a \
             deprecated capability method) — see CONTRACT.md §5.5.1"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attach_sets_all_three_header_values() {
        let mut h = HeaderMap::new();
        attach(&mut h);
        assert_eq!(h.get("deprecation").unwrap(), DEPRECATION_VALUE);
        assert_eq!(h.get("sunset").unwrap(), SUNSET_VALUE);
        assert_eq!(h.get("link").unwrap(), LINK_VALUE);
    }

    #[test]
    fn attach_appends_link_but_replaces_deprecation_and_sunset() {
        let mut h = HeaderMap::new();
        attach(&mut h);
        attach(&mut h);
        assert_eq!(h.get_all("deprecation").iter().count(), 1);
        assert_eq!(h.get_all("sunset").iter().count(), 1);
        assert_eq!(h.get_all("link").iter().count(), 2);
    }

    #[test]
    fn should_log_dedupes_per_day() {
        let mut last = None;
        let day = NaiveDate::from_ymd_opt(2026, 7, 23).unwrap();
        let next_day = NaiveDate::from_ymd_opt(2026, 7, 24).unwrap();
        assert!(should_log(&mut last, day), "first call on a new day logs");
        assert!(!should_log(&mut last, day), "same-day repeat is deduped");
        assert!(should_log(&mut last, next_day), "a new day logs again");
    }
}
