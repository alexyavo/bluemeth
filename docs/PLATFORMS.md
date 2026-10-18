# Platform support

Bluemeth is macOS-first. The crate layout keeps the platform-specific part
small so other operating systems can be added without touching the CLI or
the app's state logic.

| Layer | macOS | Linux | Windows |
| --- | --- | --- | --- |
| `bluemeth-core` backend | IOKit power assertions (done) | stub (`Error::Unsupported`) | stub (`Error::Unsupported`) |
| `bluemeth` CLI | full | compiles; exits with "unsupported" | compiles; exits with "unsupported" |
| `bluemeth-app` menu bar UI | AppKit (done) | not started | not started |
| `bluemeth-bench` | `proc_pid_rusage` (done) | not started | not started |

## How the backend abstraction works

`bluemeth-core` exposes one platform-neutral type, `Inhibitor`, which
reconciles a `Config` (which kinds of sleep to prevent, optional timeout)
against what the OS currently holds. Each backend module provides:

```rust
pub struct Assertion;            // RAII: released on drop
impl Assertion {
    pub fn create(kind: AssertionKind, name: &str, details: &str,
                  reason: &str, timeout: Option<Duration>) -> Result<Self>;
    pub fn release(self) -> Result<()>;
}
pub struct UserActivity;          // RAII: "the user is here" declaration
impl UserActivity {
    pub fn declare(name: &str) -> Result<Self>;
    pub fn redeclare(&mut self) -> Result<()>;
    pub fn release(self) -> Result<()>;
}
pub fn is_already_released(code: i32) -> bool;
pub fn monotonic_including_sleep() -> Duration;
```

`src/unsupported.rs` is the template: copy it to `src/linux/mod.rs` or
`src/windows/mod.rs`, wire it in `lib.rs` behind `cfg(target_os = ...)`, and
`Inhibitor`, the CLI, and the tests that do not touch IOKit work unchanged.

## Mapping to other platforms

### Windows

`SetThreadExecutionState` is the whole API:

| `AssertionKind` | flags |
| --- | --- |
| `DisplaySleep` | `ES_CONTINUOUS \| ES_DISPLAY_REQUIRED` |
| `IdleSystemSleep` / `SystemSleep` | `ES_CONTINUOUS \| ES_SYSTEM_REQUIRED` |
| `DiskIdle` | no equivalent (no-op) |
| user active | `ES_DISPLAY_REQUIRED` without `ES_CONTINUOUS`, re-declared periodically |

The state is per-thread and not refcounted, so the backend must compute the
union of held kinds and call `SetThreadExecutionState(ES_CONTINUOUS | union)`
after every change, and `SetThreadExecutionState(ES_CONTINUOUS)` to clear.
Timeouts have no OS-side equivalent; the `Inhibitor`'s own deadline handles
them. A single hand-written `extern "system"` declaration against
`kernel32` is enough; no crate needed.

### Linux

Two cooperating mechanisms, both over D-Bus:

* `org.freedesktop.login1.Manager.Inhibit("sleep:idle", who, why, "block")`
  returns a file descriptor; closing it releases the inhibitor. Maps to
  `IdleSystemSleep` / `SystemSleep`.
* `org.freedesktop.ScreenSaver.Inhibit(app, reason)` returns a cookie for
  `UnInhibit`. Maps to `DisplaySleep`. `SimulateUserActivity` maps to
  user-active.

`zbus` is the usual dependency; a lighter alternative that keeps the binary
small is shelling out to `systemd-inhibit` the way the `caffeinate` backend
spawns Apple's tool, which is a reasonable first step and benchmarks the same
way.

### UI

The AppKit controller in `crates/app/src/controller.rs` is deliberately thin:
all decisions live in `settings.rs`, `presets.rs`, `format.rs` and in
`bluemeth-core`. A tray UI on another platform (e.g. `tray-icon` + `muda`,
or a GTK status icon) would reuse those modules verbatim and only reimplement
the menu wiring.
