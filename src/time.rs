use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration, Utc};
use regex::Regex;
use std::sync::LazyLock;

/// Represents a time range for log queries
pub struct TimeRange {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

impl TimeRange {
    /// Create a time range from explicit start/end strings
    pub fn from_explicit(start: &str, end: Option<&str>) -> Result<Self> {
        let start_dt = parse_datetime(start)?;
        let end_dt = match end {
            Some(e) => parse_datetime(e)?,
            None => Utc::now(),
        };

        Ok(Self {
            start: start_dt,
            end: end_dt,
        })
    }

    /// Create a time range relative to now (e.g., "1h", "30m", "2d")
    pub fn from_relative(duration_str: &str) -> Result<Self> {
        let duration = parse_duration(duration_str)?;
        let end = Utc::now();
        let start = end - duration;

        Ok(Self { start, end })
    }
}

/// Parse a datetime string into UTC DateTime
fn parse_datetime(input: &str) -> Result<DateTime<Utc>> {
    // Try RFC3339 first
    if let Ok(dt) = DateTime::parse_from_rfc3339(input) {
        return Ok(dt.with_timezone(&Utc));
    }

    // Try common formats
    let formats = [
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%d",
    ];

    for fmt in formats {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(input, fmt) {
            return Ok(naive.and_utc());
        }
        // Try date-only format
        if let Ok(naive_date) = chrono::NaiveDate::parse_from_str(input, fmt) {
            return Ok(naive_date.and_hms_opt(0, 0, 0).unwrap().and_utc());
        }
    }

    Err(anyhow!(
        "Unable to parse datetime '{}'. Expected formats: RFC3339, YYYY-MM-DD HH:MM:SS, YYYY-MM-DD",
        input
    ))
}

/// Regex for matching duration components like "1.5h", "30m", "2days"
static DURATION_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(\d+(?:\.\d+)?)\s*(s(?:ec(?:ond)?s?)?|m(?:in(?:ute)?s?)?|h(?:(?:ou)?rs?)?|d(?:ays?)?|w(?:eeks?)?)")
        .unwrap()
});

/// Parse a relative duration string into a chrono Duration
///
/// Supports flexible formats:
///   - Simple: "30s", "15m", "2h", "1d", "1w"
///   - Combined: "1h30m", "2d12h", "1w2d"
///   - Decimals: "1.5h", "0.5d"
///   - Verbose: "2hours", "30mins", "1week"
fn parse_duration(input: &str) -> Result<Duration> {
    let input = input.trim().to_lowercase();

    if input.is_empty() {
        return Err(anyhow!("Empty duration string"));
    }

    let mut total_seconds: f64 = 0.0;
    let mut matched_anything = false;

    for cap in DURATION_REGEX.captures_iter(&input) {
        matched_anything = true;

        let value: f64 = cap[1].parse().map_err(|_| anyhow!("Invalid number in duration"))?;
        let unit = &cap[2].to_lowercase();

        let multiplier = match unit.chars().next() {
            Some('s') => 1.0,                    // seconds
            Some('m') => 60.0,                   // minutes
            Some('h') => 3600.0,                 // hours
            Some('d') => 86400.0,                // days
            Some('w') => 604800.0,               // weeks
            _ => return Err(anyhow!("Unknown time unit: {}", unit)),
        };

        total_seconds += value * multiplier;
    }

    if !matched_anything {
        return Err(anyhow!(
            "Invalid duration format '{}'. Examples: 1h, 30m, 2d, 1h30m, 1.5h",
            input
        ));
    }

    Ok(Duration::seconds(total_seconds as i64))
}

/// Convert a duration string to Docker's --since format
/// Docker accepts: "1h30m", "2h", "30m", etc.
pub fn to_docker_since(duration_str: &str) -> Result<String> {
    // Docker's --since format is very similar to our input format
    // It accepts combinations like "1h30m" directly
    // We just need to validate it parses correctly
    let _duration = parse_duration(duration_str)?;

    // Return the original string as Docker accepts similar formats
    // Normalize to lowercase for consistency
    Ok(duration_str.trim().to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_durations() {
        assert_eq!(parse_duration("30s").unwrap(), Duration::seconds(30));
        assert_eq!(parse_duration("15m").unwrap(), Duration::minutes(15));
        assert_eq!(parse_duration("2h").unwrap(), Duration::hours(2));
        assert_eq!(parse_duration("1d").unwrap(), Duration::days(1));
        assert_eq!(parse_duration("1w").unwrap(), Duration::weeks(1));
    }

    #[test]
    fn test_combined_durations() {
        assert_eq!(parse_duration("1h30m").unwrap(), Duration::minutes(90));
        assert_eq!(parse_duration("2d12h").unwrap(), Duration::hours(60));
        assert_eq!(parse_duration("1w2d").unwrap(), Duration::days(9));
    }

    #[test]
    fn test_decimal_durations() {
        assert_eq!(parse_duration("1.5h").unwrap(), Duration::minutes(90));
        assert_eq!(parse_duration("0.5d").unwrap(), Duration::hours(12));
    }

    #[test]
    fn test_verbose_formats() {
        assert_eq!(parse_duration("2hours").unwrap(), Duration::hours(2));
        assert_eq!(parse_duration("30mins").unwrap(), Duration::minutes(30));
        assert_eq!(parse_duration("1week").unwrap(), Duration::weeks(1));
        assert_eq!(parse_duration("3days").unwrap(), Duration::days(3));
    }

    #[test]
    fn test_case_insensitive() {
        assert_eq!(parse_duration("2H").unwrap(), Duration::hours(2));
        assert_eq!(parse_duration("30M").unwrap(), Duration::minutes(30));
        assert_eq!(parse_duration("1D").unwrap(), Duration::days(1));
    }

    #[test]
    fn test_invalid_input() {
        assert!(parse_duration("").is_err());
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("5x").is_err());
    }
}
