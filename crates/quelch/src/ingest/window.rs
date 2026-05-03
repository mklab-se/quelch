//! Minute-window planning for incremental sync.
//!
//! See `docs/sync.md` — "Per-cycle steps" for the full algorithm.

use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc};

/// A closed half-open time window `[start, end)` at minute resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    /// Inclusive start of the window (floored to the minute boundary).
    pub start: DateTime<Utc>,
    /// Exclusive end / target of the window (floored to the minute boundary).
    pub end: DateTime<Utc>,
}

/// Round `dt` down to the nearest whole minute (zeroing sub-minute components).
///
/// The result always has `second == 0` and `nanosecond == 0`.
pub fn floor_to_minute(dt: DateTime<Utc>) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(
        dt.year(),
        dt.month(),
        dt.day(),
        dt.hour(),
        dt.minute(),
        0,
    )
    .single()
    .expect("valid timestamp — flooring to minute should never produce an ambiguous or invalid time")
}

/// Compute the next incremental window to process.
///
/// Returns `Some(Window)` when there is progress to make, or `None` when:
/// - `last_complete_minute` is `None` (backfill mode — caller handles separately), **or**
/// - the safety-lagged target is already at or behind `last_complete_minute`.
///
/// # Arguments
///
/// * `last_complete_minute` — the most-recent minute whose window has been fully ingested,
///   as tracked in `cosmos::meta::Cursor::last_complete_minute`.  `None` means no window
///   has ever been processed (initial-backfill mode).
/// * `now` — current wall-clock time; will be floored to the minute before applying the lag.
/// * `safety_lag_minutes` — how many minutes to lag behind `now` to avoid racing the source's
///   indexing pipeline (typically 2–5 minutes; see `docs/sync.md`).
pub fn plan_next_window(
    last_complete_minute: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    safety_lag_minutes: u32,
) -> Option<Window> {
    let start = last_complete_minute?;
    let target = floor_to_minute(now) - Duration::minutes(i64::from(safety_lag_minutes));
    if target <= start {
        return None;
    }
    Some(Window { start, end: target })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 6, 1, h, m, 0)
            .single()
            .unwrap()
    }

    fn ts_secs(h: u32, m: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 6, 1, h, m, s)
            .single()
            .unwrap()
    }

    // --- floor_to_minute ---

    #[test]
    fn floor_strips_seconds_and_nanos() {
        let dt = ts_secs(10, 30, 45).with_nanosecond(123_456_789).unwrap();
        let floored = floor_to_minute(dt);
        assert_eq!(floored, ts(10, 30));
        assert_eq!(floored.second(), 0);
        assert_eq!(floored.nanosecond(), 0);
    }

    #[test]
    fn floor_already_on_boundary_is_idempotent() {
        let dt = ts(10, 0);
        assert_eq!(floor_to_minute(dt), dt);
    }

    // --- plan_next_window ---

    #[test]
    fn none_when_last_is_none_backfill_mode() {
        // last_complete_minute = None means we're in backfill mode; caller handles it.
        let result = plan_next_window(None, ts(10, 5), 2);
        assert!(result.is_none());
    }

    #[test]
    fn none_when_target_equals_last() {
        // now = 10:07, lag = 2 → target = 10:05; last = 10:05 → no progress
        let result = plan_next_window(Some(ts(10, 5)), ts(10, 7), 2);
        assert!(result.is_none());
    }

    #[test]
    fn none_when_target_behind_last() {
        // now = 10:06, lag = 2 → target = 10:04; last = 10:05 → already ahead
        let result = plan_next_window(Some(ts(10, 5)), ts(10, 6), 2);
        assert!(result.is_none());
    }

    #[test]
    fn some_when_normal_advance() {
        // now = 10:10, lag = 2 → target = 10:08; last = 10:05
        // Expected window: [10:05, 10:08)
        let result = plan_next_window(Some(ts(10, 5)), ts(10, 10), 2);
        let w = result.expect("should produce a window");
        assert_eq!(w.start, ts(10, 5));
        assert_eq!(w.end, ts(10, 8));
    }

    #[test]
    fn now_with_sub_minute_noise_is_floored() {
        // now = 10:10:45 (with seconds), lag = 2 → floor(now) = 10:10 → target = 10:08
        let now_noisy = ts_secs(10, 10, 45);
        let result = plan_next_window(Some(ts(10, 5)), now_noisy, 2);
        let w = result.expect("should produce a window");
        assert_eq!(w.end, ts(10, 8));
    }

    #[test]
    fn exactly_one_minute_of_progress() {
        // now = 10:08, lag = 2 → target = 10:06; last = 10:05 → one-minute window
        let result = plan_next_window(Some(ts(10, 5)), ts(10, 8), 2);
        let w = result.expect("should produce a window");
        assert_eq!(w.start, ts(10, 5));
        assert_eq!(w.end, ts(10, 6));
    }
}
