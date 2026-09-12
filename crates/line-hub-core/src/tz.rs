//! Asia/Taipei (UTC+08:00) timezone helpers.
//!
//! Most local-only date filters that flow into line-desktop-mcp are
//! interpreted by the bridge in Asia/Taipei regardless of the host
//! system timezone (v3.0.0 CHANGELOG: "日期篩選與投票計畫目前固定採
//! Asia/Taipei (UTC+08:00)"). The user-facing helpers in this module
//! make sure we don't accidentally pass UTC midnight when the user typed
//! a Taipei midnight — that would otherwise shift the result window by
//! eight hours and is a common cause of "why are my messages missing?".
//!
//! Pure functions only; no global state. `chrono` is already on the
//! dependency tree so we don't pull anything new.

use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, TimeZone, Utc};

/// Asia/Taipei as a fixed `+08:00` offset. We deliberately avoid the
/// full `chrono-tz` database — Taiwan does not observe DST so the
/// fixed offset is correct year-round.
pub const TAIPEI_OFFSET_HOURS: i32 = 8;

/// Convert a `YYYY-MM-DD` calendar date (interpreted in Asia/Taipei
/// midnight) to the matching UTC `DateTime<Utc>` boundary.
pub fn local_midnight_to_utc(date: &str) -> Result<DateTime<Utc>, String> {
    let nd = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|e| format!("invalid date '{date}' (expected YYYY-MM-DD): {e}"))?;
    let ndt: NaiveDateTime = nd
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| format!("date '{date}' cannot be represented as midnight"))?;
    // Compose the local date + Taipei offset into a fixed-offset
    // datetime, then convert to UTC. FixedOffset's `from_local_datetime`
    // returns a single unambiguous value.
    let fixed = chrono::FixedOffset::east_opt(TAIPEI_OFFSET_HOURS * 3600)
        .ok_or_else(|| "failed to construct Asia/Taipei fixed offset".to_string())?;
    let local = fixed
        .from_local_datetime(&ndt)
        .single()
        .ok_or_else(|| format!("date '{date}' is not a unique instant in Asia/Taipei"))?;
    Ok(local.with_timezone(&Utc))
}

/// Convert a UTC timestamp back to the Asia/Taipei `YYYY-MM-DD` calendar
/// date. Used when persisting "user's local day" boundaries alongside
/// canonical UTC timestamps.
pub fn utc_to_local_date(ts: &DateTime<Utc>) -> String {
    let fixed = chrono::FixedOffset::east_opt(TAIPEI_OFFSET_HOURS * 3600)
        .expect("fixed offset 8h is always valid");
    let local = ts.with_timezone(&fixed);
    format!("{:04}-{:02}-{:02}", local.year(), local.month(), local.day())
}

/// "Now" in Asia/Taipei, formatted as `YYYY-MM-DD`. Convenience helper
/// for default-arg seeding in MCP `date`/`dateFrom` parameters.
pub fn today_local() -> String {
    let now = Utc::now();
    utc_to_local_date(&now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_midnight_round_trip() {
        // 2026-09-12 00:00 Taipei = 2026-09-11 16:00 UTC
        let utc = local_midnight_to_utc("2026-09-12").expect("ok");
        assert_eq!(utc.format("%Y-%m-%dT%H:%M:%SZ").to_string(), "2026-09-11T16:00:00Z");
        assert_eq!(utc_to_local_date(&utc), "2026-09-12");
    }

    #[test]
    fn rejects_bad_date() {
        assert!(local_midnight_to_utc("2026-13-99").is_err());
        assert!(local_midnight_to_utc("hello").is_err());
    }

    #[test]
    fn today_is_today() {
        let n = today_local();
        // Should round-trip through the parsing helper.
        let utc = local_midnight_to_utc(&n).expect("ok");
        // The UTC instant must be within ±25h of "now" (Taipei can flip
        // its date up to 16h before/after the corresponding UTC moment).
        let diff = (Utc::now() - utc).num_hours().abs();
        assert!(diff <= 25, "today_local out of range, diff={diff}h");
    }
}
