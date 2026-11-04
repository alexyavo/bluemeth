//! CoreGraphics input simulation. An F20 press-and-release pair refreshes
//! input inactivity without generating any mouse events.
//! Only the menu action may request permission; background checks never do.

use std::ffi::c_void;
use std::ptr::{self, NonNull};

use objc2_core_foundation::{CFRetained, CFType};

use crate::simulation::IDLE_THRESHOLD;

pub const ACCESS_REQUIRED: &str = "Simulate Activity needs Accessibility access. Enable bluemeth in System Settings › Privacy & Security › Accessibility, then turn Simulate Activity on again.";
pub const ACCESS_ERROR: &str = "Simulate Activity: Accessibility access required";

const COMBINED_SESSION_STATE: i32 = 0;
const ANY_INPUT_EVENT: u32 = u32::MAX;
// kVK_F20 from HIToolbox/Events.h.
const F20_KEY_CODE: u16 = 0x5A;
const HID_EVENT_TAP: u32 = 0;
// Shift, Control, Option, Command and Fn. Caps Lock is a toggle, not a held key.
const HELD_MODIFIERS: u64 = (1 << 17) | (1 << 18) | (1 << 19) | (1 << 20) | (1 << 23);

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightPostEventAccess() -> bool;
    fn CGRequestPostEventAccess() -> bool;
    fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
    fn CGEventSourceFlagsState(state: i32) -> u64;
    fn CGEventSourceButtonState(state: i32, button: u32) -> bool;
    fn CGEventSourceKeyState(state: i32, key: u16) -> bool;
    fn CGEventCreateKeyboardEvent(source: *const c_void, key: u16, key_down: bool) -> *mut CFType;
    fn CGEventPost(tap: u32, event: &CFType);
}

pub fn has_access() -> bool {
    // SAFETY: no arguments; only queries the current process's TCC access.
    unsafe { CGPreflightPostEventAccess() }
}

pub fn request_access() -> bool {
    // SAFETY: no arguments; called only after the user selects the option.
    has_access() || unsafe { CGRequestPostEventAccess() }
}

fn idle_seconds() -> f64 {
    // SAFETY: documented state and event constants; no borrowed resources.
    unsafe { CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION_STATE, ANY_INPUT_EVENT) }
}

fn input_is_held() -> bool {
    // SAFETY: valid session state, virtual key code and supported button indices.
    unsafe {
        CGEventSourceFlagsState(COMBINED_SESSION_STATE) & HELD_MODIFIERS != 0
            || CGEventSourceKeyState(COMBINED_SESSION_STATE, F20_KEY_CODE)
            || (0..32).any(|button| CGEventSourceButtonState(COMBINED_SESSION_STATE, button))
    }
}

/// Returns whether an F20 press-and-release pair was posted. Real activity,
/// held mouse buttons, modifiers or F20 postpone simulation.
pub fn pulse_if_idle() -> Result<bool, &'static str> {
    if !has_access() {
        return Err(ACCESS_ERROR);
    }
    if !idle_enough(idle_seconds()) || input_is_held() {
        return Ok(false);
    }
    post_f20_keypress()?;
    Ok(true)
}

fn idle_enough(seconds: f64) -> bool {
    seconds.is_finite() && seconds >= IDLE_THRESHOLD.as_secs_f64()
}

fn post_f20_keypress() -> Result<(), &'static str> {
    // Allocate both events before posting either: allocation failure must
    // never leave a key-down in the event stream without its matching key-up.
    let down = f20_event(true)?;
    let up = f20_event(false)?;
    // SAFETY: both objects are live CGEvents; the caller checked posting
    // access and that F20/modifiers are not held. Release immediately, with
    // no fallible operation between the two posts.
    unsafe {
        CGEventPost(HID_EVENT_TAP, &down);
        CGEventPost(HID_EVENT_TAP, &up);
    }
    Ok(())
}

fn f20_event(key_down: bool) -> Result<CFRetained<CFType>, &'static str> {
    // SAFETY: a null source requests the default source; F20 is a valid macOS
    // virtual key code. The +1 CF object is adopted exactly once and released
    // by CFRetained, including if allocation of the matching event fails.
    unsafe {
        Ok(CFRetained::from_raw(
            NonNull::new(CGEventCreateKeyboardEvent(
                ptr::null(),
                F20_KEY_CODE,
                key_down,
            ))
            .ok_or("Could not create an F20 activity event")?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_or_invalid_idle_readings_do_not_trigger_input() {
        assert!(!idle_enough(0.0));
        assert!(!idle_enough(59.9));
        assert!(!idle_enough(-1.0));
        assert!(!idle_enough(f64::NAN));
        assert!(!idle_enough(f64::INFINITY));
        assert!(idle_enough(60.0));
        assert!(idle_enough(600.0));
    }

    #[test]
    #[ignore = "presses and releases F20; needs Accessibility access and an idle desktop"]
    fn f20_resets_input_idle_time_and_releases_key() {
        assert!(has_access(), "{ACCESS_REQUIRED}");
        // Wait briefly for an idle interval instead of racing ongoing input.
        for _ in 0..100 {
            if idle_seconds() >= 1.0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let before = idle_seconds();
        assert!(
            before >= 1.0,
            "leave the mouse and keyboard idle during this test"
        );
        assert!(!input_is_held());
        post_f20_keypress().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));
        let after = idle_seconds();
        eprintln!("Input idle seconds: {before:.3} → {after:.3}");
        assert!(
            after < 0.5,
            "idle timer did not reset: {before:.3} → {after:.3}"
        );
        // SAFETY: valid session state and F20 virtual key code.
        assert!(!unsafe { CGEventSourceKeyState(COMBINED_SESSION_STATE, F20_KEY_CODE) });
    }
}
