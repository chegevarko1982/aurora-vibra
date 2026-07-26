# Changelog

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
