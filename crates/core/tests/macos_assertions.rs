//! End-to-end checks against the real `powerd`: every assertion we think we
//! hold must appear in the system table, and must vanish when released.
#![cfg(target_os = "macos")]

use std::sync::{Mutex, MutexGuard};
use std::thread::sleep;
use std::time::{Duration, Instant};

use bluemeth_core::macos::caffeinate::CaffeinateProcess;
use bluemeth_core::macos::inspect::{self, AssertionInfo};
use bluemeth_core::macos::rusage::{MallocStats, ProcessUsage};
use bluemeth_core::{AssertionKind, Config, Inhibitor};

/// Tests inspect process-wide state, so they must not interleave.
fn serial() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn own(name: &str) -> Vec<AssertionInfo> {
    inspect::own_assertions()
        .expect("IOPMCopyAssertionsByProcess")
        .into_iter()
        .filter(|row| row.name == name)
        .collect()
}

/// `assert_eq!` against this (rather than `assert!(..is_empty())`) prints the
/// offending rows when a test fails.
fn no_rows() -> Vec<AssertionInfo> {
    Vec::new()
}

fn kinds_of(rows: &[AssertionInfo]) -> Vec<AssertionKind> {
    let mut kinds: Vec<_> = rows.iter().filter_map(AssertionInfo::kind_enum).collect();
    kinds.sort();
    kinds
}

fn wait_until(timeout: Duration, mut pred: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if pred() {
            return true;
        }
        sleep(Duration::from_millis(20));
    }
    pred()
}

#[test]
fn creates_and_releases_each_kind() {
    let _guard = serial();
    let name = "bluemeth-test-each-kind";
    let mut inhibitor = Inhibitor::new(name);
    assert_eq!(own(name), no_rows(), "stale assertions from a previous run");

    for kind in AssertionKind::ALL {
        let cfg = Config::default().with(kind, true);
        inhibitor.apply(&cfg).expect("apply");
        assert!(inhibitor.is_active());
        assert_eq!(inhibitor.current(), cfg);

        let rows = own(name);
        assert_eq!(
            kinds_of(&rows),
            vec![kind],
            "powerd should list exactly {kind:?}"
        );
        assert_eq!(rows[0].details.as_deref(), Some("asserting until stopped"));
        assert_eq!(rows[0].timeout_secs, None);

        inhibitor.release_all().expect("release_all");
        assert!(!inhibitor.is_active());
        assert!(
            own(name).is_empty(),
            "{kind:?} should be gone after release"
        );
    }
}

#[test]
fn apply_is_a_diff_not_a_rebuild() {
    let _guard = serial();
    let name = "bluemeth-test-diff";
    let mut inhibitor = Inhibitor::new(name);

    let cfg = Config::default()
        .with(AssertionKind::DisplaySleep, true)
        .with(AssertionKind::IdleSystemSleep, true);
    inhibitor.apply(&cfg).unwrap();
    assert_eq!(
        kinds_of(&own(name)),
        vec![AssertionKind::DisplaySleep, AssertionKind::IdleSystemSleep]
    );

    // Turn one off, one on: exactly the right rows should change.
    let cfg = cfg
        .with(AssertionKind::DisplaySleep, false)
        .with(AssertionKind::SystemSleep, true);
    inhibitor.apply(&cfg).unwrap();
    assert_eq!(
        kinds_of(&own(name)),
        vec![AssertionKind::IdleSystemSleep, AssertionKind::SystemSleep]
    );
    assert_eq!(inhibitor.current(), cfg);

    // Applying the same config again is a no-op.
    inhibitor.apply(&cfg).unwrap();
    assert_eq!(own(name).len(), 2);

    inhibitor.apply(&Config::default()).unwrap();
    assert_eq!(own(name), no_rows());
    assert!(!inhibitor.is_active());
}

#[test]
fn timeout_is_attached_and_powerd_releases_on_its_own() {
    let _guard = serial();
    let name = "bluemeth-test-timeout";
    let mut inhibitor = Inhibitor::new(name);
    let cfg = Config::idle_only().with_timeout(Some(Duration::from_secs(1)));
    inhibitor.apply(&cfg).unwrap();

    let rows = own(name);
    assert_eq!(rows.len(), 1);
    let t = rows[0].timeout_secs.expect("timeout attached");
    assert!((0.5..=1.0).contains(&t), "timeout {t} should be ~1s");
    assert_eq!(rows[0].details.as_deref(), Some("asserting for 1 secs"));
    assert!(
        inhibitor
            .remaining()
            .is_some_and(|r| r <= Duration::from_secs(1))
    );
    assert!(!inhibitor.poll_expired().unwrap());

    // powerd drops it without any help from us.
    assert!(
        wait_until(Duration::from_secs(3), || own(name).is_empty()),
        "powerd should release the assertion after its timeout"
    );
    // Our bookkeeping catches up, and releasing an already-released id is not an error.
    assert!(wait_until(Duration::from_millis(500), || inhibitor
        .remaining()
        == Some(Duration::ZERO)));
    assert!(inhibitor.poll_expired().unwrap());
    assert!(!inhibitor.is_active());
}

#[test]
fn changing_timeout_recreates_with_new_deadline() {
    let _guard = serial();
    let name = "bluemeth-test-retimeout";
    let mut inhibitor = Inhibitor::new(name);
    let cfg = Config::idle_only().with_timeout(Some(Duration::from_secs(600)));
    inhibitor.apply(&cfg).unwrap();
    let before = own(name);
    assert!(before[0].timeout_secs.is_some_and(|t| t > 500.0));

    let cfg = cfg.with_timeout(Some(Duration::from_secs(60)));
    inhibitor.apply(&cfg).unwrap();
    let after = own(name);
    assert_eq!(after.len(), 1);
    assert!(after[0].timeout_secs.is_some_and(|t| t <= 60.0 && t > 50.0));
    assert!(
        inhibitor
            .remaining()
            .is_some_and(|r| r <= Duration::from_secs(60))
    );

    // Adding a kind under an existing deadline inherits the *remaining* time.
    let cfg = cfg.with(AssertionKind::DisplaySleep, true);
    inhibitor.apply(&cfg).unwrap();
    let rows = own(name);
    assert_eq!(rows.len(), 2);
    for row in &rows {
        assert!(row.timeout_secs.is_some_and(|t| t <= 60.0));
    }

    inhibitor.restart(&cfg).unwrap();
    assert!(
        inhibitor
            .remaining()
            .is_some_and(|r| r > Duration::from_secs(59))
    );
    inhibitor.release_all().unwrap();
    assert_eq!(own(name), no_rows());
}

#[test]
fn user_activity_declares_and_refreshes() {
    let _guard = serial();
    let name = "bluemeth-test-user-active";
    let mut inhibitor = Inhibitor::new(name);
    let cfg = Config {
        declare_user_active: true,
        ..Config::default()
    };
    inhibitor.apply(&cfg).unwrap();
    assert!(inhibitor.is_active());
    assert!(inhibitor.current().declare_user_active);
    assert!(inhibitor.next_user_activity_due().is_some());

    let rows = own(name);
    assert!(
        rows.iter().any(AssertionInfo::is_user_active),
        "expected a UserIsActive row, got {rows:?}"
    );

    // Refreshing before the interval elapses is a no-op; both paths succeed.
    inhibitor.refresh_user_activity().unwrap();
    inhibitor.release_all().unwrap();
    assert_eq!(own(name), no_rows());
    assert!(inhibitor.next_user_activity_due().is_none());
}

#[test]
fn drop_releases_everything() {
    let _guard = serial();
    let name = "bluemeth-test-drop";
    {
        let mut inhibitor = Inhibitor::new(name);
        inhibitor.apply(&Config::all()).unwrap();
        assert_eq!(own(name).len(), 4, "3 assertions + UserIsActive");
    }
    assert_eq!(own(name), no_rows(), "drop must release all assertions");
}

/// A leak of a few bytes per cycle must fail this test.
///
/// Live heap bytes (`malloc_zone_statistics`) are compared before and after
/// 10 000 cycles. The physical footprint is reported too but not asserted
/// on: it moves in 16 KiB pages and steps up once, at a variable moment in
/// the first seconds of churn, which made a footprint-based assertion flaky.
/// A 48-byte-per-cycle leak (the bug this test was written for) adds 480 KiB
/// of live heap here; the threshold is 16 KiB.
#[test]
fn repeated_cycles_do_not_grow_memory() {
    let _guard = serial();
    let name = "bluemeth-test-cycles";
    let mut inhibitor = Inhibitor::new(name);
    let cfg = Config::all(); // 3 assertions + UserIsActive per cycle

    let cycles = |inhibitor: &mut Inhibitor, n: usize| {
        for _ in 0..n {
            inhibitor.apply(&cfg).unwrap();
            inhibitor.release_all().unwrap();
        }
    };

    // Apple's frameworks free some per-call bookkeeping asynchronously on
    // their own dispatch queues; a short pause lets that backlog drain so it
    // is not mistaken for growth.
    let settle = || sleep(Duration::from_millis(300));
    let sample = || {
        settle();
        (MallocStats::sample(), ProcessUsage::sample_self().unwrap())
    };

    cycles(&mut inhibitor, 1000);
    let (heap_a, usage_a) = sample();
    cycles(&mut inhibitor, 10_000);
    let (heap_b, usage_b) = sample();
    cycles(&mut inhibitor, 10_000);
    let (heap_c, usage_c) = sample();
    assert_eq!(own(name), no_rows());

    let report = |label: &str,
                  h0: &MallocStats,
                  h1: &MallocStats,
                  u0: &ProcessUsage,
                  u1: &ProcessUsage| {
        let heap = h1.size_in_use as i64 - h0.size_in_use as i64;
        let blocks = i64::from(h1.blocks_in_use) - i64::from(h0.blocks_in_use);
        let footprint = u1.phys_footprint as i64 - u0.phys_footprint as i64;
        eprintln!(
            "{label}: live heap {heap:+} bytes ({blocks:+} blocks), footprint {footprint:+} bytes"
        );
        heap
    };
    let first = report("cycles 1000..11000", &heap_a, &heap_b, &usage_a, &usage_b);
    let second = report("cycles 11000..21000", &heap_b, &heap_c, &usage_b, &usage_c);
    assert!(
        second < 16 * 1024,
        "live heap grew {second} bytes over 10000 cycles (and {first} over the previous 10000): a per-cycle leak"
    );
}

#[test]
fn caffeinate_subprocess_backend_round_trips() {
    let _guard = serial();
    let cfg = Config::idle_only().with(AssertionKind::DisplaySleep, true);
    let proc_ = CaffeinateProcess::spawn(&cfg).expect("spawn caffeinate");
    let pid = proc_.pid();
    assert!(
        wait_until(Duration::from_secs(3), || {
            inspect::assertions_for_pid(pid).unwrap().len() == 2
        }),
        "caffeinate should register 2 assertions"
    );
    proc_.stop().unwrap();
    assert!(
        wait_until(Duration::from_secs(3), || {
            inspect::assertions_for_pid(pid).unwrap().is_empty()
        }),
        "powerd should drop caffeinate's assertions after it exits"
    );
}
