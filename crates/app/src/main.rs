//! Bluemeth: a minimal menu bar app that keeps your Mac awake.
//!
//! One `NSStatusItem`, one `NSMenu`, one controller object, and an
//! [`bluemeth_core::Inhibitor`] holding IOKit assertions in-process. No
//! windows, no subprocesses, no timers unless a countdown or a user-activity
//! refresh is actually needed.

#![forbid(unsafe_op_in_unsafe_fn)]
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

mod format;
mod presets;
mod settings;

#[cfg(target_os = "macos")]
mod controller;
#[cfg(target_os = "macos")]
mod login_item;
#[cfg(target_os = "macos")]
mod prefs;

fn main() {
    #[cfg(target_os = "macos")]
    controller::run();
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!(
            "Bluemeth's menu bar app is macOS-only; see docs/PLATFORMS.md for other platforms."
        );
        std::process::exit(1);
    }
}
