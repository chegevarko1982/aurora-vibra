# Changelog

## v4.3.2

Patch release. Fixes the in-app updater failing with «Отказано в доступе (os error 5)» / "Access is denied".

**Fixes:**
- The update helper started copying files while the application was still running, so writing the new `.exe` over the running one failed with access denied. The helper now waits for the application's process to actually exit (it is passed its PID), the "Updating…" message box no longer blocks that exit — it is shown before the helper is launched, not after — and if a target file is still locked anyway, it is renamed aside instead of aborting the update. Left-over `*.aurora-old` files are cleaned up on the next start.

**Note:**
- Because the faulty code lives in the *installed* version, updating **from v4.3.1 or earlier still fails** — install this release manually once (download the zip below and unpack it over your existing folder). Automatic updates work from v4.3.2 onwards.

## v4.3.1

Patch release. Fixes restoring the window from the tray icon.

**Fixes:**
- Clicking (or double-clicking) the tray icon did not bring the window back. Two causes: the window title the tray searched for was hardcoded to an older version string and no longer matched the real window, and even when found, a window hidden by **close to tray** was only un-minimized, never made visible again. The title is now derived from the package version, and the restore path explicitly shows the window.
- The in-app updater no longer gives up when Windows returns `ERROR_ELEVATION_REQUIRED` while spawning the update helper: the helper is retried through the shell so the UAC prompt appears instead.

## v4.3.0

First stable release with three simulators. Supersedes `v4.3.0-rc1` — same code, now validated against a running X-Plane 12.

**New:**
- **X-Plane 12 support.** A third simulator link, alongside MSFS and War Thunder. Telemetry arrives over X-Plane's built-in RREF protocol on UDP port 49000 — no plugin, no SDK, nothing copied into `Resources/plugins`. X-Plane drives the same eleven effects and the same settings as MSFS: it is wired in as a second telemetry source for the existing effects engine, not as a separate feature set, so anything tuned for one simulator carries across to the other.
- Active-game detection and the manual override in **Options** now cover three simulators instead of two.
- X-Plane runs a self-check a few seconds after the first telemetry arrives and logs every dataref that never reported a value — X-Plane silently ignores subscriptions to names it does not know, which would otherwise surface only as an effect that quietly never fires.
- War Thunder: new **Overspeed (Vne)** effect, driven by a per-vehicle Vne table covering ~1300 vehicles, with the threshold lowered according to flap position.
- War Thunder: **Gear Overspeed** split out as its own effect with an independent (narrower) speed window, so exceeding Vlo is felt distinctly from exceeding Vne.
- War Thunder: built-in session recorder (**Options → record session**) captures raw telemetry for tuning and regression tests without running the separate `wt_probe` tool.

**Fixes:**
- War Thunder: on aircraft with no usable `weapon1..4` trigger keys, the ammo-based firing fallback always reported Weapon 1, leaving the second weapon group silent on aircraft carrying two ammunition types. The fallback now clusters counters into the two weapon groups independently.
- War Thunder: rocket fire is routed to both weapon effects rather than one.
- The non-Windows build was broken: `sim::sim_worker`'s stub signature had drifted from the Windows version and was missing its `GameSlot` parameter. This affected contributors and the Docker development image, not the shipped Windows build.

**Notes:**
- X-Plane support has been validated against a running X-Plane 12 by a tester. MSFS and War Thunder code paths are untouched by the X-Plane work.
- The stall/AoA buffet profile for War Thunder still uses Bf 109 F-4 aerodynamic data applied to every aircraft, pending per-aircraft profiles.

## v4.2.0-rc1

Release candidate — not distributed through the in-app auto-updater (see note below); download it manually from the GitHub Releases page if you want to test it.

**New:**
- War Thunder support: live telemetry link driving tactile effects for gunfire (weapon1/weapon2), flaps, landing gear transit, engine start/stop, and an AoA-based stall/airflow-separation buffet
- Ammo tracking gained a fallback for aircraft that expose no `weapon1..weapon4` keys at all: firing is inferred from a decrease in the sum of ammo-like telemetry fields
- Active game (MSFS / War Thunder) is now auto-detected and switches the GUI and effect set automatically
- WinWing Orion joystick/throttle support, plus a second Ursa Minor RUD
- N2 idle quick-set and "apply to current aircraft" buttons for profiles
- Active-game indicator in the top bar is now a plain text label ("MSFS"/"WarThunder") instead of an icon

**Notes:**
- The stall/AoA buffet profile uses real Bf 109 F-4 aerodynamic data and, as a temporary measure, is now applied to every aircraft (previously F-4 only) until per-aircraft profiles are added — expect it to feel wrong on other airframes until then

## v4.1.1

**Security:**
- Updated the HTTPS stack used by the auto-updater. The previous release shipped `rustls-webpki` 0.101.7 by way of `reqwest` 0.11, which carries three advisories — RUSTSEC-2026-0098 and -0099 (certificate name constraints accepted where they should have been rejected) and RUSTSEC-2026-0104 (a panic reachable while parsing certificate revocation lists). Exploiting any of them requires a misissued certificate, so real-world exposure was low, but the updater is the one component that fetches and installs code, and it should not be running on a stack with known findings.
- Further dependency updates close RUSTSEC-2026-0007 in `bytes` and unsoundness advisories in `anyhow`, `memmap2` and `rand`. `cargo audit` now reports a clean tree.

**New:**
- USDT (TRC-20) added to the Donate window alongside the existing options

**Fixes:**
- The library could not be compiled on non-Windows targets at all: `serde` and `serde_json` were declared as Windows-only dependencies while cross-platform modules used them unconditionally. This affected contributors and the Docker development image, not the shipped Windows build.

## v4.1.0

**New:**
- Redesigned interface: dark palette, sectioned navigation, effect cards with live status badges, and a Live Monitor panel that can be collapsed out of the way
- Fenix A320 support: overspeed barber-pole and gear-transit telemetry, per-variant idle N2, and a clear precedence order between built-in and user profile overrides
- Reworked gear touchdown: attack–hold–decay envelope per strut, fixed motor routing, and a settle window that mutes every other effect (except Stall) right after contact so the touchdown itself is not masked
- "Close to tray" option and a restructured tray context menu
- **Options → Check for updates** in the toolbar, alongside the tray entry
- Donate menu with selectable BTC / YooMoney details
- Single-instance guard — launching the app twice focuses the existing window instead of opening a second one

**Security:**
- The auto-updater now verifies the downloaded archive against the release's published `SHA256SUMS.txt` **before** unpacking or installing anything, and refuses to proceed if the checksum is missing or does not match. Previously the archive was unpacked and copied over the application directory — with an automatic UAC elevation when needed — without any verification at all.
- Version comparison no longer misreads pre-release tags: `4.2.0-rc1` used to parse as `4.0.0` and could be offered as an upgrade over a newer stable build. Pre-releases are now skipped entirely.

**Fixes:**
- Corrected an `elem[]` index desync that made PMDG and Fenix SimConnect variables read from the wrong slots
- Engine start/shutdown routing, gear-transit gating, and HID hot-plug recovery
- Slats tracking unified with flaps, with a dither tolerance so sub-percent telemetry jitter no longer triggers the effect
- Window title and the .exe version resource are both derived from the package version, so they can no longer drift apart
- Effect status badge lights only on real activation
- MD-11 spoiler effect no longer false-triggers on bank

**Internal:**
- Minimum supported Rust version is now 1.88
- Diagnostic binaries in `src/bin/` moved behind a `dev-tools` feature — release builds produce one executable instead of eleven
- Only one TLS stack is linked into the binary (rustls); `native-tls`/schannel are no longer pulled in
- Clippy now runs over the Windows-only UI, tray, and updater code in CI, plus a `cargo audit` job

## v4.0.1

**Rebrand:** Ursa Minor FFB is now **Aurora Vibra**. New name and icon throughout the app (window title, tray, installer metadata), package/binary renamed (`aurora-vibra.exe`), auto-updater now points at the new repository, settings/log folder moved to `%LOCALAPPDATA%\AuroraVibra`. Hardware device names (WinWing/WINCTRL joystick & throttle identifiers) are unchanged — only the application's own branding moved.

**New:**
- English/Russian UI language switch (EN by default) — toolbar toggle, covers the main window, tray menu, and update dialogs; choice is remembered between runs
- Per-aircraft configuration profiles: save/load your own preset per tail number (manual Save/Load, falls back to a default profile), on top of built-in overrides for MADDOG and Learjet
- Reworked engine-start effect: dedicated starter-spool phase tuned against real PMDG 737 SimConnect data, smooth fade-out on an aborted start instead of a hard cutoff, fixed-strength ignition kick
- Overspeed effect now tracks the live airspeed barber pole (Vmo/Mmo) instead of a static design speed, with a manual override and an extra trigger for the Learjet
- Separate strut-compression touchdown effect for nose/left/right gear, no longer confused with the ground-roll effect
- Effect sliders (Ground Roll, Flaps, Gear, Stall, Spoilers) now show 0–100% instead of raw internal units
- Collapsible telemetry panel in the UI, collapsed by default
- Independent left/right sidestick vibration channels, plus a hand-swap option for non-standard joystick/throttle seating
- Reserved "Options" menu and help ("?") button in the toolbar for upcoming features
- Default window size now shows the entire effects list without scrolling; window size/position are no longer remembered between runs, so this default always applies on launch

**Fixes:**
- Settings autosave no longer stalls indefinitely while the UI is active (it kept pushing its own save deadline forward every redraw)
- TFDI MD-11: spoiler effect no longer false-triggers on bank
- Gear touchdown effect no longer false-triggers while parked due to near-zero ground-speed telemetry noise

## v3.0.0

- Aircraft-based advanced rumble effects, plus airliner-specific effects (spoilers, engine N1)
- Rumble profiles and presets with per-preset customization, reworked preset UX
- Windows MSI installer
- Support for all sidestick variant types
- Stability fixes for rumble effects and SimConnect variable reads

## v2.3

- New runway rolling physics: individual gear struts and reworked flap logic
- Telemetry for nose/main gear strut compression surfaced in the UI
- Refactored force-feedback logic and the HID/Sim modules

## v2.1.1

- Settings are now saved automatically

## v2.1.0

- Stock spoilers effect
- Taxi Thump extended up to 250 kt
- Overspeed option
- PWM vibration output
- Full test suite for the project

## v2.0.1

- Support for Fighter and Space sidesticks

## v2.0.0

- Fixes HID payload for URSA MINOR R version

## v1.2.0

- Improve HID device selection reliability
- Possible fix for L/R handed Ursa Minor sidesticks

## v1.1.4

- Add better logging for HID worker

## v1.1.3

- Fixes auto-updater helper: Runs in elevated mode if needed

## v1.1.2

- Fixes SimConnect issues for users that weren't able to connect to the sim

## v1.1.1

- Effects for Flaps, Landing gear and Paused states
- Fixes tray issues when interacting with context menu
- Auto-updater reliability improvements

## v1.1.0

- Improve effects for Flaps, Landing gear and Paused states
- Add tray menu and options
- Add auto updater

## v1.0.4

- Project first release
- SimConnect API integration
- Rumble feedback for Sim variables through HID
- HID integration with Sidestick
