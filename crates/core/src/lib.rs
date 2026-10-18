//! Cross-platform sleep-inhibition primitives.
//!
//! On macOS this crate talks directly to IOKit power-management assertions,
//! the same kernel-facing API that `/usr/bin/caffeinate` wraps, but without
//! spawning a subprocess. The central type is [`Inhibitor`]: give it a
//! [`Config`] describing which kinds of sleep to prevent and it creates,
//! releases, or recreates the underlying assertions to match.
//!
//! Every assertion is an RAII handle, so dropping an [`Inhibitor`] (or the
//! process exiting for any reason, including a crash) releases everything.
//!
//! Other platforms currently compile against a stub backend that returns
//! [`Error::Unsupported`]; see `docs/PLATFORMS.md` for the planned backends.

#![forbid(unsafe_op_in_unsafe_fn)]

mod clock;
mod config;
mod duration;
mod error;
mod inhibitor;

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "macos")]
use macos as backend;

#[cfg(not(target_os = "macos"))]
mod unsupported;
#[cfg(not(target_os = "macos"))]
use unsupported as backend;

pub use clock::{Monotonic, now};
pub use config::{AssertionKind, Config};
pub use duration::{DurationParseError, format_duration, parse_duration};
pub use error::Error;
pub use inhibitor::Inhibitor;

/// Convenience alias for results produced by this crate.
pub type Result<T> = std::result::Result<T, Error>;
