use std::time::Duration;

/// A kind of sleep that can be prevented with a power assertion.
///
/// Each variant maps to one `caffeinate` flag and one IOKit assertion type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AssertionKind {
    /// Keep the display awake (`caffeinate -d`).
    DisplaySleep,
    /// Prevent idle system sleep (`caffeinate -i`, caffeinate's default).
    IdleSystemSleep,
    /// Prevent system sleep; honoured only on AC power (`caffeinate -s`).
    SystemSleep,
    /// Prevent the disk from idling (`caffeinate -m`).
    DiskIdle,
}

impl AssertionKind {
    /// Every kind, in `caffeinate` flag order.
    pub const ALL: [Self; 4] = [
        Self::DisplaySleep,
        Self::IdleSystemSleep,
        Self::SystemSleep,
        Self::DiskIdle,
    ];

    /// The single-letter `caffeinate` flag for this kind.
    pub const fn flag(self) -> char {
        match self {
            Self::DisplaySleep => 'd',
            Self::IdleSystemSleep => 'i',
            Self::SystemSleep => 's',
            Self::DiskIdle => 'm',
        }
    }

    /// The IOKit assertion type string (`kIOPMAssertionTypeKey` value).
    pub const fn iokit_type(self) -> &'static str {
        match self {
            Self::DisplaySleep => "PreventUserIdleDisplaySleep",
            Self::IdleSystemSleep => "PreventUserIdleSystemSleep",
            Self::SystemSleep => "PreventSystemSleep",
            Self::DiskIdle => "PreventDiskIdle",
        }
    }

    /// Short human-readable label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::DisplaySleep => "Prevent Display Sleep",
            Self::IdleSystemSleep => "Prevent Idle Sleep",
            Self::SystemSleep => "Prevent System Sleep",
            Self::DiskIdle => "Prevent Disk Idle",
        }
    }

    /// One-line description of what the assertion does.
    pub const fn description(self) -> &'static str {
        match self {
            Self::DisplaySleep => "Keeps the display awake",
            Self::IdleSystemSleep => "Prevents idle sleep (default caffeinate behavior)",
            Self::SystemSleep => "Prevents system sleep (AC power only)",
            Self::DiskIdle => "Keeps the disk spinning",
        }
    }
}

/// Which kinds of sleep to prevent, plus an optional overall timeout.
///
/// This is a plain value type: it describes *intent*. Hand it to
/// [`Inhibitor::apply`](crate::Inhibitor::apply) to make the OS match it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Config {
    /// `caffeinate -d`
    pub prevent_display_sleep: bool,
    /// `caffeinate -i`
    pub prevent_idle_sleep: bool,
    /// `caffeinate -s`
    pub prevent_system_sleep: bool,
    /// `caffeinate -m`
    pub prevent_disk_idle: bool,
    /// `caffeinate -u`: periodically declare that the user is active, which
    /// turns the display on if it is off and resets the system idle timer.
    pub declare_user_active: bool,
    /// Release everything after this long. `None` means until stopped.
    pub timeout: Option<Duration>,
}

impl Config {
    /// The four assertions a desktop keep-awake tool normally offers
    /// (display, idle, system, user-active), with no timeout.
    pub const fn all() -> Self {
        Self {
            prevent_display_sleep: true,
            prevent_idle_sleep: true,
            prevent_system_sleep: true,
            prevent_disk_idle: false,
            declare_user_active: true,
            timeout: None,
        }
    }

    /// `caffeinate`'s behaviour when given no flags: prevent idle sleep only.
    pub const fn idle_only() -> Self {
        Self {
            prevent_display_sleep: false,
            prevent_idle_sleep: true,
            prevent_system_sleep: false,
            prevent_disk_idle: false,
            declare_user_active: false,
            timeout: None,
        }
    }

    /// Whether `kind` is enabled.
    pub const fn is_enabled(&self, kind: AssertionKind) -> bool {
        match kind {
            AssertionKind::DisplaySleep => self.prevent_display_sleep,
            AssertionKind::IdleSystemSleep => self.prevent_idle_sleep,
            AssertionKind::SystemSleep => self.prevent_system_sleep,
            AssertionKind::DiskIdle => self.prevent_disk_idle,
        }
    }

    /// Enable or disable `kind`.
    pub fn set(&mut self, kind: AssertionKind, enabled: bool) {
        match kind {
            AssertionKind::DisplaySleep => self.prevent_display_sleep = enabled,
            AssertionKind::IdleSystemSleep => self.prevent_idle_sleep = enabled,
            AssertionKind::SystemSleep => self.prevent_system_sleep = enabled,
            AssertionKind::DiskIdle => self.prevent_disk_idle = enabled,
        }
    }

    /// Builder-style [`set`](Self::set).
    #[must_use]
    pub fn with(mut self, kind: AssertionKind, enabled: bool) -> Self {
        self.set(kind, enabled);
        self
    }

    /// Builder-style timeout.
    #[must_use]
    pub const fn with_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.timeout = timeout;
        self
    }

    /// The enabled assertion kinds, in [`AssertionKind::ALL`] order.
    pub fn kinds(&self) -> impl Iterator<Item = AssertionKind> + '_ {
        AssertionKind::ALL
            .into_iter()
            .filter(move |kind| self.is_enabled(*kind))
    }

    /// Whether anything at all would be held (any assertion or user activity).
    pub fn any_enabled(&self) -> bool {
        self.declare_user_active || self.kinds().next().is_some()
    }

    /// Whether every assertion kind in `kinds` plus user-active is enabled.
    pub fn all_enabled_of(&self, kinds: &[AssertionKind]) -> bool {
        self.declare_user_active && kinds.iter().all(|k| self.is_enabled(*k))
    }

    /// Arguments that make `/usr/bin/caffeinate` behave like this config.
    pub fn caffeinate_args(&self) -> Vec<String> {
        let mut flags = String::from("-");
        for kind in self.kinds() {
            flags.push(kind.flag());
        }
        if self.declare_user_active {
            flags.push('u');
        }
        let mut args = Vec::new();
        if flags.len() > 1 {
            args.push(flags);
        }
        if let Some(timeout) = self.timeout {
            args.push("-t".to_owned());
            args.push(timeout.as_secs().max(1).to_string());
        }
        args
    }

    /// The equivalent `caffeinate` command line, for display.
    pub fn caffeinate_command_line(&self) -> String {
        let mut line = String::from("caffeinate");
        for arg in self.caffeinate_args() {
            line.push(' ');
            line.push_str(&arg);
        }
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_follow_flag_order() {
        let cfg = Config::all().with(AssertionKind::DiskIdle, true);
        let kinds: Vec<_> = cfg.kinds().collect();
        assert_eq!(kinds, AssertionKind::ALL);
    }

    #[test]
    fn caffeinate_args_match_flags() {
        let cfg = Config::all().with_timeout(Some(Duration::from_secs(10)));
        assert_eq!(cfg.caffeinate_args(), ["-disu", "-t", "10"]);
        assert_eq!(cfg.caffeinate_command_line(), "caffeinate -disu -t 10");
        assert_eq!(Config::idle_only().caffeinate_args(), ["-i"]);
        assert!(Config::default().caffeinate_args().is_empty());
    }

    #[test]
    fn sub_second_timeout_rounds_up_for_caffeinate() {
        let cfg = Config::idle_only().with_timeout(Some(Duration::from_millis(200)));
        assert_eq!(cfg.caffeinate_args(), ["-i", "-t", "1"]);
    }

    #[test]
    fn any_enabled_counts_user_activity() {
        let cfg = Config {
            declare_user_active: true,
            ..Config::default()
        };
        assert!(cfg.any_enabled());
        assert!(!Config::default().any_enabled());
    }

    #[test]
    fn set_and_is_enabled_round_trip() {
        let mut cfg = Config::default();
        for kind in AssertionKind::ALL {
            cfg.set(kind, true);
            assert!(cfg.is_enabled(kind));
            cfg.set(kind, false);
            assert!(!cfg.is_enabled(kind));
        }
    }
}
