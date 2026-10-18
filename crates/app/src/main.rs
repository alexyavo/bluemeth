//! bluemeth: a minimal menu bar app that keeps your Mac awake.
//!
//! One `NSStatusItem`, one `NSMenu`, one controller object, and an
//! [`bluemeth_core::Inhibitor`] holding IOKit assertions in-process. No
//! windows, no subprocesses, no timers unless a countdown or a user-activity
//! refresh is actually needed.
//!
//! Started without arguments it runs the menu bar app. A few switches exist
//! for scripts (`make login`, `make unlogin`):
//!
//! ```text
//! bluemeth-app --login-item enable|disable|status
//! bluemeth-app --help | --version
//! ```
//!
//! Anything else on the command line is ignored, so nothing Launch Services
//! or a login-item launch might append can keep the app from starting.

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

#[cfg(target_os = "macos")]
const USAGE: &str = "\
usage: bluemeth-app                                  start the menu bar app
       bluemeth-app --login-item enable|disable|status
       bluemeth-app --help | --version
";

fn main() {
    #[cfg(target_os = "macos")]
    {
        if let Some(code) = run_switch() {
            std::process::exit(code);
        }
        controller::run();
    }
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!(
            "bluemeth's menu bar app is macOS-only; see docs/PLATFORMS.md for other platforms."
        );
        std::process::exit(1);
    }
}

/// Handle a command line switch. `None` means "no switch, start the app".
#[cfg(target_os = "macos")]
fn run_switch() -> Option<i32> {
    use login_item::LoginItem;

    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let args: Vec<&str> = args.iter().map(|arg| arg.to_str()).collect::<Option<_>>()?;
    let code = match args.as_slice() {
        ["--help" | "-h"] => {
            print!("{USAGE}");
            0
        }
        ["--version" | "-V"] => {
            println!("bluemeth-app {}", env!("CARGO_PKG_VERSION"));
            0
        }
        ["--login-item", "status"] => {
            println!("{}", describe(&login_item::status()));
            0
        }
        ["--login-item", action @ ("enable" | "disable")] => {
            let enable = *action == "enable";
            let status = login_item::status();
            let unchanged = (enable && status == LoginItem::Enabled)
                || (!enable && status == LoginItem::Disabled);
            if unchanged {
                println!("{}", describe(&status));
                return Some(0);
            }
            match login_item::set_enabled(enable) {
                Ok(()) => {
                    println!("{}", describe(&login_item::status()));
                    0
                }
                Err(err) => {
                    eprintln!("bluemeth-app: launch at login: {err}");
                    1
                }
            }
        }
        ["--login-item", ..] => {
            eprint!("{USAGE}");
            2
        }
        _ => return None,
    };
    Some(code)
}

#[cfg(target_os = "macos")]
fn describe(status: &login_item::LoginItem) -> String {
    use login_item::LoginItem;
    match status {
        LoginItem::Enabled => "launch at login: enabled".to_owned(),
        LoginItem::Disabled => "launch at login: disabled".to_owned(),
        LoginItem::RequiresApproval => {
            "launch at login: waiting for approval in System Settings > General > Login Items"
                .to_owned()
        }
        LoginItem::Unavailable(why) => format!("launch at login: unavailable ({why})"),
    }
}
