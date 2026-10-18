//! Text formatting for the status bar.

use std::time::Duration;

/// Countdown text: `M:SS` under an hour, `H:MM:SS` from one hour up.
/// Rounds *up*, so the display reads `0:01` until release and never `0:00`.
pub fn countdown(remaining: Duration) -> String {
    let secs = remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0);
    let (h, m, s) = (secs / 3600, secs % 3600 / 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_like_a_kitchen_timer() {
        assert_eq!(countdown(Duration::from_secs(9)), "0:09");
        assert_eq!(countdown(Duration::from_secs(65)), "1:05");
        assert_eq!(countdown(Duration::from_secs(3599)), "59:59");
        assert_eq!(countdown(Duration::from_secs(3600)), "1:00:00");
        assert_eq!(countdown(Duration::from_secs(4 * 3600 + 7)), "4:00:07");
    }

    #[test]
    fn rounds_up_and_never_shows_zero_until_done() {
        assert_eq!(countdown(Duration::from_millis(300)), "0:01");
        assert_eq!(countdown(Duration::from_millis(1001)), "0:02");
        assert_eq!(countdown(Duration::ZERO), "0:00");
    }
}
