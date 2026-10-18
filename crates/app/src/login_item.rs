//! "Launch at Login" through `SMAppService` (macOS 13+).

use objc2_foundation::{NSBundle, NSOperatingSystemVersion, NSProcessInfo};
use objc2_service_management::{SMAppService, SMAppServiceStatus};

/// Current state of the login item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginItem {
    Enabled,
    Disabled,
    /// Registered, but the user must approve it in System Settings.
    RequiresApproval,
    /// Cannot be used right now, with a short reason for the UI.
    Unavailable(&'static str),
}

fn available() -> Result<(), &'static str> {
    if NSBundle::mainBundle().bundleIdentifier().is_none() {
        return Err("Available when running from bluemeth.app");
    }
    let macos13 = NSOperatingSystemVersion {
        majorVersion: 13,
        minorVersion: 0,
        patchVersion: 0,
    };
    if !NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(macos13) {
        return Err("Requires macOS 13 or later");
    }
    Ok(())
}

/// Query launchd for the login item's status.
pub fn status() -> LoginItem {
    if let Err(why) = available() {
        return LoginItem::Unavailable(why);
    }
    // SAFETY: `SMAppService.mainApp` has no preconditions beyond running from
    // an app bundle on macOS 13+, both checked by `available()`.
    let service = unsafe { SMAppService::mainAppService() };
    // SAFETY: `status` is a plain getter on a valid service object.
    match unsafe { service.status() } {
        SMAppServiceStatus::Enabled => LoginItem::Enabled,
        SMAppServiceStatus::RequiresApproval => LoginItem::RequiresApproval,
        _ => LoginItem::Disabled,
    }
}

/// Register or unregister the app as a login item.
pub fn set_enabled(enabled: bool) -> Result<(), String> {
    available().map_err(str::to_owned)?;
    // SAFETY: see `status()`.
    let service = unsafe { SMAppService::mainAppService() };
    // SAFETY: both calls are synchronous requests to launchd on a valid
    // service object and report failure through the returned NSError.
    let result = unsafe {
        if enabled {
            service.registerAndReturnError()
        } else {
            service.unregisterAndReturnError()
        }
    };
    result.map_err(|err| err.localizedDescription().to_string())
}
