# Benchmarks

All numbers below were produced by `bluemeth-bench` on an Apple M3 Pro
running macOS 27.0.1, release builds, 2026-10-18. Reproduce with
`make bench` (process + in-process), or run the individual modes described
at the end. Everything is measured with `proc_pid_rusage`, the kernel's
per-process accounting that Activity Monitor displays, so "footprint",
"idle wake-ups" and "energy" mean the same thing here as there.

## 1. Holding assertions: `caffeinate` vs. `bluemeth`

Both binaries are started with identical flags (`-dis -t 3600`), left alone
for a 10 s window, then sent `SIGTERM`. 5 trials each after one untimed
warm-up. "ready" is spawn → all three assertions visible in `powerd`'s table;
"release" is `SIGTERM` → table empty.

| mechanism                   |  ready (ms) | CPU in window (ms) | idle wake-ups | interrupt wake-ups |   energy (mJ) | footprint (MB) | peak footprint (MB) | release (ms) | lifetime CPU (ms) | max RSS (MB) |
| --------------------------- | ----------: | -----------------: | ------------: | -----------------: | ------------: | -------------: | ------------------: | -----------: | ----------------: | -----------: |
| caffeinate (Apple)          | 28.9 ± 4.43 |      0.357 ± 0.077 |         0 ± 0 |              2 ± 0 | 0.004 ± 0.001 |   2.38 ± 0.016 |        2.43 ± 0.016 | 3.06 ± 0.615 |       22.7 ± 3.43 | 3.41 ± 0.016 |
| bluemeth (in-process IOKit) | 19.2 ± 7.31 |      0.099 ± 0.022 |         0 ± 0 |              1 ± 0 | 0.007 ± 0.003 |    2.42 ± 0.02 |         2.45 ± 0.02 |  2.55 ± 1.08 |       14.9 ± 4.88 |  7.51 ± 0.02 |

Reading it:

* **Idle cost.** Over a 10 s hold the `bluemeth` process used 0.10 ms of
  CPU to `caffeinate`'s 0.36 ms (3.6× less) and took one interrupt wake-up to
  its two. Neither had an idle wake-up; both are far below anything a user
  could notice. `bluemeth` idles inside one `kevent` call and does nothing
  until a signal, a watched process exit, or the deadline.
* **Memory.** Physical footprint, the number Activity Monitor shows, is the
  same within 2 % (2.42 vs 2.38 MB). RSS is higher for `bluemeth` (7.5 vs
  3.4 MB) because RSS also counts the shared-library pages a process has
  touched and the statically linked Rust binary touches more of them; those
  pages are shared with every other process using the same libraries and are
  not attributed to the process's footprint.
* **Latency.** Ready in 19 ms vs 29 ms; released in 2.6 ms vs 3.1 ms. The
  release path differs in kind, not just speed: `bluemeth` releases
  synchronously and exits, while `caffeinate`'s assertions disappear only
  once `powerd` notices the process is gone.
* **Energy** is at the noise floor for both (µJ per second).

### One-shot lifecycle (`-i -t 1`: spawn, hold 1 s, exit)

| mechanism                   | `-i -t 1` wall (ms) | lifetime CPU (ms) | max RSS (MB) |
| --------------------------- | ------------------: | ----------------: | -----------: |
| caffeinate (Apple)          |         1057 ± 1.54 |       9.18 ± 2.61 |     2.42 ± 0 |
| bluemeth (in-process IOKit) |         1021 ± 3.68 |        14.7 ± 2.9 | 6.22 ± 0.013 |

`bluemeth` is on time (21 ms over the second, vs 57 ms for `caffeinate`) but
spends about 5 ms more CPU starting up, consistent with paging in a 412 KB
static binary versus a 212 KB one that lives in the shared cache. For a
process that runs for seconds this is a wash; for one that runs for hours
the steady-state column above is what matters, and there `bluemeth` repays
the difference after roughly 20 s.

### Binary size

| binary                      | size (KB) |
| --------------------------- | --------: |
| caffeinate (Apple)          |       212 |
| bluemeth (in-process IOKit) |       412 |

## 2. The mechanism itself: in-process vs. spawn

What a menu bar app pays each time the user flips a toggle. "create" and
"release" are `Inhibitor::apply` / `Inhibitor::release_all` for three
assertions, measured in-process; the subprocess rows spawn
`/usr/bin/caffeinate -dis`, wait until its assertions are visible, kill it,
and wait until they are gone.

| operation                                             |    n | median (µs) | p95 (µs) | max (µs) |
| ----------------------------------------------------- | ---: | ----------: | -------: | -------: |
| IOKit in-process: create                              | 1000 |         119 |      201 |      303 |
| IOKit in-process: release                             | 1000 |        71.5 |      134 |      213 |
| caffeinate subprocess: spawn until assertions visible |   20 |        7056 |     8125 |     8546 |
| caffeinate subprocess: kill until assertions gone     |   20 |         1215 |     1480 |     1561 |

Creating is 59× faster and releasing 17× faster in-process, and, more
importantly for a UI, toggling one option changes one assertion instead of
restarting a process with new flags (which briefly releases everything).

### Memory over repeated cycles

30 000 create/release cycles of three assertions plus a UserIsActive
declaration (`bluemeth-bench inproc --iterations 30000 --user-active`):

|        | footprint (MB) | resident (MB) |
| ------ | -------------: | ------------: |
| before |           3.21 |          8.29 |
| after  |           3.21 |          8.29 |
| delta  |              0 |             0 |

The footprint steps up by exactly two pages (32 KiB) once, within the first
second or two of churn, and never again; the core test
`repeated_cycles_do_not_grow_memory` measures 10 000 cycles after a warm-up
and fails on 64 KiB of growth (an 8-byte-per-cycle leak would add 80 KiB).

## 3. The menu bar app at rest

`Bluemeth.app` (release bundle) sampled for 30 s next to (a) a do-nothing
Objective-C status-item app, `scripts/leak-baseline.m`, that represents
AppKit's floor, and (b) a `caffeinate -sdui` that had been running for an
hour. Bluemeth is holding display, idle and system assertions with no
countdown displayed, so it has no timer running at all.

| process                          | footprint (MB) | peak footprint (MB) | CPU in 30 s (ms) | idle wake-ups | interrupt wake-ups | energy (mJ) |
| -------------------------------- | -------------: | ------------------: | ---------------: | ------------: | -----------------: | ----------: |
| Bluemeth.app (no countdown)      |           12.9 |                13.4 |             2.17 |             0 |                  6 |       0.004 |
| baseline ObjC app (1 Hz timer)   |           12.3 |                12.6 |              155 |             0 |                169 |        2.09 |
| caffeinate -sdui (CLI, no UI)    |            2.3 |                 2.3 |                0 |             0 |                  0 |           0 |

AppKit costs about 12 MB just to put an item in the menu bar; Bluemeth adds
0.6 MB on top of that and otherwise does nothing: 2 ms of CPU and six
wake-ups in half a minute.

With a countdown showing, Bluemeth updates the status item once per second.
That costs what it costs any AppKit app to redraw a status item at 1 Hz, as
the baseline app with its own 1 Hz title timer shows:

| process                          | footprint (MB) | CPU in 30 s (ms) | idle wake-ups | interrupt wake-ups | energy (mJ) |
| -------------------------------- | -------------: | ---------------: | ------------: | -----------------: | ----------: |
| Bluemeth.app (1 Hz countdown)    |           13.4 |              199 |             1 |                157 |        1.66 |
| baseline ObjC app (1 Hz timer)   |           12.2 |              172 |             3 |                159 |        1.70 |

That is roughly 0.6 % of one core while a countdown is visible; the timer
is invalidated the moment the countdown ends or is set to "Never".

## 4. Leaks

`scripts/leakcheck.sh`, Apple's `leaks` with `MallocStackLogging`, binaries
re-signed with `get-task-allow` so all memory is inspectable:

```
== CLI: hold -dimsu for 1 s, then exit
   0 leaks, 0 bytes
== CLI: run a utility
   0 leaks, 0 bytes
== App vs. baseline AppKit app (3 s and 13 s after launch)
                            t=3s leaks         bytes |  t=13s leaks         bytes
   Bluemeth app              417         26496       |    417         26496
   baseline ObjC app         417         26496       |    417         26496
   OK: no leaks beyond the AppKit baseline, no growth over time
```

The 417 blocks are identical in both apps: a retain cycle around an
`NSXPCConnection` to `com.apple.linkd.autoShortcut` that AppIntents creates
in every AppKit process on this macOS version. None of the reported stacks
pass through Bluemeth, objc2 or IOKit, and the count does not change while
Bluemeth's countdown and user-activity timers run.

This check found one real bug during development: `UserActivity::release`
used `mem::forget(self)`, which also forgot the 48-byte owned name string.
It is now released by swapping in the null id and letting Drop run.

## Reproducing

```sh
cargo build --release --workspace
target/release/bluemeth-bench process                      # section 1
target/release/bluemeth-bench inproc                       # section 2
target/release/bluemeth-bench inproc --iterations 30000 --user-active
open target/Bluemeth.app && target/release/bluemeth-bench watch --pid $(pgrep -x Bluemeth) --window 30
scripts/leakcheck.sh                                       # section 4
```

`bluemeth-bench watch` accepts several `--pid` flags, so any other keep-awake
app can be put in the same table.
