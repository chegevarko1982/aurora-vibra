<div align="center">
<img src="assets/aurora-vibra-logo.png" alt="Aurora Vibra logo" width="220">

# ✈️ Aurora Vibra
### Advanced Flight Simulator Tactile & FFB Hub

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Platform: Windows](https://img.shields.io/badge/Platform-Windows-lightgrey.svg)]()
[![Built with Rust](https://img.shields.io/badge/Built%20with-Rust-orange.svg)]()

*Next-generation tactile feedback (force feedback / rumble) and telemetry utility for Microsoft Flight Simulator (MSFS).*

</div>

---

## 🌟 Overview

An advanced control hub for tactile feedback and physical response designed for flight sticks, yokes, and throttles (including the WinWing ecosystem). This utility overcomes default software limitations and delivers an immersive, realistic feel for both heavy jetliners and general aviation aircraft.

---

## 🚀 Key Features & Advantages

### 1. 🧩 Native Support for Complex High-Fidelity Add-ons (PMDG, MADDOG X, TFDi)
- **Overcoming vendor limitations:** dedicated telemetry handling for complex study-level add-ons (**PMDG 737 / 777**, **Leonardo MADDOG X**, **TFDi MD-11**) where standard vendor tools either lack support or suffer from telemetry desync.
- **Lag-free sync:** direct reading of custom internal variables (L:Vars and SDK data areas) keeps physical vibration, cockpit instruments, and sound modules in sync.

### 2. 🛬 Progressive Runway & Taxi Physics (Taxi & Takeoff Thump)
- **Seamless speed blending:** an advanced mathematical model smoothly transitions individual runway-joint thumps during taxi into a continuous, dense rumble as the aircraft accelerates down the runway.
- **Independent gear struts:** individual load and compression processing for nose and main landing gear — a distinct tactile sense of main-gear touchdown followed by the nose gear lowering onto the centerline.

### 3. 🎛️ Multi-Device Routing & Asymmetric Throttle "Ping-Pong"
- **Flexible channel addressing:** route any tactile effect (gear thumps, engine rumble, flap actuation) independently to the flight stick, the throttle unit, or both.
- **Hydraulic mechanics simulation:** alternating vibration logic between throttle motors, mimicking the tactile feel of hydraulic pumps and electric actuators.

### 4. 🔥 Detailed Engine Start & Turbine Telemetry
- **Spool-up tracking (N2):** monitors real high-pressure spool (N2) acceleration from 0% through starter cutout, dynamically shaping vibration at fuel ignition.
- **Multi-engine support:** tailored profiles for jet and piston powerplants, up to 4 engines.

### 5. 🌊 True Control-Surface Animation Tracking
- **Physical surface movement:** flap and slat vibration is tied to actual aerodynamic surface displacement, not the cockpit switch position.
- **Flight-envelope dynamics:** progressive pulsation during steep turns, stall buffet, and overspeed.

### 6. 🤖 Automation & Profile Management
- **Automatic aircraft detection:** reads SimConnect variables in real time to identify the aircraft and apply the right tactile preset.
- **Modern GUI (egui):** lightweight, responsive interface with fine-grained sliders, per-aircraft profiles, and an English/Russian language switch.

---

## 🛠️ System Requirements & Building

- **OS:** Windows 10 / 11
- **Simulator:** Microsoft Flight Simulator (MSFS 2020 / 2024) via SimConnect
- **Supported hardware:** FFB & vibration-capable flight controls (WinWing joystick & throttle, etc.)

To build from source (requires the **Rust** toolchain):

```bash
cargo build --release --bin aurora-vibra --features app
```

---

## 📌 Acknowledgments

Special thanks to [Rodrigo Troncoso](https://github.com/rtroncoso) for creating the original base repository ([ursa-minor-ffb](https://github.com/rtroncoso/ursa-minor-ffb)), its foundational architecture, and the open-source codebase that made this extended and enhanced utility possible.

## ☕ Support the Project

We'd be happy to receive your donation towards the project's development — adding new features and adapting support for custom aircraft models:

**BTC:** `bc1p5txluxsen8uqhy0k3j0v9s6afemt5zkyftzjv4asc5uh3lw44u7snkplr5`

**YooMoney:** [410011348629282](https://yoomoney.ru/to/410011348629282)

## 📜 License

Distributed under the [MIT License](LICENSE).
