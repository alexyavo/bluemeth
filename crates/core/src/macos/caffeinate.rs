//! The "spawn `/usr/bin/caffeinate`" mechanism, kept as a reference
//! implementation so the in-process backend can be benchmarked against it.

use std::io;
use std::process::{Child, Command, Stdio};

use crate::Config;

/// Default location of Apple's tool.
pub const CAFFEINATE_PATH: &str = "/usr/bin/caffeinate";

/// A running `caffeinate` child process holding assertions on our behalf.
///
/// Dropping it kills the child; `powerd` releases the child's assertions when
/// the process disappears.
#[derive(Debug)]
pub struct CaffeinateProcess {
    child: Child,
}

impl CaffeinateProcess {
    /// Spawn `caffeinate` with the flags equivalent to `config`.
    pub fn spawn(config: &Config) -> io::Result<Self> {
        Self::spawn_at(CAFFEINATE_PATH, config)
    }

    /// Spawn a caffeinate-compatible binary at `path`.
    pub fn spawn_at(path: &str, config: &Config) -> io::Result<Self> {
        let child = Command::new(path)
            .args(config.caffeinate_args())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        Ok(Self { child })
    }

    /// The child's pid.
    pub fn pid(&self) -> i32 {
        self.child.id() as i32
    }

    /// Terminate the child and reap it.
    pub fn stop(mut self) -> io::Result<()> {
        let result = self.terminate_and_wait();
        // Prevent Drop from trying again on an already reaped child.
        std::mem::forget(self);
        result
    }

    fn terminate_and_wait(&mut self) -> io::Result<()> {
        // `Child::kill` is SIGKILL; caffeinate has no cleanup to do and powerd
        // drops its assertions either way, so the stronger signal is fine.
        match self.child.kill() {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::InvalidInput => {} // already exited
            Err(e) => return Err(e),
        }
        self.child.wait().map(drop)
    }
}

impl Drop for CaffeinateProcess {
    fn drop(&mut self) {
        // Errors cannot be reported from Drop; use `stop()` to observe them.
        let _ = self.terminate_and_wait();
    }
}
