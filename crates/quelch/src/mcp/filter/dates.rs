//! Relative-date parsing for filter values.
//!
//! Converts human-readable relative date strings (e.g. `"6 months ago"`) and
//! ISO 8601 timestamps into [`chrono::DateTime<Utc>`] values.
//!
//! ## Simplification notice
//!
//! Months and years are approximated as fixed day counts (1 month = 30 days,
//! 1 year = 365 days).  This intentional simplification keeps the implementation
//! dependency-free while covering the 95% case.  Agents that need exact calendar
//! boundaries (e.g. "exactly one calendar month ago") should pass ISO 8601
//! timestamps directly instead of relative strings.

use chrono::{DateTime, Duration, Utc};

/// Parse a relative or absolute date string into a UTC datetime.
///
/// Supported relative formats: `"N unit ago"` where N is a positive integer and
/// unit is one of:
///
/// - `second` / `seconds`
/// - `minute` / `minutes`
/// - `hour` / `hours`
/// - `day` / `days`
/// - `week` / `weeks`
/// - `month` / `months` (approximated as 30 days)
/// - `year` / `years` (approximated as 365 days)
///
/// Absolute formats: any ISO 8601 string accepted by [`DateTime::parse_from_rfc3339`].
///
/// Returns `None` if the input cannot be parsed.
pub fn parse_relative(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();

    // Try absolute ISO 8601 first.
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }

    // Try relative format: "N unit ago"
    parse_relative_duration(s)
}

fn parse_relative_duration(s: &str) -> Option<DateTime<Utc>> {
    // Expected pattern: "<number> <unit> ago"
    let s = s.strip_suffix("ago")?.trim_end();
    let (num_str, unit_str) = s.split_once(char::is_whitespace)?;
    let n: i64 = num_str.trim().parse().ok()?;
    let unit = unit_str.trim();

    let duration = match unit {
        "second" | "seconds" => Duration::seconds(n),
        "minute" | "minutes" => Duration::minutes(n),
        "hour" | "hours" => Duration::hours(n),
        "day" | "days" => Duration::days(n),
        "week" | "weeks" => Duration::weeks(n),
        // 1 month ≈ 30 days (intentional simplification).
        "month" | "months" => Duration::days(n * 30),
        // 1 year ≈ 365 days (intentional simplification).
        "year" | "years" => Duration::days(n * 365),
        _ => return None,
    };

    Some(Utc::now() - duration)
}

/// Format a [`DateTime<Utc>`] as an ISO 8601 / RFC 3339 string suitable for
/// use in Cosmos SQL parameters and OData filter strings.
pub fn to_iso(dt: DateTime<Utc>) -> String {
    dt.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn approx_eq(a: DateTime<Utc>, b: DateTime<Utc>, tolerance_secs: i64) -> bool {
        let diff = (a - b).num_seconds().abs();
        diff <= tolerance_secs
    }

    #[test]
    fn parses_6_months_ago() {
        let result = parse_relative("6 months ago").expect("should parse");
        let expected = Utc::now() - Duration::days(180);
        assert!(
            approx_eq(result, expected, 2),
            "result={result} expected≈{expected}"
        );
    }

    #[test]
    fn parses_1_hour_ago() {
        let result = parse_relative("1 hour ago").expect("should parse");
        let expected = Utc::now() - Duration::hours(1);
        assert!(approx_eq(result, expected, 2));
    }

    #[test]
    fn parses_1_year_ago() {
        let result = parse_relative("1 year ago").expect("should parse");
        let expected = Utc::now() - Duration::days(365);
        assert!(approx_eq(result, expected, 2));
    }

    #[test]
    fn parses_30_days_ago() {
        let result = parse_relative("30 days ago").expect("should parse");
        let expected = Utc::now() - Duration::days(30);
        assert!(approx_eq(result, expected, 2));
    }

    #[test]
    fn parses_2_weeks_ago() {
        let result = parse_relative("2 weeks ago").expect("should parse");
        let expected = Utc::now() - Duration::weeks(2);
        assert!(approx_eq(result, expected, 2));
    }

    #[test]
    fn parses_iso_absolute() {
        let result = parse_relative("2026-01-01T00:00:00Z").expect("should parse");
        let expected: DateTime<Utc> = "2026-01-01T00:00:00Z".parse().unwrap();
        assert_eq!(result, expected);
    }

    #[test]
    fn returns_none_for_garbage() {
        assert!(parse_relative("not a date").is_none());
        assert!(parse_relative("").is_none());
        assert!(parse_relative("yesterday").is_none());
    }

    #[test]
    fn to_iso_formats_correctly() {
        let dt: DateTime<Utc> = "2026-01-15T09:30:00Z".parse().unwrap();
        assert_eq!(to_iso(dt), "2026-01-15T09:30:00Z");
    }
}
