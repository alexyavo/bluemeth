//! A kqueue-based waiter: blocks in a single syscall until a signal arrives,
//! a watched process exits, or a deadline passes. No polling, no threads, no
//! wake-ups while idle.

use std::io;
use std::os::fd::RawFd;
use std::ptr;
use std::time::Duration;

/// Why [`Waiter::wait`] returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// A watched signal was delivered.
    Signal(libc::c_int),
    /// A watched process exited.
    ProcessExit(i32),
    /// The requested timeout elapsed.
    Timeout,
}

/// Owns a kqueue descriptor.
#[derive(Debug)]
pub struct Waiter {
    kq: RawFd,
}

impl Waiter {
    /// Create an empty kqueue.
    pub fn new() -> io::Result<Self> {
        // SAFETY: kqueue() takes no arguments and returns a new fd or -1.
        let kq = unsafe { libc::kqueue() };
        if kq < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `kq` is a valid fd we just created; FD_CLOEXEC keeps it out
        // of any utility we spawn.
        if unsafe { libc::fcntl(kq, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            let err = io::Error::last_os_error();
            // SAFETY: closing the fd we own; nothing else references it yet.
            unsafe { libc::close(kq) };
            return Err(err);
        }
        Ok(Self { kq })
    }

    /// Deliver `sig` through the kqueue instead of its default action.
    ///
    /// The disposition is set to `SIG_IGN` so the process is not killed;
    /// children spawned *after* this call inherit that, so spawn first.
    pub fn watch_signal(&self, sig: libc::c_int) -> io::Result<()> {
        // SAFETY: setting a signal disposition for our own process to the
        // well-defined SIG_IGN constant.
        if unsafe { libc::signal(sig, libc::SIG_IGN) } == libc::SIG_ERR {
            return Err(io::Error::last_os_error());
        }
        self.add(libc::kevent {
            ident: sig as libc::uintptr_t,
            filter: libc::EVFILT_SIGNAL,
            flags: libc::EV_ADD | libc::EV_ENABLE,
            fflags: 0,
            data: 0,
            udata: ptr::null_mut(),
        })
    }

    /// Fire once when `pid` exits. Fails with `ESRCH` if it does not exist.
    pub fn watch_process_exit(&self, pid: i32) -> io::Result<()> {
        self.add(libc::kevent {
            ident: pid as libc::uintptr_t,
            filter: libc::EVFILT_PROC,
            flags: libc::EV_ADD | libc::EV_ENABLE | libc::EV_ONESHOT,
            fflags: libc::NOTE_EXIT,
            data: 0,
            udata: ptr::null_mut(),
        })
    }

    fn add(&self, change: libc::kevent) -> io::Result<()> {
        let mut result = libc::kevent {
            ident: 0,
            filter: 0,
            flags: 0,
            fflags: 0,
            data: 0,
            udata: ptr::null_mut(),
        };
        // SAFETY: `change` is a fully initialised kevent; `result` has room
        // for the one event kqueue may report back (EV_ERROR); the timeout is
        // zero so this never blocks.
        let n = unsafe {
            libc::kevent(
                self.kq,
                &raw const change,
                1,
                &raw mut result,
                1,
                &libc::timespec {
                    tv_sec: 0,
                    tv_nsec: 0,
                },
            )
        };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        if n > 0 && result.flags & libc::EV_ERROR != 0 && result.data != 0 {
            return Err(io::Error::from_raw_os_error(result.data as i32));
        }
        Ok(())
    }

    /// Block until something happens, or `timeout` elapses (`None` = forever).
    pub fn wait(&self, timeout: Option<Duration>) -> io::Result<Event> {
        let ts = timeout.map(|t| libc::timespec {
            tv_sec: t.as_secs().min(libc::time_t::MAX as u64) as libc::time_t,
            tv_nsec: libc::c_long::from(t.subsec_nanos()),
        });
        let ts_ptr = ts
            .as_ref()
            .map_or(ptr::null(), ptr::from_ref::<libc::timespec>);
        let mut event = libc::kevent {
            ident: 0,
            filter: 0,
            flags: 0,
            fflags: 0,
            data: 0,
            udata: ptr::null_mut(),
        };
        loop {
            // SAFETY: no changes are submitted; `event` has room for one
            // result; `ts_ptr` is either null (block) or points at a timespec
            // that outlives the call.
            let n = unsafe { libc::kevent(self.kq, ptr::null(), 0, &raw mut event, 1, ts_ptr) };
            if n < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(err);
            }
            if n == 0 {
                return Ok(Event::Timeout);
            }
            return Ok(match event.filter {
                libc::EVFILT_SIGNAL => Event::Signal(event.ident as libc::c_int),
                libc::EVFILT_PROC => Event::ProcessExit(event.ident as i32),
                _ => continue,
            });
        }
    }
}

impl Drop for Waiter {
    fn drop(&mut self) {
        // SAFETY: closing the descriptor this struct owns, exactly once.
        unsafe { libc::close(self.kq) };
    }
}

/// Restore `sig`'s default disposition and re-raise it, so the shell sees the
/// process die from the signal it was sent. Returns only if that failed.
pub fn reraise(sig: libc::c_int) {
    // SAFETY: restoring the default disposition and raising a signal in our
    // own process; both are always valid for the signals we watch.
    unsafe {
        libc::signal(sig, libc::SIG_DFL);
        libc::raise(sig);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::time::Instant;

    #[test]
    fn timeout_fires() {
        let waiter = Waiter::new().unwrap();
        let start = Instant::now();
        assert_eq!(
            waiter.wait(Some(Duration::from_millis(50))).unwrap(),
            Event::Timeout
        );
        assert!(start.elapsed() >= Duration::from_millis(45));
    }

    #[test]
    fn process_exit_fires() {
        let waiter = Waiter::new().unwrap();
        let mut child = Command::new("/bin/sleep").arg("0.1").spawn().unwrap();
        let pid = child.id() as i32;
        waiter.watch_process_exit(pid).unwrap();
        assert_eq!(
            waiter.wait(Some(Duration::from_secs(5))).unwrap(),
            Event::ProcessExit(pid)
        );
        child.wait().unwrap();
    }

    #[test]
    fn missing_process_is_an_error() {
        let waiter = Waiter::new().unwrap();
        let err = waiter.watch_process_exit(i32::MAX - 1).unwrap_err();
        assert_eq!(err.raw_os_error(), Some(libc::ESRCH));
    }
}
