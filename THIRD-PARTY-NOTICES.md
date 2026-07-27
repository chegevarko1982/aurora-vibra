# Third-Party Notices

Aurora Vibra is distributed under the [MIT License](LICENSE). That license covers
the Aurora Vibra source code and the upstream code it derives from — **it does not
cover the third-party components listed below**, which remain under their own terms.

---

## Microsoft Flight Simulator SimConnect (`SimConnect.dll`)

`lib/SimConnect.dll` is a proprietary Microsoft component originating from the
Microsoft Flight Simulator SDK. It is **not** part of Aurora Vibra, is **not**
covered by Aurora Vibra's MIT License, and is not modified by this project.

Its use and redistribution are governed by the
[Microsoft Flight Simulator SDK End User License Agreement](https://docs.flightsimulator.com/msfs2024/html/1_Introduction/SDK_EULA.htm),
not by any license granted here. Nothing in this repository grants you rights to
that file.

### How it is used

The file is embedded into `aurora-vibra.exe` at compile time and written to the
system temporary directory at startup, but only as a **last-resort fallback**.
An existing `SimConnect.dll` is always preferred: the loader first tries a path
configured by the user, then the executable's own directory, then the normal
Win32 search path, and only then falls back to the embedded copy.

The reason a copy is carried at all is that on a machine with only MSFS 2024
installed there is no file under this name to find — the simulator keeps its own
copy as `SimConnect_internal.dll`, and `C:\Program Files\WindowsApps` is not
readable by ordinary processes.

### Provenance

Recorded honestly, because it cannot be established from the file itself:

| | |
|---|---|
| Size | 77,824 bytes |
| SHA-256 | `cae445bddaf35c6f75f01ed415cf708c61fc3a0cbc2412f577f768d82f143b30` |
| Authenticode signature | **None** — the file is not digitally signed |
| Version resource | **Absent** — no `CompanyName`, `LegalCopyright` or `FileVersion` |
| Origin in this repository | Inherited from upstream `ursa-minor-ffb`, commit `7a3013a` (2025-10-06) |
| Originating SDK version | **Unknown** — not recoverable from the binary or from repository history |

Because the file carries no signature and no version metadata, this project cannot
independently attest that it is an unmodified Microsoft binary. It is reproduced
here exactly as inherited from upstream.

### Redistribution status: unresolved

The MSFS SDK EULA permits distribution of "distributable code" but **does not define
that term**, and no Microsoft document identifies `SimConnect.dll` as redistributable.
Requests for an authoritative clarification have been filed on the official MSFS
DevSupport forum and remain **unanswered** as of 2026-07-27:

- [Permission to redistribute SimConnect.dll and Microsoft.FlightSimulator.SimConnect.dll](https://devsupport.flightsimulator.com/t/permission-to-redistribute-simconnect-dll-and-microsoft-flightsimulator-simconnect-dll/18087) (2026-06-22)
- [Redistribution rights for MSFS 2024 native SimConnect.dll](https://devsupport.flightsimulator.com/t/redistribution-rights-for-msfs-2024-native-simconnect-dll/18233) (2026-07-25)

This project makes **no claim** that redistribution is authorised. If Microsoft or
Asobo indicate that it is not permitted, the embedded copy will be removed.

If you are Microsoft or Asobo and want this file removed, please open an issue and
it will be done.

---

## Rust dependencies

All Rust crate dependencies are permissively licensed (MIT, Apache-2.0, or dual
MIT/Apache-2.0). For a full machine-readable inventory of the dependency tree and
its licenses:

```bash
cargo install cargo-about && cargo about generate about.hbs
```
