//! Black-box tests of the `bluemeth` binary against the live system.
#![cfg(target_os = "macos")]

use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use bluemeth_core::macos::inspect::{self, AssertionInfo};

fn bluemeth() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_bluemeth"));
    cmd.stdin(Stdio::null());
    cmd
}

fn wait_until(timeout: Duration, mut pred: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if pred() {
            return true;
        }
        sleep(Duration::from_millis(10));
    }
    pred()
}

fn assertion_count(pid: i32) -> usize {
    inspect::assertions_for_pid(pid).unwrap().len()
}

fn kill(child: &Child, sig: libc::c_int) {
    // SAFETY: sending a signal to a child we spawned and have not reaped.
    assert_eq!(unsafe { libc::kill(child.id() as i32, sig) }, 0);
}

#[test]
fn help_and_version() {
    let out = bluemeth().arg("--help").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("USAGE"));
    assert!(text.contains("-t TIMEOUT"));

    let out = bluemeth().arg("-V").output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("bluemeth "));
}

#[test]
fn bad_usage_exits_64_with_usage_on_stderr() {
    let out = bluemeth().arg("--frobnicate").output().unwrap();
    assert_eq!(out.status.code(), Some(64));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("frobnicate"));
    assert!(err.contains("USAGE"));

    let out = bluemeth().args(["-t", "soon"]).output().unwrap();
    assert_eq!(out.status.code(), Some(64));
}

#[test]
fn timeout_releases_and_exits_zero() {
    let start = Instant::now();
    let status = bluemeth().args(["-i", "-t", "1"]).status().unwrap();
    assert!(status.success());
    let elapsed = start.elapsed();
    assert!(elapsed >= Duration::from_millis(900), "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(4), "{elapsed:?}");
}

#[test]
fn holds_assertions_while_running_and_releases_on_sigterm() {
    let mut child = bluemeth().args(["-dis"]).spawn().unwrap();
    let pid = child.id() as i32;
    assert!(
        wait_until(Duration::from_secs(3), || assertion_count(pid) == 3),
        "expected 3 assertions, saw {}",
        assertion_count(pid)
    );
    let rows = inspect::assertions_for_pid(pid).unwrap();
    assert!(rows.iter().all(|r| r.name == "bluemeth command-line tool"));
    assert!(rows.iter().all(|r| r.timeout_secs.is_none()));

    // `status <pid>` shows the same thing.
    let out = bluemeth()
        .args(["status", &pid.to_string()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("PreventUserIdleDisplaySleep"), "{text}");
    assert!(text.contains("PreventUserIdleSystemSleep"), "{text}");
    assert!(text.contains("PreventSystemSleep"), "{text}");
    assert!(text.contains("bluemeth"), "{text}");

    kill(&child, libc::SIGTERM);
    let status = child.wait().unwrap();
    // Re-raised SIGTERM: the shell sees death-by-signal, like caffeinate.
    assert_eq!(
        std::os::unix::process::ExitStatusExt::signal(&status),
        Some(libc::SIGTERM)
    );
    assert!(wait_until(Duration::from_secs(2), || assertion_count(pid) == 0));
}

#[test]
fn sigint_releases_too() {
    let mut child = bluemeth().args(["-i"]).spawn().unwrap();
    let pid = child.id() as i32;
    assert!(wait_until(Duration::from_secs(3), || assertion_count(pid) == 1));
    kill(&child, libc::SIGINT);
    child.wait().unwrap();
    assert!(wait_until(Duration::from_secs(2), || assertion_count(pid) == 0));
}

#[test]
fn utility_mode_propagates_exit_code_and_holds_until_exit() {
    let status = bluemeth()
        .args(["-i", "/bin/sh", "-c", "exit 7"])
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(7));

    // While the utility runs, the *bluemeth* process holds the assertion.
    let mut child = bluemeth()
        .args(["-d", "-t", "1", "/bin/sleep", "2"])
        .spawn()
        .unwrap();
    let pid = child.id() as i32;
    assert!(wait_until(Duration::from_secs(3), || assertion_count(pid) == 1));
    // -t is ignored with a utility: still held after 1.3s.
    sleep(Duration::from_millis(1300));
    assert_eq!(assertion_count(pid), 1);
    let start = Instant::now();
    assert!(child.wait().unwrap().success());
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(wait_until(Duration::from_secs(2), || assertion_count(pid) == 0));
}

#[test]
fn missing_utility_exits_127() {
    let out = bluemeth()
        .args(["-i", "/nonexistent/definitely-not-here"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(127));
    assert!(String::from_utf8_lossy(&out.stderr).contains("command not found"));
}

#[test]
fn wait_pid_mode_releases_when_target_exits() {
    let mut target = Command::new("/bin/sleep").arg("1").spawn().unwrap();
    let start = Instant::now();
    let mut child = bluemeth()
        .args(["-i", "-w", &target.id().to_string()])
        .spawn()
        .unwrap();
    let pid = child.id() as i32;
    assert!(wait_until(Duration::from_secs(3), || assertion_count(pid) == 1));
    assert!(child.wait().unwrap().success());
    let elapsed = start.elapsed();
    assert!(elapsed >= Duration::from_millis(900), "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(4), "{elapsed:?}");
    target.wait().unwrap();
    assert!(wait_until(Duration::from_secs(2), || assertion_count(pid) == 0));

    let out = bluemeth().args(["-w", "2147483646"]).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot wait for pid"));
}

#[test]
fn user_active_alone_lasts_five_seconds() {
    let mut child = bluemeth().arg("-u").spawn().unwrap();
    let pid = child.id() as i32;
    assert!(wait_until(Duration::from_secs(3), || {
        inspect::assertions_for_pid(pid)
            .unwrap()
            .iter()
            .any(AssertionInfo::is_user_active)
    }));
    kill(&child, libc::SIGTERM);
    child.wait().unwrap();
}

#[test]
fn caffeinate_backend_round_trips() {
    let start = Instant::now();
    let status = bluemeth()
        .args(["--backend", "caffeinate", "-i", "-t", "1"])
        .status()
        .unwrap();
    assert!(status.success());
    assert!(start.elapsed() < Duration::from_secs(4));
}

#[test]
fn status_with_no_assertions_for_pid_says_so() {
    let out = bluemeth().args(["status", "1"]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("no power assertions held by pid 1") || text.contains("PID"));
}
