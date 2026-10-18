//! Command line parsing. Flag-compatible with `caffeinate(8)`.

use std::ffi::OsString;
use std::fmt;
use std::time::Duration;

use bluemeth_core::{Config, parse_duration};

/// Help text.
pub const USAGE: &str = "\
bluemeth: keep your Mac awake (caffeinate-compatible, no subprocess)

USAGE:
    bluemeth [-dimsu] [-t TIMEOUT] [-w PID] [--backend iokit|caffeinate] [utility [args...]]
    bluemeth status [PID]

OPTIONS:
    -d             Prevent the display from sleeping
    -i             Prevent idle system sleep (the default when no flags are given)
    -m             Prevent the disk from idling
    -s             Prevent system sleep (honoured on AC power only)
    -u             Declare the user active: turns the display on and resets the
                   idle timer. Alone it lasts 5 seconds, like caffeinate; with
                   -t, -w or a utility it is re-declared every 30 seconds.
    -t TIMEOUT     Release after TIMEOUT: plain seconds, or 90s / 5m / 1h30m.
                   Ignored when a utility is given (caffeinate does the same).
    -w PID         Release when process PID exits
    --backend B    iokit (default): hold assertions in this process
                   caffeinate: spawn /usr/bin/caffeinate instead (for comparison)
    -h, --help     Show this help
    -V, --version  Show the version

COMMANDS:
    status [PID]   List power assertions held by every process (or just PID)

With a utility, bluemeth runs it and holds the assertions until it exits,
then exits with the utility's status.";

/// Which mechanism holds the assertions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Backend {
    /// In-process IOKit assertions.
    #[default]
    IoKit,
    /// Spawn `/usr/bin/caffeinate`.
    Caffeinate,
}

/// What the user asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Hold assertions (the default).
    Hold,
    /// Print the assertion table.
    Status {
        /// Only this pid, if given.
        pid: Option<i32>,
    },
    /// Print usage.
    Help,
    /// Print the version.
    Version,
}

/// A fully parsed command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    /// What to hold.
    pub config: Config,
    /// `-w PID`.
    pub wait_pid: Option<i32>,
    /// `--backend`.
    pub backend: Backend,
    /// Utility and its arguments, if any.
    pub utility: Vec<OsString>,
    /// Subcommand / flags that change what we do.
    pub mode: Mode,
}

/// Parse error with a message suitable for stderr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseError {}

impl From<lexopt::Error> for ParseError {
    fn from(err: lexopt::Error) -> Self {
        Self(err.to_string())
    }
}

/// `caffeinate -u` without `-t` holds for this long.
pub const DEFAULT_USER_ACTIVE_TIMEOUT: Duration = Duration::from_secs(5);

/// Parse arguments (without the program name).
pub fn parse<I>(args: I) -> Result<Invocation, ParseError>
where
    I: IntoIterator,
    I::Item: Into<OsString>,
{
    use lexopt::prelude::*;

    let mut parser = lexopt::Parser::from_args(args);
    let mut config = Config::default();
    let mut saw_kind_flag = false;
    let mut wait_pid = None;
    let mut backend = Backend::default();
    let mut utility: Vec<OsString> = Vec::new();
    let mut mode = Mode::Hold;

    while let Some(arg) = parser.next()? {
        match arg {
            Short('d') => {
                config.prevent_display_sleep = true;
                saw_kind_flag = true;
            }
            Short('i') => {
                config.prevent_idle_sleep = true;
                saw_kind_flag = true;
            }
            Short('m') => {
                config.prevent_disk_idle = true;
                saw_kind_flag = true;
            }
            Short('s') => {
                config.prevent_system_sleep = true;
                saw_kind_flag = true;
            }
            Short('u') => {
                config.declare_user_active = true;
                saw_kind_flag = true;
            }
            Short('t') | Long("timeout") => {
                let raw = parser.value()?.string()?;
                let timeout = parse_duration(&raw).map_err(|e| ParseError(e.to_string()))?;
                config.timeout = Some(timeout);
            }
            Short('w') | Long("wait") => {
                let raw = parser.value()?.string()?;
                let pid: i32 = raw
                    .parse()
                    .map_err(|_| ParseError(format!("invalid pid {raw:?}")))?;
                if pid <= 0 {
                    return Err(ParseError(format!("invalid pid {pid}")));
                }
                wait_pid = Some(pid);
            }
            Long("backend") => {
                let raw = parser.value()?.string()?;
                backend = match raw.as_str() {
                    "iokit" => Backend::IoKit,
                    "caffeinate" => Backend::Caffeinate,
                    other => {
                        return Err(ParseError(format!(
                            "unknown backend {other:?} (expected iokit or caffeinate)"
                        )));
                    }
                };
            }
            Short('h') | Long("help") => mode = Mode::Help,
            Short('V') | Long("version") => mode = Mode::Version,
            Value(first) => {
                if first == "status" && utility.is_empty() {
                    let mut pid = None;
                    for rest in parser.raw_args()? {
                        let raw = rest.to_string_lossy().into_owned();
                        if pid.is_some() {
                            return Err(ParseError(format!("unexpected argument {raw:?}")));
                        }
                        pid = Some(
                            raw.parse::<i32>()
                                .map_err(|_| ParseError(format!("invalid pid {raw:?}")))?,
                        );
                    }
                    mode = Mode::Status { pid };
                } else {
                    utility.push(first);
                    utility.extend(parser.raw_args()?);
                }
            }
            _ => return Err(arg.unexpected().into()),
        }
    }

    if !saw_kind_flag {
        config.prevent_idle_sleep = true;
    }
    if config.declare_user_active
        && config.timeout.is_none()
        && wait_pid.is_none()
        && utility.is_empty()
    {
        config.timeout = Some(DEFAULT_USER_ACTIVE_TIMEOUT);
    }

    Ok(Invocation {
        config,
        wait_pid,
        backend,
        utility,
        mode,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bluemeth_core::AssertionKind;

    fn parse_strs(args: &[&str]) -> Result<Invocation, ParseError> {
        parse(args.iter().map(|s| OsString::from(*s)))
    }

    #[test]
    fn no_flags_means_idle_only() {
        let inv = parse_strs(&[]).unwrap();
        assert_eq!(inv.config, Config::idle_only());
        assert_eq!(inv.mode, Mode::Hold);
        assert_eq!(inv.backend, Backend::IoKit);
    }

    #[test]
    fn combined_short_flags_and_timeout() {
        let inv = parse_strs(&["-disu", "-t", "10"]).unwrap();
        assert!(inv.config.prevent_display_sleep);
        assert!(inv.config.prevent_idle_sleep);
        assert!(inv.config.prevent_system_sleep);
        assert!(inv.config.declare_user_active);
        assert!(!inv.config.prevent_disk_idle);
        assert_eq!(inv.config.timeout, Some(Duration::from_secs(10)));
    }

    #[test]
    fn explicit_flags_do_not_imply_idle() {
        let inv = parse_strs(&["-d"]).unwrap();
        assert!(inv.config.prevent_display_sleep);
        assert!(!inv.config.prevent_idle_sleep);
        let inv = parse_strs(&["-m"]).unwrap();
        assert!(inv.config.is_enabled(AssertionKind::DiskIdle));
        assert!(!inv.config.prevent_idle_sleep);
    }

    #[test]
    fn human_timeouts() {
        let inv = parse_strs(&["-t", "1h30m"]).unwrap();
        assert_eq!(inv.config.timeout, Some(Duration::from_secs(5400)));
        assert!(parse_strs(&["-t", "soon"]).is_err());
    }

    #[test]
    fn user_active_alone_gets_five_seconds() {
        assert_eq!(
            parse_strs(&["-u"]).unwrap().config.timeout,
            Some(DEFAULT_USER_ACTIVE_TIMEOUT)
        );
        assert_eq!(
            parse_strs(&["-u", "-t", "60"]).unwrap().config.timeout,
            Some(Duration::from_secs(60))
        );
        assert_eq!(parse_strs(&["-u", "-w", "1"]).unwrap().config.timeout, None);
        assert_eq!(
            parse_strs(&["-u", "sleep", "1"]).unwrap().config.timeout,
            None
        );
    }

    #[test]
    fn utility_swallows_remaining_args() {
        let inv = parse_strs(&["-i", "make", "-j8", "--keep-going", "-t"]).unwrap();
        assert_eq!(inv.utility, ["make", "-j8", "--keep-going", "-t"]);
        assert_eq!(inv.config.timeout, None);
    }

    #[test]
    fn status_subcommand() {
        assert_eq!(
            parse_strs(&["status"]).unwrap().mode,
            Mode::Status { pid: None }
        );
        assert_eq!(
            parse_strs(&["status", "123"]).unwrap().mode,
            Mode::Status { pid: Some(123) }
        );
        assert!(parse_strs(&["status", "abc"]).is_err());
        assert!(parse_strs(&["status", "1", "2"]).is_err());
    }

    #[test]
    fn wait_pid_and_backend() {
        let inv = parse_strs(&["-w", "42", "--backend", "caffeinate"]).unwrap();
        assert_eq!(inv.wait_pid, Some(42));
        assert_eq!(inv.backend, Backend::Caffeinate);
        assert!(parse_strs(&["-w", "0"]).is_err());
        assert!(parse_strs(&["-w", "x"]).is_err());
        assert!(parse_strs(&["--backend", "magic"]).is_err());
    }

    #[test]
    fn help_and_version_and_unknown() {
        assert_eq!(parse_strs(&["-h"]).unwrap().mode, Mode::Help);
        assert_eq!(parse_strs(&["--version"]).unwrap().mode, Mode::Version);
        assert!(parse_strs(&["-x"]).is_err());
        assert!(parse_strs(&["--frobnicate"]).is_err());
    }
}
