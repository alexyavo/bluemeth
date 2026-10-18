//! What the user asked for, independent of any UI toolkit.

use std::time::Duration;

use bluemeth_core::{AssertionKind, Config};

/// The assertion kinds offered in the menu, in display order.
/// Disk-idle is deliberately left to the CLI; it is rarely wanted on a laptop.
pub const MENU_KINDS: [AssertionKind; 3] = [
    AssertionKind::DisplaySleep,
    AssertionKind::IdleSystemSleep,
    AssertionKind::SystemSleep,
];

/// Persisted user settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub display: bool,
    pub idle: bool,
    pub system: bool,
    pub user_active: bool,
    /// 0 means no timeout.
    pub timeout_secs: u64,
}

impl Default for Settings {
    /// First-launch defaults: the three sleep preventions on, no timeout.
    /// "Declare User Active" also turns the display *on*, which is
    /// surprising as a default, so it starts off.
    fn default() -> Self {
        Self {
            display: true,
            idle: true,
            system: true,
            user_active: false,
            timeout_secs: 0,
        }
    }
}

impl Settings {
    pub fn kind(&self, kind: AssertionKind) -> bool {
        match kind {
            AssertionKind::DisplaySleep => self.display,
            AssertionKind::IdleSystemSleep => self.idle,
            AssertionKind::SystemSleep => self.system,
            AssertionKind::DiskIdle => false,
        }
    }

    pub fn set_kind(&mut self, kind: AssertionKind, on: bool) {
        match kind {
            AssertionKind::DisplaySleep => self.display = on,
            AssertionKind::IdleSystemSleep => self.idle = on,
            AssertionKind::SystemSleep => self.system = on,
            AssertionKind::DiskIdle => {}
        }
    }

    /// Every menu toggle is on.
    pub fn all_on(&self) -> bool {
        self.user_active && MENU_KINDS.iter().all(|k| self.kind(*k))
    }

    pub fn set_all(&mut self, on: bool) {
        for kind in MENU_KINDS {
            self.set_kind(kind, on);
        }
        self.user_active = on;
    }

    pub fn timeout(&self) -> Option<Duration> {
        (self.timeout_secs > 0).then(|| Duration::from_secs(self.timeout_secs))
    }

    pub fn to_config(self) -> Config {
        let mut cfg = Config {
            declare_user_active: self.user_active,
            timeout: self.timeout(),
            ..Config::default()
        };
        for kind in MENU_KINDS {
            cfg.set(kind, self.kind(kind));
        }
        cfg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_keep_awake_without_user_activity() {
        let s = Settings::default();
        assert!(!s.all_on());
        let cfg = s.to_config();
        assert!(cfg.prevent_display_sleep && cfg.prevent_idle_sleep && cfg.prevent_system_sleep);
        assert!(!cfg.declare_user_active);
        assert!(!cfg.prevent_disk_idle);
        assert_eq!(cfg.timeout, None);
    }

    #[test]
    fn set_all_round_trips() {
        let mut s = Settings::default();
        s.set_all(true);
        assert!(s.all_on());
        s.set_all(false);
        assert!(!s.to_config().any_enabled());
    }

    #[test]
    fn timeout_zero_means_none() {
        let mut s = Settings::default();
        assert_eq!(s.timeout(), None);
        s.timeout_secs = 90;
        assert_eq!(s.to_config().timeout, Some(Duration::from_secs(90)));
    }

    #[test]
    fn disk_idle_is_ignored() {
        let mut s = Settings::default();
        s.set_kind(AssertionKind::DiskIdle, true);
        assert!(!s.kind(AssertionKind::DiskIdle));
    }
}
