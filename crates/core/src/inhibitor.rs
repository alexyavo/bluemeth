use std::time::Duration;

use crate::backend;
use crate::clock::{self, Monotonic};
use crate::{AssertionKind, Config, Error, Result};

/// Holds the set of sleep-inhibiting assertions described by a [`Config`].
///
/// An `Inhibitor` is a reconciler: each call to [`apply`](Self::apply)
/// creates the assertions that are newly wanted, releases the ones that are
/// no longer wanted, and leaves the rest untouched. Nothing is spawned; on
/// macOS every assertion is one IPC call to `powerd`.
///
/// Dropping the inhibitor releases everything. The OS also releases all
/// assertions owned by a process when it exits, so there is no way to leak
/// one past the lifetime of the process.
#[derive(Debug)]
pub struct Inhibitor {
    name: String,
    reason: String,
    held: Vec<Held>,
    user_activity: Option<backend::UserActivity>,
    timeout: Option<Duration>,
    deadline: Option<Monotonic>,
    last_user_activity: Option<Monotonic>,
}

#[derive(Debug)]
struct Held {
    kind: AssertionKind,
    assertion: backend::Assertion,
}

impl Inhibitor {
    /// How often [`refresh_user_activity`](Self::refresh_user_activity) should
    /// be called while `declare_user_active` is enabled. The shortest display
    /// sleep setting macOS offers is one minute, so half of that keeps the
    /// idle timer from ever reaching it.
    pub const USER_ACTIVITY_INTERVAL: Duration = Duration::from_secs(30);

    /// Create an inhibitor that holds nothing yet.
    ///
    /// `name` is shown by `pmset -g assertions` and Activity Monitor as the
    /// owner of the assertions, so make it recognisable (e.g. the app name).
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        let reason = format!("{name} is keeping this Mac awake");
        Self {
            name,
            reason,
            held: Vec::new(),
            user_activity: None,
            timeout: None,
            deadline: None,
            last_user_activity: None,
        }
    }

    /// Set the human-readable reason attached to each assertion.
    #[must_use]
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = reason.into();
        self
    }

    /// Make what is held match `config`.
    ///
    /// Changing the timeout recreates every assertion, because on macOS the
    /// timeout is a property baked into each assertion at creation time (the
    /// same mechanism `caffeinate -t` uses). Toggling individual kinds while
    /// the timeout is unchanged keeps the existing deadline, so everything
    /// still expires together.
    ///
    /// On partial failure the assertions that *did* succeed stay held, the
    /// first error is returned, and [`current`](Self::current) reports what
    /// is really active.
    pub fn apply(&mut self, config: &Config) -> Result<()> {
        let now = clock::now();
        let mut first_err: Option<Error> = None;
        let mut note = |result: Result<()>| {
            if let Err(e) = result {
                first_err.get_or_insert(e);
            }
        };

        if config.timeout != self.timeout {
            note(self.release_all());
            self.timeout = config.timeout;
        }

        if !config.any_enabled() {
            note(self.release_all());
            return first_err.map_or(Ok(()), Err);
        }

        if self.deadline.is_none_or(|d| d <= now) {
            self.deadline = self.timeout.map(|t| now + t);
        }
        let per_assertion_timeout = self.deadline.map(|d| d.remaining_from(now));

        let (keep, release): (Vec<Held>, Vec<Held>) =
            self.held.drain(..).partition(|h| config.is_enabled(h.kind));
        self.held = keep;
        for held in release {
            note(ignore_already_released(held.assertion.release()));
        }

        for kind in config.kinds() {
            if self.held.iter().any(|h| h.kind == kind) {
                continue;
            }
            let details = match per_assertion_timeout {
                Some(t) => format!("asserting for {} secs", t.as_secs()),
                None => "asserting until stopped".to_owned(),
            };
            match backend::Assertion::create(
                kind,
                &self.name,
                &details,
                &self.reason,
                per_assertion_timeout,
            ) {
                Ok(assertion) => self.held.push(Held { kind, assertion }),
                Err(e) => note(Err(e)),
            }
        }

        if config.declare_user_active {
            if self.user_activity.is_none() {
                match backend::UserActivity::declare(&self.name) {
                    Ok(activity) => {
                        self.user_activity = Some(activity);
                        self.last_user_activity = Some(now);
                    }
                    Err(e) => note(Err(e)),
                }
            }
        } else if let Some(activity) = self.user_activity.take() {
            self.last_user_activity = None;
            note(ignore_already_released(activity.release()));
        }

        if !self.is_active() {
            self.deadline = None;
        }
        first_err.map_or(Ok(()), Err)
    }

    /// Like [`apply`](Self::apply) but always recreates everything, restarting
    /// the countdown from the full timeout.
    pub fn restart(&mut self, config: &Config) -> Result<()> {
        let released = self.release_all();
        self.timeout = None;
        self.deadline = None;
        let applied = self.apply(config);
        released.and(applied)
    }

    /// Release every assertion and stop declaring user activity.
    ///
    /// Returns the first non-trivial error. An assertion that the OS has
    /// already dropped (because its own timeout fired) is not an error.
    pub fn release_all(&mut self) -> Result<()> {
        let mut first_err = None;
        for held in self.held.drain(..) {
            if let Err(e) = ignore_already_released(held.assertion.release()) {
                first_err.get_or_insert(e);
            }
        }
        if let Some(activity) = self.user_activity.take() {
            if let Err(e) = ignore_already_released(activity.release()) {
                first_err.get_or_insert(e);
            }
        }
        self.last_user_activity = None;
        self.deadline = None;
        first_err.map_or(Ok(()), Err)
    }

    /// Re-declare user activity if it is enabled and
    /// [`USER_ACTIVITY_INTERVAL`](Self::USER_ACTIVITY_INTERVAL) has elapsed
    /// since the last declaration. Cheap to call often; does one IPC at most
    /// every interval.
    pub fn refresh_user_activity(&mut self) -> Result<()> {
        let Some(activity) = self.user_activity.as_mut() else {
            return Ok(());
        };
        let now = clock::now();
        let due = self
            .last_user_activity
            .is_none_or(|last| now - last >= Self::USER_ACTIVITY_INTERVAL);
        if due {
            activity.redeclare()?;
            self.last_user_activity = Some(now);
        }
        Ok(())
    }

    /// When the next user-activity declaration is due, if any.
    pub fn next_user_activity_due(&self) -> Option<Monotonic> {
        self.user_activity.as_ref()?;
        Some(
            self.last_user_activity
                .map_or_else(clock::now, |last| last + Self::USER_ACTIVITY_INTERVAL),
        )
    }

    /// If the deadline has passed, release everything and return `true`.
    ///
    /// On macOS the assertions have already been dropped by `powerd` at this
    /// point; this just brings our bookkeeping (and user-activity refreshes)
    /// in line with reality.
    pub fn poll_expired(&mut self) -> Result<bool> {
        match self.deadline {
            Some(deadline) if clock::now() >= deadline => {
                self.release_all()?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Whether any assertion or user-activity declaration is currently held.
    pub fn is_active(&self) -> bool {
        !self.held.is_empty() || self.user_activity.is_some()
    }

    /// What is actually held right now, as a [`Config`].
    pub fn current(&self) -> Config {
        let mut cfg = Config {
            declare_user_active: self.user_activity.is_some(),
            timeout: self.timeout,
            ..Config::default()
        };
        for held in &self.held {
            cfg.set(held.kind, true);
        }
        cfg
    }

    /// The point at which everything will be released, if a timeout is set.
    pub fn deadline(&self) -> Option<Monotonic> {
        self.deadline
    }

    /// Time left until the deadline, if a timeout is set and we are active.
    pub fn remaining(&self) -> Option<Duration> {
        let deadline = self.deadline?;
        Some(deadline.remaining_from(clock::now()))
    }

    /// The owner name passed to [`new`](Self::new).
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Drop for Inhibitor {
    fn drop(&mut self) {
        // Each `Held`/`UserActivity` releases itself in its own Drop; nothing
        // to do here except keep the type's Drop explicit for readers.
    }
}

fn ignore_already_released(result: Result<()>) -> Result<()> {
    match result {
        Err(Error::Release { code }) if backend::is_already_released(code) => Ok(()),
        other => other,
    }
}
