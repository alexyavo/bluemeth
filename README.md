<p align="center"><img src="docs/icon.png" alt="bluemeth icon" width="128"></p>

# bluemeth

A keep-awake menu bar app for macOS that does what `caffeinate` does, without
`caffeinate`. Written in Rust, one process, nothing spawned.

<img src="docs/menu.png" alt="bluemeth menu" width="247" align="right">

* **Prevent Display Sleep**, **Prevent Idle Sleep**, **Prevent System Sleep**
  and **Declare User Active**, each toggled independently. These are the four
  things `caffeinate -d -i -s -u` do.
* **Simulate Activity**, an additional opt-in option, presses and releases
  **F20** after 60 seconds of input inactivity. It generates no mouse events.
  Requires macOS
  **Accessibility** access and can help the Slack desktop app stay active.
* **Timeout**: never, a preset (5 min to 4 h) or a custom duration such as
  `1h30m`. The remaining time counts down in the menu bar.
* **Enable / Disable** at the top toggles the four keep-awake options together.
  Simulate Activity must be enabled separately; Disable stops it too.
  **Launch at Login** starts the app automatically.
* Settings persist across launches; relaunching at login resumes keeping the
  Mac awake.

### Activity simulation and Slack

Select **Simulate Activity**, approve bluemeth under **System Settings →
Privacy & Security → Accessibility**, then select the option again if it
remained unchecked. Permission is requested only when you select this option.
If access is missing on launch or revoked later, simulation turns off and the
menu explains how to restore it. The option is off by default, and an older
installation stays opted out when upgraded.

The app checks every 30 seconds and posts only after at least 60 seconds
without input, skipping held mouse buttons, modifier keys, and F20. An app
or custom shortcut bound to F20 may respond to the simulated keypress. Simulation
uses the same timeout as the other options and works when they are all off.
For long sessions, also enable **Prevent Idle Sleep** so the Mac stays awake
between checks. Disable, timeout expiry, and quitting all stop simulation.

Slack must be running and connected, and must not be manually set to away.
This is best effort: a locked screen, Slack in a browser, or changes to Slack's
presence rules can still show you as away. It does not set your Slack status
through an API. [Slack documents its automatic and manual away rules here.](https://docs.slack.dev/apis/web-api/user-presence-and-status/)

Simulated input may wake the display or prevent display sleep. To let the
screen sleep while background processes continue, leave **Prevent Idle
Sleep** on and turn **Prevent Display Sleep**, **Declare User Active**, and
**Simulate Activity** off. Declare User Active only refreshes macOS power
management; it does not generate input for apps' inactivity detection.

Also in the box:

* `bluemeth`, a `caffeinate`-compatible command line tool
  (`bluemeth -dis -t 1h`, `bluemeth -i make`, `bluemeth -w <pid>`,
  `bluemeth status`).
* `bluemeth-core`, the library both are built on, with a stub backend for
  other platforms (see [docs/PLATFORMS.md](docs/PLATFORMS.md)).
* `bluemeth-bench`, which measures all of the above against `caffeinate`
  using the kernel's own per-process accounting.

## How it works, and why it is lighter than `caffeinate`

`/usr/bin/caffeinate` is a thin wrapper over IOKit power assertions:
`IOPMAssertionCreateWithProperties` per flag, `IOPMAssertionDeclareUserActivity`
for `-u`, a `TimeoutSeconds` property for `-t`. bluemeth calls the same four
IOKit functions directly from the process that wants to stay awake. A menu bar
app that shells out to `caffeinate` instead pays for a second process on every
toggle: `fork`/`exec`, dyld, a second heap, and a kill-and-wait to release.

Measured on this machine (Apple silicon, macOS 27), full tables in
[docs/BENCHMARKS.md](docs/BENCHMARKS.md):

| | in-process IOKit (bluemeth) | spawn `caffeinate` |
| --- | ---: | ---: |
| Take 3 assertions | ~120 µs | ~7 ms |
| Release them | ~70 µs | ~1.2 ms |
| CPU used by the holding process over 10 s | 0.10 ms | 0.36 ms |
| Memory footprint of the holding process | 2.4 MB | 2.4 MB |
| Extra processes | 0 | 1 |

The app itself, holding all three assertions with no countdown shown, uses
about 2 ms of CPU per 30 s, zero idle wake-ups and 13 MB of memory, of which
12 MB is AppKit's floor for any status-item app (measured with a do-nothing
Objective-C app in `scripts/leak-baseline.m`). Showing the 1 Hz countdown
costs the same as it costs any AppKit app to redraw a status item once a
second, about 0.6 % of one core, and stops when the countdown does.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the design.

## Memory safety and leaks

The app is meant to run for weeks, so this is treated as a correctness
requirement, not a nice-to-have:

* Every assertion is an RAII handle; dropping anything releases it. The OS
  also drops all of a process's assertions when it exits, however it exits.
* `unsafe` exists only at the FFI edges, every block has a `SAFETY:` comment,
  and `clippy::undocumented_unsafe_blocks` is a hard error.
* `cargo test` includes end-to-end checks against `powerd`'s live assertion
  table and a test that runs create/release cycles in 10 000-cycle segments
  and fails unless one segment leaves the live heap (every malloc block,
  enumerated the way `heap` does it) completely unchanged in both block
  count and bytes, which a per-cycle leak can never do.
* `scripts/leakcheck.sh` runs Apple's `leaks` on the CLI (must report 0) and
  on the live app next to the Objective-C baseline app (must not exceed it,
  must not grow over time). `bluemeth-bench inproc` reports zero footprint
  growth over 30 000 cycles.

## Build, run, install

Requires Rust 1.85+ and Xcode Command Line Tools.

```sh
make build            # debug build of everything
make test             # unit + integration tests (they talk to the real powerd)
make lint             # rustfmt + clippy with -D warnings
make run              # run the menu bar app unbundled (Launch at Login disabled)
make bundle           # target/bluemeth.app, ad-hoc signed
make install          # ...and copy it to /Applications
make login            # register /Applications/bluemeth.app as a login item
make unlogin          # ...and remove it again
make icon             # re-render the app and menu bar icons from assets/*.png (needs ImageMagick)
make leaks            # scripts/leakcheck.sh
make bench            # release build + bluemeth-bench all
```

`Launch at Login` uses `SMAppService` and therefore needs the bundled app
(macOS 13+). The same switch is scriptable, which is what `make login` runs:

```sh
/Applications/bluemeth.app/Contents/MacOS/bluemeth-app --login-item enable|disable|status
```

The bundle is ad-hoc signed; replace `-` in `scripts/bundle.sh` with a
Developer ID to notarise. The app icon and the two menu bar icons (the pill
in colour while keeping awake, faded grey while idle) are rendered from the
artwork in `assets/` by `scripts/make-icon.sh` (Apple's 1024 px icon grid,
every `.icns` slot downsampled in linear light); the results are committed so
a plain `make bundle` needs no ImageMagick.

### CLI

```
bluemeth [-dimsu] [-t TIMEOUT] [-w PID] [--backend iokit|caffeinate] [utility [args...]]
bluemeth status [PID]
```

Flags match `caffeinate(8)`. Differences: `-t` accepts `90s`, `5m`, `1h30m`
as well as seconds; `-u` with `-t`, `-w` or a utility re-declares user
activity every 30 s for the whole duration instead of once; `status` prints
the system assertion table; `--backend caffeinate` spawns Apple's tool
instead, for A/B comparisons.

## Layout

```
crates/core    bluemeth-core    Config, Inhibitor, IOKit backend, inspector, rusage sampler
crates/cli     bluemeth         CLI (kqueue-based wait loop)
crates/app     bluemeth-app     AppKit menu bar app via objc2, bundled as bluemeth.app
crates/bench   bluemeth-bench   benchmarks
scripts/       bundle.sh, make-icon.sh, leakcheck.sh, leak-baseline.m
assets/        pill artwork, AppIcon.icns, StatusIcon.tiff, StatusIconIdle.tiff
docs/          ARCHITECTURE.md, BENCHMARKS.md, PLATFORMS.md
```

## License

MIT
