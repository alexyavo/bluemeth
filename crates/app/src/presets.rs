//! Timeout choices shown in the menu.

/// A labelled timeout. `secs == 0` means no timeout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preset {
    pub label: &'static str,
    pub secs: u64,
}

/// Presets in menu order.
pub const PRESETS: &[Preset] = &[
    Preset {
        label: "Never",
        secs: 0,
    },
    Preset {
        label: "5 minutes",
        secs: 5 * 60,
    },
    Preset {
        label: "15 minutes",
        secs: 15 * 60,
    },
    Preset {
        label: "30 minutes",
        secs: 30 * 60,
    },
    Preset {
        label: "1 hour",
        secs: 3600,
    },
    Preset {
        label: "2 hours",
        secs: 2 * 3600,
    },
    Preset {
        label: "4 hours",
        secs: 4 * 3600,
    },
];

/// Whether `secs` is one of the presets (so "Custom…" should not be checked).
pub fn is_preset(secs: u64) -> bool {
    PRESETS.iter().any(|p| p.secs == secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_is_first_and_zero() {
        assert_eq!(PRESETS[0].secs, 0);
        assert!(is_preset(0));
        assert!(is_preset(3600));
        assert!(!is_preset(5400));
    }

    #[test]
    fn presets_are_strictly_increasing() {
        assert!(PRESETS.windows(2).all(|w| w[0].secs < w[1].secs));
    }
}
