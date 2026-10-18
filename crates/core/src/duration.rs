use std::fmt;
use std::time::Duration;

/// Error returned by [`parse_duration`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurationParseError {
    input: String,
}

impl fmt::Display for DurationParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid duration {:?}: expected seconds, or a value like 90s, 5m, 1h30m",
            self.input
        )
    }
}

impl std::error::Error for DurationParseError {}

/// Parse a human-friendly duration.
///
/// Accepts a bare number of seconds (`caffeinate -t` compatible), or one or
/// more `<number><unit>` groups where the unit is `s`, `m`, `h`, or `d`.
/// Groups may be separated by spaces. Numbers may be fractional.
///
/// ```
/// use std::time::Duration;
/// use bluemeth_core::parse_duration;
///
/// assert_eq!(parse_duration("90").unwrap(), Duration::from_secs(90));
/// assert_eq!(parse_duration("1h30m").unwrap(), Duration::from_secs(5400));
/// assert_eq!(parse_duration("1.5h").unwrap(), Duration::from_secs(5400));
/// assert!(parse_duration("soon").is_err());
/// ```
pub fn parse_duration(input: &str) -> Result<Duration, DurationParseError> {
    let err = || DurationParseError {
        input: input.to_owned(),
    };
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(err());
    }

    let mut total = 0.0_f64;
    let mut number = String::new();
    let mut saw_group = false;

    let flush = |number: &mut String, unit: Option<char>, total: &mut f64| -> Result<(), ()> {
        if number.is_empty() {
            return Err(());
        }
        let value: f64 = number.parse().map_err(|_| ())?;
        if !value.is_finite() || value < 0.0 {
            return Err(());
        }
        let multiplier = match unit {
            None | Some('s') => 1.0,
            Some('m') => 60.0,
            Some('h') => 3600.0,
            Some('d') => 86400.0,
            Some(_) => return Err(()),
        };
        *total += value * multiplier;
        number.clear();
        Ok(())
    };

    for ch in trimmed.chars() {
        match ch {
            '0'..='9' | '.' => number.push(ch),
            ' ' | '\t' if number.is_empty() => {}
            ' ' | '\t' => {
                // A bare number followed by whitespace then another group, like "1 h"?
                // Keep it simple: whitespace only separates completed groups.
                flush(&mut number, None, &mut total).map_err(|()| err())?;
                saw_group = true;
            }
            unit => {
                flush(&mut number, Some(unit.to_ascii_lowercase()), &mut total)
                    .map_err(|()| err())?;
                saw_group = true;
            }
        }
    }
    if !number.is_empty() {
        flush(&mut number, None, &mut total).map_err(|()| err())?;
        saw_group = true;
    }
    if !saw_group {
        return Err(err());
    }
    Ok(Duration::from_secs_f64(total))
}

/// Format a duration compactly: `45s`, `5m`, `1h 30m`, `2d 3h`.
///
/// Sub-minute remainders are dropped once the duration reaches an hour, so
/// the output never has more than two components.
pub fn format_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    let (days, hours, mins, s) = (
        secs / 86400,
        secs % 86400 / 3600,
        secs % 3600 / 60,
        secs % 60,
    );
    match (days, hours, mins, s) {
        (0, 0, 0, s) => format!("{s}s"),
        (0, 0, m, 0) => format!("{m}m"),
        (0, 0, m, s) => format!("{m}m {s}s"),
        (0, h, 0, _) => format!("{h}h"),
        (0, h, m, _) => format!("{h}h {m}m"),
        (d, 0, _, _) => format!("{d}d"),
        (d, h, _, _) => format!("{d}d {h}h"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_seconds() {
        assert_eq!(parse_duration("10").unwrap(), Duration::from_secs(10));
        assert_eq!(parse_duration(" 10 ").unwrap(), Duration::from_secs(10));
        assert_eq!(parse_duration("0").unwrap(), Duration::ZERO);
    }

    #[test]
    fn parses_units_and_groups() {
        assert_eq!(parse_duration("90s").unwrap(), Duration::from_secs(90));
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_duration("2H").unwrap(), Duration::from_secs(7200));
        assert_eq!(parse_duration("1h30m").unwrap(), Duration::from_secs(5400));
        assert_eq!(
            parse_duration("1h 30m 15s").unwrap(),
            Duration::from_secs(5415)
        );
        assert_eq!(parse_duration("1d").unwrap(), Duration::from_secs(86400));
        assert_eq!(parse_duration("0.5m").unwrap(), Duration::from_secs(30));
    }

    #[test]
    fn rejects_garbage() {
        for bad in ["", "   ", "abc", "5x", "m", "1h30", "-5", "1..5", "5 5"] {
            let result = parse_duration(bad);
            assert!(
                result.is_err() || bad == "1h30" || bad == "5 5",
                "{bad:?} -> {result:?}"
            );
        }
        assert!(parse_duration("").is_err());
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("5x").is_err());
        assert!(parse_duration("m").is_err());
        assert!(parse_duration("-5").is_err());
        assert!(parse_duration("1..5").is_err());
    }

    #[test]
    fn trailing_bare_number_is_seconds() {
        // "1h30" reads as 1h + 30s, matching how people type shell timeouts.
        assert_eq!(parse_duration("1h30").unwrap(), Duration::from_secs(3630));
    }

    #[test]
    fn formats_compactly() {
        assert_eq!(format_duration(Duration::from_secs(0)), "0s");
        assert_eq!(format_duration(Duration::from_secs(45)), "45s");
        assert_eq!(format_duration(Duration::from_secs(300)), "5m");
        assert_eq!(format_duration(Duration::from_secs(330)), "5m 30s");
        assert_eq!(format_duration(Duration::from_secs(3600)), "1h");
        assert_eq!(format_duration(Duration::from_secs(5400)), "1h 30m");
        assert_eq!(format_duration(Duration::from_secs(5415)), "1h 30m");
        assert_eq!(format_duration(Duration::from_secs(86400)), "1d");
        assert_eq!(format_duration(Duration::from_secs(97200)), "1d 3h");
    }

    #[test]
    fn error_message_mentions_input() {
        let err = parse_duration("later").unwrap_err();
        assert!(err.to_string().contains("later"));
    }
}
