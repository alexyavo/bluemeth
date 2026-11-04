# Architecture

```
crates/
  core/   bluemeth-core   platform API: Config, Inhibitor, AssertionKind
          └─ macos/       IOKit FFI, RAII assertions, assertion-table inspector,
                          `caffeinate` subprocess backend, proc_pid_rusage sampler
  cli/    bluemeth        caffeinate-compatible CLI (kqueue wait loop, no polling)
  app/    bluemeth-app    AppKit menu bar app (objc2); bundled as bluemeth.app
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

bluemeth makes exactly those calls from inside the process that wants to stay
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
  or while user-activity refreshes are needed (every 15 s); otherwise none,
  unless the optional input simulation below is enabled.

`crates/core/src/macos/caffeinate.rs` keeps the subprocess approach as a
reference implementation so the two can be measured side by side
(`bluemeth-bench inproc`).

## Optional input simulation

The menu bar app's **Simulate Activity** setting is independent of IOKit and
the CLI. It stays off by default and is excluded from the global Enable
action; global Disable stops it. `input_activity.rs` calls CoreGraphics
directly, posting an F20 key-down followed immediately by its key-up at the
HID event tap. It generates no mouse events. Both events are allocated before
posting either so allocation failure cannot leave F20 pressed. Each event
is a `CFRetained<CFType>` and is released on return.

The existing AppKit timer checks every 30 seconds while simulation is enabled.
It reads the combined session input-idle clock and only posts after 60 seconds
of inactivity, provided no mouse button, Shift/Control/Option/Command/Fn key,
or F20 key is held. There are no event taps, input logs, helper processes, or extra
threads. A delayed timer performs at most one check, without catching up.

Only clicking the option calls `CGRequestPostEventAccess`; restoring settings
and timer ticks use `CGPreflightPostEventAccess` without prompting. Missing
access stops simulation, clears the saved toggle, and displays an error.
The permission dialog's **Open Settings** button uses `NSWorkspace` to open
`x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility`.
The link remains valid on macOS 27, where the pane is named **Device Control
and Data Access**; displayed instructions and the tooltip use the name for
the running OS version. **Not Now** leaves the option off; a failed URL open
shows the matching manual path.
Settings and the common session deadline cover simulation even without any
IOKit assertions. Expiry is checked before posting, using the core's
sleep-inclusive monotonic clock. Changing the timeout restarts the session;
changing other toggles preserves its deadline.

Input-idle time is separate from `IOPMAssertionDeclareUserActivity`. Chromium's
[macOS idle implementation](https://raw.githubusercontent.com/chromium/chromium/main/ui/base/idle/idle_mac.mm)
reads `CGEventSourceSecondsSinceLastEventType` with the combined session state.
Input simulation is still best effort for Slack: it does not override a
manual away status or guarantee presence while locked, disconnected, or in
a browser. It can also wake or hold the display awake, and apps or custom
shortcuts bound to F20 can react to it.

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
  create/release cycles in 10 000-cycle segments until one segment leaves
  the live heap (every malloc block, enumerated with libmalloc's own zone
  enumerator as `heap` does) completely unchanged, which a per-cycle leak
  can never do.
* `scripts/leakcheck.sh` runs Apple's `leaks` on the CLI (`--atExit`) and the
  live app.
* CI also lints the finished bundle: `plutil -lint` on its Info.plist,
  `codesign --verify --strict`, and an `iconutil` round trip of the icon.
* App tests cover simulation scheduling, deadline expiry, stopping and
  restarting, invalid idle readings, and explicit opt-in. The ignored
  `f20_resets_input_idle_time_and_releases_key` test posts an F20 press/release
  pair and verifies the idle clock resets and F20 is released; run it from an Accessibility-
  authorized terminal with an idle desktop:
  `cargo test -p bluemeth-app f20_resets_input_idle_time_and_releases_key -- --ignored`.
* `bluemeth-bench` reports CPU time, idle wake-ups, physical footprint, and
  energy from `proc_pid_rusage`, the same counters Activity Monitor shows.
  See `docs/BENCHMARKS.md`.

## Why the app binary is called `bluemeth-app`

Cargo puts every workspace binary in `target/<profile>/`. The CLI is
`bluemeth`, so the app's binary needs another name: on macOS's case-insensitive
filesystem even a name that differs only in case would be the *same file*, and
the two would overwrite each other. The bundle keeps that name
(`bluemeth.app/Contents/MacOS/bluemeth-app`), so `pgrep -x bluemeth` and
`killall bluemeth` only ever match the CLI. Menus, Login Items and Activity
Monitor show the bundle's name, `bluemeth`.
