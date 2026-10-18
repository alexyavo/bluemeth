//! Stub backend for platforms without an implementation yet.

use std::time::{Duration, Instant};

use crate::{AssertionKind, Error, Result};

#[derive(Debug)]
pub struct Assertion;

impl Assertion {
    pub fn create(
        _kind: AssertionKind,
        _name: &str,
        _details: &str,
        _reason: &str,
        _timeout: Option<Duration>,
    ) -> Result<Self> {
        Err(Error::Unsupported)
    }

    pub fn release(self) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
pub struct UserActivity;

impl UserActivity {
    pub fn declare(_name: &str) -> Result<Self> {
        Err(Error::Unsupported)
    }

    pub fn redeclare(&mut self) -> Result<()> {
        Err(Error::Unsupported)
    }

    pub fn release(self) -> Result<()> {
        Ok(())
    }
}

pub fn is_already_released(_code: i32) -> bool {
    false
}

pub fn monotonic_including_sleep() -> Duration {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed()
}
