# Architecture

```
crates/
  core/   bluemeth-core   platform API: Config, Inhibitor, AssertionKind
          └─ macos/       IOKit FFI, RAII assertions, assertion-table inspector,
                          `caffeinate` subprocess backend, proc_pid_rusage sampler
  cli/    bluemeth        caffeinate-compatible CLI (kqueue wait loop, no polling)
  app/    bluemeth-app    AppKit menu bar app (objc2); bundled as Bluemeth.app
  bench/  bluemeth-bench  resource benchmarks: caffeinate vs bluemeth, in-process vs spawn
```

## The mechanism

`/usr/bin/caffeinate` is a small wrapper around IOKit's power-management
assertion API. It calls `IOPMAssertionCreateWithProperties` once per flag
(`-d` → `PreventUserIdleDisplaySleep`, `-i` → `PreventUserIdleSystemSleep`,
`-s` → `PreventSystemSleep`, `-m` → `PreventDiskIdle`),
`IOPMAssertionDeclareUserActivity` for `-u`, and attaches `TimeoutSeconds` +
`TimeoutActionRelease` for `-t`. Assertions belong to the calling process;
`powerd` releases them when the process exits.

Bluemeth makes exactly those calls from inside the process that wants to stay
awake. Compared with a menu bar app that shells out to `caffeinate`:

* **No second process.** One fewer process to launch, schedule, keep resident
  and clean up. `caffeinate` itself is tiny, but it still costs a `fork`/`exec`,
  dyld, its own heap, and its own run loop.
* **Release is a synchronous call, not a signal.** Toggling an option off
  returns when `powerd` has dropped the assertion, instead of "kill and wait
  for `powerd` to notice the exit".
* **Diffing instead of restarting.** Changing one toggle creates or releases
  one assertion; the others are untouched. A subprocess design has to kill and
  respawn with new flags, briefly releasing everything.
* **Nothing to poll.** The CLI blocks in one `kevent` call until a signal,
  a watched process exit, or the deadline; it has zero wake-ups while idle.
  The app runs a timer only while a countdown is visible (1 Hz, coalescable)
  or while user-activity refreshes are needed (every 15 s); otherwise none.

`crates/core/src/macos/caffeinate.rs` keeps the subprocess approach as a
reference implementation so the two can be measured side by side
(`bluemeth-bench inproc`).

## Memory and lifetime rules

* Every IOKit assertion is an RAII handle (`Assertion`, `UserActivity`);
  `Inhibitor` owns them; dropping anything releases it. There is no code path
  that forgets an id.
* After `powerd` releases an assertion on its own timeout, releasing it again
  returns `kIOReturnBadArgument` (observed) or `kIOReturnNotFound`
  (documented). Both are treated as "already gone"; any other error is
  surfaced to the caller rather than swallowed.
* All CoreFoundation objects go through `objc2-core-foundation`'s
  `CFRetained<T>`; all AppKit objects through `objc2`'s `Retained<T>`.
  `IOPMCopyAssertionsByProcess` hands back a +1 object that is wrapped with
  `CFRetained::from_raw`, so it is released exactly once.
* AppKit's only retain cycle risk is `NSTimer` → target. The controller
  stores the timer and invalidates it before replacing it or terminating.
* `unsafe` is confined to the FFI edges, every block carries a `SAFETY:`
  comment, and `clippy::undocumented_unsafe_blocks` is `deny`.
* `panic = "abort"` in release: a bug cannot leave the app half-alive. If the
  process dies for any reason, `powerd` drops its assertions.

## Verification

* `cargo test --workspace` includes end-to-end tests that create assertions
  and read them back from `powerd`'s table (`IOPMCopyAssertionsByProcess`),
  verify the timeout property and that `powerd` releases on its own, and run
  1000 create/release cycles asserting the physical footprint does not grow.
* `scripts/leakcheck.sh` runs Apple's `leaks` on the CLI (`--atExit`) and the
  live app.
* `bluemeth-bench` reports CPU time, idle wake-ups, physical footprint, and
  energy from `proc_pid_rusage`, the same counters Activity Monitor shows.
  See `docs/BENCHMARKS.md`.

## Why the app binary is called `bluemeth-app`

Cargo puts every workspace binary in `target/<profile>/`. The CLI is
`bluemeth`; on macOS's case-insensitive filesystem a binary named `Bluemeth`
would be the *same file*, and the two would overwrite each other. The bundle
script copies `bluemeth-app` to `Bluemeth.app/Contents/MacOS/Bluemeth`, so the
process name users see is still `Bluemeth`.
