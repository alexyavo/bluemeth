//! macOS backend: IOKit power-management assertions, spoken directly.
//!
//! `/usr/bin/caffeinate` is a thin wrapper over exactly these calls
//! (`IOPMAssertionCreateWithProperties`, `IOPMAssertionDeclareUserActivity`,
//! `IOPMAssertionRelease`). Calling them in-process means one fewer process,
//! no `fork`/`exec`, no pipe or signal plumbing, and release is a single
//! synchronous call instead of "kill and hope".

mod assertion;
pub mod caffeinate;
pub mod ffi;
pub mod inspect;
pub mod rusage;

pub use assertion::{Assertion, UserActivity};

use std::time::Duration;

/// Whether a failed `IOPMAssertionRelease` means "powerd no longer knows this
/// id", which happens after powerd released it via `TimeoutActionRelease`.
///
/// `kIOReturnNotFound` is the documented code; in practice (macOS 27)
/// `kIOReturnBadArgument` is what comes back, so both are accepted. An id we
/// obtained from a successful create cannot be malformed, so for a release
/// call there is no other meaning for either code.
pub(crate) fn is_already_released(code: i32) -> bool {
    code == ffi::K_IORETURN_NOT_FOUND || code == ffi::K_IORETURN_BAD_ARGUMENT
}

/// `CLOCK_MONOTONIC` on Darwin keeps advancing during system sleep (unlike
/// `CLOCK_UPTIME_RAW`, which backs `std::time::Instant`).
pub(crate) fn monotonic_including_sleep() -> Duration {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid, writable timespec and CLOCK_MONOTONIC is always
    // available on Darwin; clock_gettime only writes through the pointer.
    let rc = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &raw mut ts) };
    debug_assert_eq!(rc, 0, "clock_gettime(CLOCK_MONOTONIC) cannot fail");
    Duration::new(
        ts.tv_sec.max(0) as u64,
        ts.tv_nsec.clamp(0, 999_999_999) as u32,
    )
}
