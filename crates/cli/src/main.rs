//! `bluemeth`: a `caffeinate`-compatible command line keep-awake tool that
//! holds IOKit power assertions in-process instead of spawning anything.

#![forbid(unsafe_op_in_unsafe_fn)]

mod args;
#[cfg(target_os = "macos")]
mod hold;
#[cfg(target_os = "macos")]
mod status;
#[cfg(target_os = "macos")]
mod wait;

use std::process::ExitCode;

use args::Mode;

/// `sysexits.h` EX_USAGE: the command was used incorrectly.
const EX_USAGE: u8 = 64;

fn main() -> ExitCode {
    let invocation = match args::parse(std::env::args_os().skip(1)) {
        Ok(invocation) => invocation,
        Err(err) => {
            eprintln!("bluemeth: {err}");
            eprintln!("{}", args::USAGE);
            return ExitCode::from(EX_USAGE);
        }
    };

    match invocation.mode {
        Mode::Help => {
            println!("{}", args::USAGE);
            ExitCode::SUCCESS
        }
        Mode::Version => {
            println!("bluemeth {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        #[cfg(target_os = "macos")]
        Mode::Status { pid } => match status::print(pid) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("bluemeth: {err}");
                ExitCode::FAILURE
            }
        },
        #[cfg(target_os = "macos")]
        Mode::Hold => match hold::run(&invocation) {
            Ok(code) => code,
            Err(err) => {
                eprintln!("bluemeth: {err}");
                ExitCode::FAILURE
            }
        },
        #[cfg(not(target_os = "macos"))]
        Mode::Status { .. } | Mode::Hold => {
            eprintln!("bluemeth: {}", bluemeth_core::Error::Unsupported);
            ExitCode::FAILURE
        }
    }
}
