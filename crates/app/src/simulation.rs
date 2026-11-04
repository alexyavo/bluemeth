//! Scheduling for optional input simulation, including when no power
//! assertions are enabled. Uses the same sleep-inclusive clock as the core.

use std::time::Duration;

use bluemeth_core::Monotonic;

pub const CHECK_INTERVAL: Duration = Duration::from_secs(30);
pub const IDLE_THRESHOLD: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct Simulation {
    next_check: Option<Monotonic>,
    deadline: Option<Monotonic>,
}

impl Simulation {
    /// Adopt the session deadline without postponing an already scheduled
    /// check when another setting changes. Enabling never posts immediately.
    pub fn configure(&mut self, enabled: bool, deadline: Option<Monotonic>, now: Monotonic) {
        if enabled {
            self.next_check.get_or_insert(now + CHECK_INTERVAL);
            self.deadline = deadline;
        } else {
            self.next_check = None;
            self.deadline = None;
        }
    }

    pub fn is_active(&self) -> bool {
        self.next_check.is_some()
    }

    /// A delayed timer does one check, never a burst of catch-up events.
    pub fn check_due(&mut self, now: Monotonic) -> bool {
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            return false;
        }
        if self.next_check.is_some_and(|next| now >= next) {
            self.next_check = Some(now + CHECK_INTERVAL);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use bluemeth_core::now;

    use super::*;

    #[test]
    fn checks_only_when_enabled_and_due() {
        let start = now();
        let mut simulation = Simulation::default();
        assert!(!simulation.check_due(start));
        simulation.configure(true, None, start);
        assert!(!simulation.check_due(start));
        assert!(!simulation.check_due(start + Duration::from_secs(29)));
        assert!(simulation.check_due(start + CHECK_INTERVAL));
        assert!(!simulation.check_due(start + CHECK_INTERVAL));
    }

    #[test]
    fn reconfiguration_does_not_starve_checks() {
        let start = now();
        let mut simulation = Simulation::default();
        simulation.configure(true, None, start);
        simulation.configure(true, None, start + Duration::from_secs(25));
        assert!(simulation.check_due(start + CHECK_INTERVAL));
    }

    #[test]
    fn never_posts_at_or_after_timeout_even_after_sleep() {
        let start = now();
        let mut simulation = Simulation::default();
        simulation.configure(true, Some(start + CHECK_INTERVAL), start);
        assert!(!simulation.check_due(start + CHECK_INTERVAL));
        assert!(!simulation.check_due(start + Duration::from_secs(3600)));
    }

    #[test]
    fn disable_cancels_pending_work_and_reenable_starts_fresh() {
        let start = now();
        let mut simulation = Simulation::default();
        simulation.configure(true, Some(start + Duration::from_secs(90)), start);
        simulation.configure(false, None, start);
        assert!(!simulation.is_active());
        assert_eq!(simulation.deadline, None);
        let later = start + Duration::from_secs(120);
        assert!(!simulation.check_due(later));
        simulation.configure(true, None, later);
        assert!(!simulation.check_due(later));
        assert!(simulation.check_due(later + CHECK_INTERVAL));
    }

    #[test]
    fn delayed_timer_does_not_catch_up_with_multiple_events() {
        let start = now();
        let mut simulation = Simulation::default();
        simulation.configure(true, None, start);
        let later = start + Duration::from_secs(600);
        assert!(simulation.check_due(later));
        assert!(!simulation.check_due(later));
        assert!(!simulation.check_due(later + Duration::from_secs(1)));
    }
}
