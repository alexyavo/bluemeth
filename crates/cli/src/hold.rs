//! Hold assertions until a deadline, a signal, or a watched process exit.

use std::error::Error;
use std::io;
use std::os::unix::process::ExitStatusExt;
use std::process::{Child, Command, ExitCode, ExitStatus};

use bluemeth_core::macos::caffeinate::CaffeinateProcess;
use bluemeth_core::{Config, Inhibitor, Monotonic, now};

use crate::args::{Backend, Invocation};
use crate::wait::{self, Event, Waiter};

/// Shell convention for "command not found".
const EX_NOT_FOUND: u8 = 127;

enum Holder {
    IoKit(Inhibitor),
    Caffeinate(CaffeinateProcess),
}

impl Holder {
    fn start(backend: Backend, config: &Config) -> Result<Self, Box<dyn Error>> {
        match backend {
            Backend::IoKit => {
                let mut inhibitor = Inhibitor::new("bluemeth command-line tool");
                inhibitor.apply(config)?;
                Ok(Self::IoKit(inhibitor))
            }
            Backend::Caffeinate => Ok(Self::Caffeinate(CaffeinateProcess::spawn(config)?)),
        }
    }

    fn next_user_activity_due(&self) -> Option<Monotonic> {
        match self {
            Self::IoKit(inhibitor) => inhibitor.next_user_activity_due(),
            Self::Caffeinate(_) => None,
        }
    }

    fn refresh(&mut self) -> Result<(), Box<dyn Error>> {
        if let Self::IoKit(inhibitor) = self {
            inhibitor.refresh_user_activity()?;
        }
        Ok(())
    }

    fn stop(self) -> Result<(), Box<dyn Error>> {
        match self {
            Self::IoKit(mut inhibitor) => inhibitor.release_all()?,
            Self::Caffeinate(process) => process.stop()?,
        }
        Ok(())
    }
}

/// Run the hold loop and return the process exit code.
pub fn run(invocation: &Invocation) -> Result<ExitCode, Box<dyn Error>> {
    // Spawn the utility before touching signal dispositions so it inherits
    // the defaults, exactly like running it from a shell.
    let mut child = match spawn_utility(invocation) {
        Ok(child) => child,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            eprintln!(
                "bluemeth: {}: command not found",
                invocation.utility[0].to_string_lossy()
            );
            return Ok(ExitCode::from(EX_NOT_FOUND));
        }
        Err(err) => {
            return Err(format!(
                "cannot run {}: {err}",
                invocation.utility[0].to_string_lossy()
            )
            .into());
        }
    };

    let mut config = invocation.config;
    if child.is_some() {
        // caffeinate(8): "Timeout value is not used when an utility is invoked".
        config.timeout = None;
    }

    let waiter = Waiter::new()?;
    if let Some(child) = &child {
        waiter.watch_process_exit(child.id() as i32)?;
    } else {
        for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
            waiter.watch_signal(sig)?;
        }
    }
    if let Some(pid) = invocation.wait_pid {
        waiter
            .watch_process_exit(pid)
            .map_err(|err| format!("cannot wait for pid {pid}: {err}"))?;
    }

    let mut holder = Holder::start(invocation.backend, &config)?;
    let deadline = config.timeout.map(|t| now() + t);
    let mut received_signal = None;

    loop {
        let current = now();
        if deadline.is_some_and(|d| current >= d) {
            break;
        }
        let next = [deadline, holder.next_user_activity_due()]
            .into_iter()
            .flatten()
            .min();
        match waiter.wait(next.map(|n| n.remaining_from(current)))? {
            Event::Timeout => holder.refresh()?,
            Event::Signal(sig) => {
                received_signal = Some(sig);
                break;
            }
            Event::ProcessExit(_) => break,
        }
    }

    holder.stop()?;

    if let Some(child) = child.as_mut() {
        return Ok(exit_code_of(child.wait()?));
    }
    if let Some(sig) = received_signal {
        wait::reraise(sig);
        // Only reached if the re-raised signal did not terminate us.
        return Ok(ExitCode::from(128u8.wrapping_add(sig as u8)));
    }
    Ok(ExitCode::SUCCESS)
}

fn spawn_utility(invocation: &Invocation) -> io::Result<Option<Child>> {
    let Some((program, args)) = invocation.utility.split_first() else {
        return Ok(None);
    };
    Command::new(program).args(args).spawn().map(Some)
}

fn exit_code_of(status: ExitStatus) -> ExitCode {
    if let Some(code) = status.code() {
        return ExitCode::from(code.clamp(0, 255) as u8);
    }
    match status.signal() {
        Some(sig) => ExitCode::from(128u8.wrapping_add(sig as u8)),
        None => ExitCode::FAILURE,
    }
}
