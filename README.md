# Aurora Vibra

A Rust-based flight simulator add-on.

## Overview
This project is a flight simulator add-on that interfaces with external hardware and provides various utility functions. The code is written in Rust and targets Windows platforms.

## Key Components
- Main entry point in `src/main.rs`
- Various modules including:
  - `src/types.rs` - Data structures
  - `src/ui.rs` - User interface components
  - `src/settings.rs` - Configuration management
  - `src/tray.rs` - System tray integration
  - `src/updater.rs` - Update management
  - `src/sim/` - Simulation logic
- Build scripts and supporting infrastructure

## Build System
- Uses Cargo as the build system
- Includes DLL dependencies and platform-specific resources
- Supports Windows-specific features

## Files
- Source code: `src/`
- Tests: `tests/`
- Assets: `assets/`
- Platform resources: `windows/`
- Configuration: `Cargo.toml`

## Technical Details
The project appears to be a complex Windows-based flight simulator add-on with native code, system tray integration, and hardware interface capabilities. It uses Rust's FFI capabilities to interface with external systems.

What would you like me to help you with? I can:
- Analyze existing code structure and identify improvements
- Help debug or fix existing issues
- Add new functionality to modules
- Review and improve code quality
- Test or build the project

Please let me know what specific task you'd like me to focus on!