//! Persistence through `NSUserDefaults` (no files of our own, no extra deps).

use objc2_foundation::{NSString, NSUserDefaults, ns_string};

use crate::settings::Settings;

fn key_display() -> &'static NSString {
    ns_string!("preventDisplaySleep")
}
fn key_idle() -> &'static NSString {
    ns_string!("preventIdleSleep")
}
fn key_system() -> &'static NSString {
    ns_string!("preventSystemSleep")
}
fn key_user_active() -> &'static NSString {
    ns_string!("declareUserActive")
}
fn key_timeout() -> &'static NSString {
    ns_string!("timeoutSeconds")
}

/// Load settings, or the defaults on first launch.
pub fn load() -> Settings {
    let defaults = NSUserDefaults::standardUserDefaults();
    if defaults.objectForKey(key_display()).is_none() {
        return Settings::default();
    }
    Settings {
        display: defaults.boolForKey(key_display()),
        idle: defaults.boolForKey(key_idle()),
        system: defaults.boolForKey(key_system()),
        user_active: defaults.boolForKey(key_user_active()),
        timeout_secs: u64::try_from(defaults.integerForKey(key_timeout())).unwrap_or(0),
    }
}

/// Persist settings. `NSUserDefaults` writes asynchronously and on exit.
pub fn save(settings: &Settings) {
    let defaults = NSUserDefaults::standardUserDefaults();
    defaults.setBool_forKey(settings.display, key_display());
    defaults.setBool_forKey(settings.idle, key_idle());
    defaults.setBool_forKey(settings.system, key_system());
    defaults.setBool_forKey(settings.user_active, key_user_active());
    defaults.setInteger_forKey(
        isize::try_from(settings.timeout_secs).unwrap_or(isize::MAX),
        key_timeout(),
    );
}
