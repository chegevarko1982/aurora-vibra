<div align="center">
<img src="assets/aurora-vibra-logo.png" alt="Aurora Vibra logo" width="220">

# ✈️ Aurora Vibra
### Advanced Flight Simulator Tactile & FFB Hub

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Platform: Windows](https://img.shields.io/badge/Platform-Windows-lightgrey.svg)]()
[![Built with Rust](https://img.shields.io/badge/Built%20with-Rust-orange.svg)]()

*Next-generation tactile feedback (force feedback / rumble) and telemetry utility for
**Microsoft Flight Simulator**, **X-Plane 12** and **War Thunder**.*

</div>

---

## 🌟 Overview

An advanced control hub for tactile feedback and physical response designed for flight
sticks, yokes, and throttles (including the WinWing ecosystem). This utility overcomes
default software limitations and delivers an immersive, realistic feel for heavy
jetliners, general aviation aircraft, and warbirds alike.

<img src="assets/screenshot-main.png" alt="Aurora Vibra main window — Aerodynamics effects and Live Monitor" width="900">

Effect groups sit on the left, per-effect intensity and thresholds in the middle, and live
telemetry on the right. Every effect can be routed to the sidestick, the throttle, or both.

**Three simulators, one app.** Aurora Vibra keeps a live link open to each supported
simulator at once and hands the hardware to whichever one is actually running — there is
no mode switch to remember. The GUI follows automatically, and a manual override is
available if you run two of them side by side.

| Simulator | Transport | Notes |
|---|---|---|
| Microsoft Flight Simulator 2020 / 2024 | SimConnect | Deep add-on support (PMDG, Fenix, MADDOG X, TFDi MD-11, Flysimware) |
| X-Plane 12 | UDP RREF, port 49000 | Nothing to install into the simulator |
| War Thunder | HTTP, `localhost:8111` | Enable the in-game web telemetry server |

---

## 📥 Download & Install

1. Grab the latest `aurora-vibra-vX.Y.Z-windows-x64.zip` from the
   [**Releases**](https://github.com/chegevarko1982/aurora-vibra/releases/latest) page.
2. *(Recommended)* Verify the download against the `SHA256SUMS.txt` published with it:

   ```powershell
   Get-FileHash -Algorithm SHA256 .\aurora-vibra-vX.Y.Z-windows-x64.zip
   ```

3. Unzip it into a folder **you can write to** — e.g. `%LOCALAPPDATA%\Aurora Vibra`
   or anywhere under `Documents`. The app stores its settings and log next to the
   executable when possible, and falls back to `%LOCALAPPDATA%\AuroraVibra` otherwise.
   Avoid `C:\Program Files`: the built-in updater would then need an
   administrator prompt on every update.
4. Run `Aurora Vibra.exe`. No installer, no registry entries, no admin rights required.

There is nothing else to install — the SimConnect client library is carried inside
the executable as a fallback (see [the licensing note](#-license) below), so the app
works even on machines where MSFS 2024 is the only simulator present. X-Plane and
War Thunder need no client library at all.

> **SmartScreen warning.** The executable is not code-signed, so Windows may show
> "Windows protected your PC" on first launch — choose **More info → Run anyway**.
> If you would rather not take that on trust, verify the checksum in step 2 or
> build from source (below).

### First run

- Start your simulator, then Aurora Vibra (either order works — it reconnects on its own).
- The top bar shows the active game, the link status, and two dots for the Sidestick and
  the Throttle, which light up once each device is detected.
- If something does not connect, `AuroraVibra.log` (next to the .exe, or in
  `%LOCALAPPDATA%\AuroraVibra`) records every path that was tried.

### Per-simulator setup

- **MSFS** — nothing to configure.
- **X-Plane 12** — nothing to install. The app subscribes to datarefs over X-Plane's
  built-in UDP protocol on port 49000. If no data arrives, check
  **Settings → Network** in the simulator.
- **War Thunder** — the local telemetry web server must be reachable on
  `http://localhost:8111`.

### Updating

**Options → Check for updates** in the toolbar, or the same entry in the tray menu.
The updater verifies the downloaded archive against the release's published SHA-256
before installing anything, and refuses to proceed if it does not match.

Release candidates (tags like `v4.3.0-rc1`) are published as GitHub pre-releases and are
deliberately **not** offered by the auto-updater — download them by hand if you want to
test one.

---

## 🚀 Key Features & Advantages

### 1. 🧩 Native support for complex high-fidelity add-ons (PMDG, MADDOG X, TFDi, Fenix)
- **Overcoming vendor limitations:** dedicated telemetry handling for study-level add-ons
  (**PMDG 737 / 777**, **Leonardo MADDOG X**, **TFDi MD-11**, **Fenix A320**,
  **Flysimware Learjet 35A**) where standard vendor tools either lack support or suffer
  from telemetry desync.
- **Lag-free sync:** direct reading of custom internal variables (L:Vars and SDK data
  areas) keeps physical vibration, cockpit instruments, and sound modules in sync.
- Add-on-specific variables are read unconditionally and self-neutralise on aircraft that
  do not define them, so nothing has to be toggled per flight.

### 2. 🛬 Progressive runway & taxi physics (Taxi & Takeoff Thump)
- **Seamless speed blending:** a physical model turns individual runway-joint thumps
  during taxi into a continuous, dense rumble as the aircraft accelerates down the runway.
  Slab length, thump duration, and the blending curve are all adjustable.
- **Independent gear struts:** individual load and compression processing for nose and
  main landing gear — a distinct tactile sense of main-gear touchdown followed by the
  nose gear lowering onto the centerline.
- **Touchdown envelope:** each strut fires an attack–hold–decay pulse whose *duration*
  encodes landing hardness, and a short settle window mutes competing effects so the
  touchdown itself is never masked.

### 3. 🎛️ Multi-device routing & asymmetric throttle "ping-pong"
- **Flexible channel addressing:** route any tactile effect independently to the flight
  stick, the throttle unit, or both — per effect, with live status badges showing what is
  firing right now.
- **Hydraulic mechanics simulation:** alternating vibration logic between the two throttle
  motors, mimicking the tactile feel of hydraulic pumps and electric actuators.
- **Hand-aware layouts:** touchdown and engine-start effects map left/right to the
  physical devices actually connected, with a swap toggle for left-handed setups.

### 4. 🔥 Detailed engine start & turbine telemetry
- **Spool-up tracking (N2):** monitors real high-pressure spool acceleration from 0 %
  through starter cutout, dynamically shaping vibration at fuel ignition.
- **Four-stage model:** pre-combustion starter ramp → ignition kick → N2 spool curve →
  shutdown, including aborted starts.
- **Multi-engine support:** tailored profiles for jet and piston powerplants, up to
  4 engines, with a dedicated crank-pulse model for pistons where N2 is unavailable.

### 5. 🌊 True control-surface animation tracking
- **Physical surface movement:** flap and slat vibration is tied to actual aerodynamic
  surface displacement, not the cockpit switch position.
- **Flight-envelope dynamics:** progressive pulsation during steep turns, stall buffet,
  and overspeed, with the overspeed threshold taken from the simulator's own moving
  barber pole (or a manual value you set).
- **Spoiler symmetry checks:** airflow rumble only fires when the panels are actually
  symmetric, so roll spoilers in a turn do not trigger it.

### 6. 🤖 Automation & profile management
- **Automatic game detection:** MSFS, X-Plane, and War Thunder links run side by side;
  whichever simulator is live claims the hardware, and the GUI follows. A manual override
  is one click away in **Options**.
- **Automatic aircraft detection:** the current aircraft is read live and its tactile
  preset applied — the same named-profile system works across all three simulators.
- **Modern GUI (egui):** lightweight, responsive interface with fine-grained sliders,
  per-aircraft profiles, a collapsible Live Monitor, tray integration with
  "close to tray", a single-instance guard, and an English/Russian language switch.

---

## 🎮 Simulator support in detail

### Microsoft Flight Simulator (SimConnect)

Eleven effects: **Ground Roll / Taxi Thump**, **Overspeed**, **Bank / Turbulence**,
**Spoiler Airflow**, **Flap Motor Hum**, **Stall**, **Gear Handle Bump**,
**Gear Transit & Doors**, **Gear Strut Compression (Touchdown)**,
**Engine Spool-up & Ignition (jet)**, and **Engine Start (piston)**.

Gear transit plays an 80 BPM three-beat rhythm scaled by how many struts are moving, and
adds a lock "slam" when the gear reaches its up or down stops.

### X-Plane 12 (UDP RREF)

X-Plane drives **the same eleven effects and the same settings** as MSFS — it is wired in
as a second telemetry source for the existing effects engine rather than as a separate
feature set, so everything you tune for one simulator carries across.

Telemetry arrives over X-Plane's built-in RREF protocol on UDP port 49000: the app
subscribes to the datarefs it needs and the simulator streams values back. No plugin, no
SDK, no files copied into `Resources/plugins`.

Because X-Plane splits gear geometry across two datarefs (`gear/deploy_ratio` and
`gear/tire_vertical_deflection_mtr`, the latter in metres) where MSFS uses a single
0–100 scale, the two are recombined into the scale the effects engine expects. The
"full compression" reference is a single tunable constant.

If a dataref name is wrong or unsupported by your aircraft, X-Plane silently ignores the
subscription — so the app runs a self-check a few seconds after the first data arrives and
logs every dataref that never reported. Check `AuroraVibra.log` if an effect stays quiet.

### War Thunder (HTTP telemetry)

Eight effects: **Weapon 1** and **Weapon 2** gunfire, **Stall buffet**,
**Overspeed (Vne)**, **Gear Overspeed**, **Flaps**, **Gear Transit & Doors**, and
**Engine Start / Stop**.

- **Gunfire texture:** each weapon group is a software-PWM generator (carrier frequency,
  duty cycle, per-cycle amplitude jitter, attack ramp) tuned on real hardware. Weapon 1 and
  Weapon 2 are deliberately routed to different hands so two gun types feel distinct.
- **Adaptive ammo tracking:** many aircraft expose no usable `weapon1..4` trigger keys at
  all. For those, firing is inferred from decreasing ammo counters, and a two-bucket
  learner works out on its own which counters belong to which weapon group.
- **Per-vehicle Vne table** covering ~1300 vehicles, plus flap- and gear-position-aware
  overspeed windows.
- **Built-in session recorder** (Options) captures raw telemetry sessions for tuning and
  regression tests.

---

## 🛠️ System requirements & building

- **OS:** Windows 10 / 11
- **Simulators:** Microsoft Flight Simulator (2020 / 2024) via SimConnect, X-Plane 12 via
  UDP RREF, War Thunder via its local telemetry server
- **Supported hardware:** FFB & vibration-capable flight controls — WinWing Combat
  Joystick R, Orion joystick and throttle, Ursa Minor throttle, and similar

To build from source (requires the **Rust** toolchain, 1.88 or newer):

```bash
cargo build --release --bin aurora-vibra --features app
```

The build embeds a copy of `SimConnect.dll` and extracts it at runtime as a last-resort
fallback, so the binary works even where no SimConnect client library is installed — see
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) for the licensing status of that file.

The `src/bin/` directory holds manual diagnostic benches (motor sweeps, HID frame
captures, SimConnect probes). They are gated behind a separate feature so ordinary
builds stay lean:

```bash
cargo run --features dev-tools --bin test_motor_gui
```

The War Thunder telemetry reconnaissance tools live behind their own feature:

```bash
cargo run --features wt-probe --bin wt_probe_gui
```

---

## 📌 Acknowledgments

Special thanks to [Rodrigo Troncoso](https://github.com/rtroncoso) for creating the
original base repository ([ursa-minor-ffb](https://github.com/rtroncoso/ursa-minor-ffb)),
its foundational architecture, and the open-source codebase that made this extended and
enhanced utility possible.

Aurora Vibra is a **fork** of [ursa-minor-ffb](https://github.com/rtroncoso/ursa-minor-ffb),
used under the MIT License. Portions of this software are Copyright (c) 2025 Rodrigo Troncoso.

**X-Plane telemetry** is based on
[**rswilem/winctrl-xplane-plugin**](https://github.com/rswilem/winctrl-xplane-plugin) by
[Ramon Wilem](https://github.com/rswilem) — the source of the X-Plane dataref set Aurora
Vibra reads and of the approach to working with them (runtime dataref type detection,
handle caching, aircraft identification by probing for signature datarefs, and
send-only-on-change gating). That project is an in-process XPLM plugin driving WinWing
panels directly; Aurora Vibra applies the same dataref knowledge from outside the
simulator over the RREF protocol. Well worth a look if you own WinWing FCU/EFIS, MCP,
or CDU hardware.

## ☕ Support the Project

We'd be happy to receive your donation towards the project's development — adding new
features and adapting support for custom aircraft models:

**USDT (TRC-20):** `TSP24RnqTRzA215LNDzWrNQBawWpR9z5YD`

**BTC:** `bc1p5txluxsen8uqhy0k3j0v9s6afemt5zkyftzjv4asc5uh3lw44u7snkplr5`

**YooMoney:** [410011348629282](https://yoomoney.ru/to/410011348629282)

## 📜 License

Distributed under the [MIT License](LICENSE), which covers the Aurora Vibra source code
and the upstream code it derives from.

It does **not** cover third-party components. In particular, released binaries embed
Microsoft's proprietary `SimConnect.dll` as a fallback for machines where no SimConnect
client library is present; that file is governed by the
[MSFS SDK EULA](https://docs.flightsimulator.com/msfs2024/html/1_Introduction/SDK_EULA.htm),
not by this project's license. See [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) for
the full statement, including how to build without it.
