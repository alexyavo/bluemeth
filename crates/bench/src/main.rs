//! `bluemeth-bench`: measure what keeping a Mac awake costs, mechanism by
//! mechanism, using the kernel's own per-process accounting
//! (`proc_pid_rusage`: CPU time, idle wake-ups, memory footprint, energy).
//!
//! Three experiments:
//!
//! * `process`: run `/usr/bin/caffeinate` and the `bluemeth` CLI side by side
//!   with identical flags and compare startup latency, steady-state cost over
//!   a window, release latency, and whole-lifetime CPU.
//! * `inproc`: micro-benchmark creating/releasing assertions in-process
//!   against spawning/killing a `caffeinate` child, and check memory stays
//!   flat over thousands of cycles.
//! * `watch`: sample any running processes (e.g. menu bar apps) over a window.

#![forbid(unsafe_op_in_unsafe_fn)]
#![cfg(target_os = "macos")]

mod stats;
mod table;

use std::error::Error;
use std::io;
use std::mem::MaybeUninit;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use bluemeth_core::macos::caffeinate::{CAFFEINATE_PATH, CaffeinateProcess};
use bluemeth_core::macos::inspect;
use bluemeth_core::macos::rusage::ProcessUsage;
use bluemeth_core::{AssertionKind, Config, Inhibitor};

use stats::{Summary, fmt, pm};
use table::Table;

const USAGE: &str = "\
bluemeth-bench: compare keep-awake mechanisms by resource usage

USAGE:
    bluemeth-bench process [--trials N] [--window SECS] [--bluemeth PATH] [--caffeinate PATH]
    bluemeth-bench inproc  [--iterations N] [--spawn-iterations N] [--user-active]
    bluemeth-bench watch   --pid PID [--pid PID...] [--window SECS]
    bluemeth-bench all     (process + inproc with defaults)

Defaults: 5 trials, 10 s window, 1000 iterations, 20 spawn iterations.
The bluemeth CLI defaults to the `bluemeth` binary next to this executable.";

#[derive(Debug)]
struct Options {
    command: String,
    trials: usize,
    window: Duration,
    iterations: usize,
    spawn_iterations: usize,
    bluemeth: Option<PathBuf>,
    caffeinate: PathBuf,
    pids: Vec<i32>,
    user_active: bool,
}

fn parse_options() -> Result<Options, lexopt::Error> {
    use lexopt::prelude::*;
    let mut opts = Options {
        command: "all".to_owned(),
        trials: 5,
        window: Duration::from_secs(10),
        iterations: 1000,
        spawn_iterations: 20,
        bluemeth: None,
        caffeinate: PathBuf::from(CAFFEINATE_PATH),
        pids: Vec::new(),
        user_active: false,
    };
    let mut parser = lexopt::Parser::from_env();
    let mut saw_command = false;
    while let Some(arg) = parser.next()? {
        match arg {
            Long("trials") => opts.trials = parser.value()?.parse()?,
            Long("window") => opts.window = Duration::from_secs_f64(parser.value()?.parse()?),
            Long("iterations") => opts.iterations = parser.value()?.parse()?,
            Long("spawn-iterations") => opts.spawn_iterations = parser.value()?.parse()?,
            Long("bluemeth") => opts.bluemeth = Some(parser.value()?.into()),
            Long("caffeinate") => opts.caffeinate = parser.value()?.into(),
            Long("pid") => opts.pids.push(parser.value()?.parse()?),
            Long("user-active") => opts.user_active = true,
            Short('h') | Long("help") => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            Value(v) if !saw_command => {
                opts.command = v.string()?;
                saw_command = true;
            }
            _ => return Err(arg.unexpected()),
        }
    }
    Ok(opts)
}

fn main() {
    let opts = match parse_options() {
        Ok(opts) => opts,
        Err(err) => {
            eprintln!("bluemeth-bench: {err}\n{USAGE}");
            std::process::exit(64);
        }
    };
    let result = match opts.command.as_str() {
        "process" => bench_process(&opts),
        "inproc" => bench_inproc(&opts),
        "watch" => bench_watch(&opts),
        "all" => bench_process(&opts).and_then(|()| bench_inproc(&opts)),
        other => Err(format!("unknown command {other:?}\n{USAGE}").into()),
    };
    if let Err(err) = result {
        eprintln!("bluemeth-bench: {err}");
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// process: caffeinate vs bluemeth CLI, identical flags
// ---------------------------------------------------------------------------

struct Candidate {
    label: &'static str,
    path: PathBuf,
}

#[derive(Debug, Default)]
struct SteadyTrial {
    ready_ms: f64,
    cpu_ms_window: f64,
    idle_wakeups: f64,
    interrupt_wakeups: f64,
    energy_mj: f64,
    footprint_mb: f64,
    peak_mb: f64,
    release_ms: f64,
    lifetime_cpu_ms: f64,
    maxrss_mb: f64,
}

#[derive(Debug, Default)]
struct OneShotTrial {
    wall_ms: f64,
    cpu_ms: f64,
    maxrss_mb: f64,
}

#[allow(clippy::too_many_lines)] // one table per metric group; splitting hides the flow
fn bench_process(opts: &Options) -> Result<(), Box<dyn Error>> {
    let bluemeth = match &opts.bluemeth {
        Some(path) => path.clone(),
        None => std::env::current_exe()?
            .parent()
            .ok_or("cannot locate sibling bluemeth binary")?
            .join("bluemeth"),
    };
    if !bluemeth.exists() {
        return Err(format!(
            "bluemeth CLI not found at {}; build it or pass --bluemeth PATH",
            bluemeth.display()
        )
        .into());
    }
    let candidates = [
        Candidate {
            label: "caffeinate (Apple)",
            path: opts.caffeinate.clone(),
        },
        Candidate {
            label: "bluemeth (in-process IOKit)",
            path: bluemeth,
        },
    ];

    println!(
        "## Process comparison: `-dis` held for a {}s window, {} trials\n",
        opts.window.as_secs(),
        opts.trials
    );
    println!(
        "Both binaries are run with identical flags; counters come from `proc_pid_rusage` on the child process. Window deltas exclude startup.\n"
    );

    let mut steady = Table::new([
        "mechanism",
        "ready (ms)",
        "CPU in window (ms)",
        "idle wake-ups",
        "interrupt wake-ups",
        "energy (mJ)",
        "footprint (MB)",
        "peak footprint (MB)",
        "release (ms)",
        "lifetime CPU (ms)",
        "max RSS (MB)",
    ]);
    let mut oneshot = Table::new([
        "mechanism",
        "`-i -t 1` wall (ms)",
        "lifetime CPU (ms)",
        "max RSS (MB)",
    ]);
    let mut sizes = Table::new(["binary", "path", "size (KB)"]);

    for cand in &candidates {
        // Untimed warm-up so the first trial does not pay for cold page cache.
        steady_trial(cand, Duration::from_secs(1))?;
        oneshot_trial(cand)?;
        eprint!("{}: steady-state", cand.label);
        let mut trials = Vec::with_capacity(opts.trials);
        for _ in 0..opts.trials {
            trials.push(steady_trial(cand, opts.window)?);
            eprint!(".");
        }
        eprint!(" one-shot");
        let mut shots = Vec::with_capacity(opts.trials);
        for _ in 0..opts.trials {
            shots.push(oneshot_trial(cand)?);
            eprint!(".");
        }
        eprintln!();

        let col = |f: fn(&SteadyTrial) -> f64| {
            pm(&Summary::of(&trials.iter().map(f).collect::<Vec<_>>()))
        };
        steady.row([
            cand.label.to_owned(),
            col(|t| t.ready_ms),
            col(|t| t.cpu_ms_window),
            col(|t| t.idle_wakeups),
            col(|t| t.interrupt_wakeups),
            col(|t| t.energy_mj),
            col(|t| t.footprint_mb),
            col(|t| t.peak_mb),
            col(|t| t.release_ms),
            col(|t| t.lifetime_cpu_ms),
            col(|t| t.maxrss_mb),
        ]);
        let scol = |f: fn(&OneShotTrial) -> f64| {
            pm(&Summary::of(&shots.iter().map(f).collect::<Vec<_>>()))
        };
        oneshot.row([
            cand.label.to_owned(),
            scol(|t| t.wall_ms),
            scol(|t| t.cpu_ms),
            scol(|t| t.maxrss_mb),
        ]);
        let size_kb = std::fs::metadata(&cand.path).map_or(0, |m| m.len() / 1024);
        sizes.row([
            cand.label.to_owned(),
            cand.path.display().to_string(),
            size_kb.to_string(),
        ]);
    }

    println!("{}\n", steady.render());
    println!("### One-shot lifecycle (spawn, hold 1 s, exit)\n");
    println!("{}\n", oneshot.render());
    println!("### Binary size\n");
    println!("{}\n", sizes.render());
    Ok(())
}

fn hold_config() -> Config {
    Config::idle_only()
        .with(AssertionKind::DisplaySleep, true)
        .with(AssertionKind::SystemSleep, true)
}

fn spawn_quiet(path: &PathBuf, args: &[String]) -> io::Result<std::process::Child> {
    Command::new(path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

fn steady_trial(cand: &Candidate, window: Duration) -> Result<SteadyTrial, Box<dyn Error>> {
    let config = hold_config().with_timeout(Some(Duration::from_secs(3600)));
    let expected = config.kinds().count();

    let start = Instant::now();
    let child = spawn_quiet(&cand.path, &config.caffeinate_args())?;
    let pid = child.id() as i32;
    let ready_ms = wait_for_assertion_count(pid, expected, Duration::from_secs(5))
        .ok_or_else(|| format!("{}: assertions never appeared", cand.label))?
        .as_secs_f64()
        * 1e3;
    let _ = start;

    sleep(Duration::from_millis(300));
    let a = ProcessUsage::sample(pid)?;
    sleep(window);
    let b = ProcessUsage::sample(pid)?;
    let d = b.delta_since(&a);

    let kill_at = Instant::now();
    // SAFETY: signalling our own child, which has not been reaped yet.
    unsafe { libc::kill(pid, libc::SIGTERM) };
    let release_ms = wait_for_assertion_count(pid, 0, Duration::from_secs(5))
        .ok_or_else(|| format!("{}: assertions never released", cand.label))?
        .as_secs_f64()
        * 1e3;
    let _ = kill_at;
    let (_, ru) = reap(pid)?;
    drop(child);

    Ok(SteadyTrial {
        ready_ms,
        cpu_ms_window: d.cpu_time_ns() as f64 / 1e6,
        idle_wakeups: d.pkg_idle_wakeups as f64,
        interrupt_wakeups: d.interrupt_wakeups as f64,
        energy_mj: d.billed_energy_nj as f64 / 1e6,
        footprint_mb: b.phys_footprint as f64 / 1e6,
        peak_mb: b.lifetime_max_phys_footprint as f64 / 1e6,
        release_ms,
        lifetime_cpu_ms: rusage_cpu_ms(&ru),
        maxrss_mb: ru.ru_maxrss as f64 / 1e6,
    })
}

fn oneshot_trial(cand: &Candidate) -> Result<OneShotTrial, Box<dyn Error>> {
    let config = Config::idle_only().with_timeout(Some(Duration::from_secs(1)));
    let start = Instant::now();
    let child = spawn_quiet(&cand.path, &config.caffeinate_args())?;
    let pid = child.id() as i32;
    let (_, ru) = reap(pid)?;
    let wall_ms = start.elapsed().as_secs_f64() * 1e3;
    drop(child);
    Ok(OneShotTrial {
        wall_ms,
        cpu_ms: rusage_cpu_ms(&ru),
        maxrss_mb: ru.ru_maxrss as f64 / 1e6,
    })
}

/// Poll the assertion table until `pid` holds exactly `count` assertions.
fn wait_for_assertion_count(pid: i32, count: usize, timeout: Duration) -> Option<Duration> {
    let start = Instant::now();
    loop {
        if inspect::assertions_for_pid(pid).ok()?.len() == count {
            return Some(start.elapsed());
        }
        if start.elapsed() > timeout {
            return None;
        }
        sleep(Duration::from_micros(500));
    }
}

fn reap(pid: i32) -> io::Result<(libc::c_int, libc::rusage)> {
    let mut status: libc::c_int = 0;
    let mut ru = MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: `status` and `ru` are valid out-pointers; wait4 fills both on success.
    let rc = unsafe { libc::wait4(pid, &raw mut status, 0, ru.as_mut_ptr()) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: wait4 succeeded and initialised the rusage struct.
    Ok((status, unsafe { ru.assume_init() }))
}

fn rusage_cpu_ms(ru: &libc::rusage) -> f64 {
    let tv = |t: libc::timeval| t.tv_sec as f64 * 1e3 + f64::from(t.tv_usec) / 1e3;
    tv(ru.ru_utime) + tv(ru.ru_stime)
}

// ---------------------------------------------------------------------------
// inproc: in-process create/release vs spawn/kill caffeinate
// ---------------------------------------------------------------------------

fn bench_inproc(opts: &Options) -> Result<(), Box<dyn Error>> {
    let mut config = hold_config();
    config.declare_user_active = opts.user_active;
    let n_kinds = config.kinds().count();
    let extra = if opts.user_active {
        " + UserIsActive"
    } else {
        ""
    };
    println!("## Mechanism micro-benchmark: {n_kinds} assertions{extra} per cycle\n");

    let mut inhibitor = Inhibitor::new("bluemeth-bench");
    for _ in 0..20 {
        inhibitor.apply(&config)?;
        inhibitor.release_all()?;
    }
    let mut create = Vec::with_capacity(opts.iterations);
    let mut release = Vec::with_capacity(opts.iterations);
    eprint!("in-process: {} timed cycles", opts.iterations);
    for _ in 0..opts.iterations {
        let t = Instant::now();
        inhibitor.apply(&config)?;
        create.push(t.elapsed().as_secs_f64() * 1e6);
        let t = Instant::now();
        inhibitor.release_all()?;
        release.push(t.elapsed().as_secs_f64() * 1e6);
    }
    // Memory is measured over a second, bookkeeping-free loop so the sample
    // vectors above cannot show up as "growth".
    eprint!(", {} plain cycles", opts.iterations);
    let before = ProcessUsage::sample_self()?;
    for _ in 0..opts.iterations {
        inhibitor.apply(&config)?;
        inhibitor.release_all()?;
    }
    let after = ProcessUsage::sample_self()?;
    eprintln!();

    let mut spawn = Vec::with_capacity(opts.spawn_iterations);
    let mut stop = Vec::with_capacity(opts.spawn_iterations);
    eprint!("caffeinate subprocess: {} cycles", opts.spawn_iterations);
    for _ in 0..opts.spawn_iterations {
        let t = Instant::now();
        let process = CaffeinateProcess::spawn(&config)?;
        let pid = process.pid();
        wait_for_assertion_count(pid, n_kinds, Duration::from_secs(5))
            .ok_or("caffeinate assertions never appeared")?;
        spawn.push(t.elapsed().as_secs_f64() * 1e6);
        let t = Instant::now();
        process.stop()?;
        wait_for_assertion_count(pid, 0, Duration::from_secs(5))
            .ok_or("caffeinate assertions never released")?;
        stop.push(t.elapsed().as_secs_f64() * 1e6);
    }
    eprintln!();

    let mut t = Table::new(["operation", "n", "median (µs)", "p95 (µs)", "max (µs)"]);
    for (label, samples) in [
        ("IOKit in-process: create", &create),
        ("IOKit in-process: release", &release),
        (
            "caffeinate subprocess: spawn until assertions visible",
            &spawn,
        ),
        ("caffeinate subprocess: kill until assertions gone", &stop),
    ] {
        let s = Summary::of(samples);
        t.row([
            label.to_owned(),
            s.n.to_string(),
            fmt(s.median),
            fmt(s.p95),
            fmt(s.max),
        ]);
    }
    println!("{}\n", t.render());

    println!(
        "### Memory over {} in-process create/release cycles\n",
        opts.iterations
    );
    let mut m = Table::new(["", "footprint (MB)", "resident (MB)"]);
    m.row([
        "before".to_owned(),
        fmt(before.phys_footprint as f64 / 1e6),
        fmt(before.resident_size as f64 / 1e6),
    ]);
    m.row([
        "after".to_owned(),
        fmt(after.phys_footprint as f64 / 1e6),
        fmt(after.resident_size as f64 / 1e6),
    ]);
    let growth = after.phys_footprint as i64 - before.phys_footprint as i64;
    m.row([
        "delta".to_owned(),
        fmt(growth as f64 / 1e6),
        fmt((after.resident_size as i64 - before.resident_size as i64) as f64 / 1e6),
    ]);
    println!("{}\n", m.render());
    Ok(())
}

// ---------------------------------------------------------------------------
// watch: sample arbitrary running processes
// ---------------------------------------------------------------------------

fn bench_watch(opts: &Options) -> Result<(), Box<dyn Error>> {
    if opts.pids.is_empty() {
        return Err("watch needs at least one --pid".into());
    }
    let mut first = Vec::new();
    for pid in &opts.pids {
        first.push(ProcessUsage::sample(*pid).map_err(|e| format!("pid {pid}: {e}"))?);
    }
    eprintln!(
        "sampling {} process(es) for {}s",
        opts.pids.len(),
        opts.window.as_secs_f64()
    );
    sleep(opts.window);
    let mut t = Table::new([
        "pid",
        "process",
        "footprint (MB)",
        "peak footprint (MB)",
        "CPU in window (ms)",
        "idle wake-ups",
        "interrupt wake-ups",
        "energy (mJ)",
    ]);
    for (pid, a) in opts.pids.iter().zip(first) {
        let b = ProcessUsage::sample(*pid).map_err(|e| format!("pid {pid}: {e}"))?;
        let d = b.delta_since(&a);
        t.row([
            pid.to_string(),
            process_name(*pid),
            fmt(b.phys_footprint as f64 / 1e6),
            fmt(b.lifetime_max_phys_footprint as f64 / 1e6),
            fmt(d.cpu_time_ns() as f64 / 1e6),
            d.pkg_idle_wakeups.to_string(),
            d.interrupt_wakeups.to_string(),
            fmt(d.billed_energy_nj as f64 / 1e6),
        ]);
    }
    println!(
        "## Steady-state over a {}s window\n",
        opts.window.as_secs_f64()
    );
    println!("{}\n", t.render());
    Ok(())
}

fn process_name(pid: i32) -> String {
    let mut buf = [0u8; 256];
    // SAFETY: valid writable buffer of the stated size; proc_name NUL-terminates.
    let len = unsafe { libc::proc_name(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if len <= 0 {
        return "?".to_owned();
    }
    std::ffi::CStr::from_bytes_until_nul(&buf)
        .map_or_else(|_| "?".to_owned(), |s| s.to_string_lossy().into_owned())
}
