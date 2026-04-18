# chimera-mapper

HID side-button mapper for **Kreo Chimera V1** mouse.

## Status

This project is in active development and is **not ready for download or regular use yet**.  
The first goal is a stable base version.

## Scope (right now)

Current development/testing device:

- **Kreo Chimera V1**

Other brands/models are **not tested yet**, so compatibility is **not confirmed**.

## Goal

Provide stable `Forward` / `Back` side-button mapping for Kreo Chimera V1 with predictable behavior during normal use and reconnection scenarios.

## How it works

`chimera-mapper` runs as a small HID event translation layer:

1. **Device selection**
   - Uses saved profile when available
   - Otherwise auto-detects a likely Kreo Chimera V1 HID interface
2. **Report reading**
   - Continuously reads HID input reports from the selected interface
3. **Button-state parsing**
   - Inspects configured report byte + bit masks for side-button states
4. **Transition detection**
   - Tracks previous/current state to detect only press/release transitions
5. **Action emission**
   - Emits mapped actions for `Forward` and `Back`
6. **Recovery loop**
   - On disconnect, retries until device is available again
   - Resumes with clean state handling after reconnect

In short: read HID reports → detect side-button transitions → emit mapped actions → recover on reconnect.

## Current priorities (before base release)

- [ ] Verify Linux behavior works properly end-to-end
- [x] Validate disconnect behavior while running
- [x] Validate reconnect behavior after interruptions
- [x] Validate wired ↔ wireless switching behavior
- [x] Ensure no stuck button state after reconnect/switch
- [ ] Fix issues found during these stability checks

## Non-goals for now

- Broad multi-brand mouse support
- Feature expansion before base stability
- Claiming compatibility beyond tested hardware

## Roadmap (after stable base version)

- Custom key mapping (allow users to set custom shortcuts or options instead of just Forward/Back)
- Graphical User Interface (GUI)
- Better diagnostics and troubleshooting logs
- Cleaner end-user setup/usage documentation

## License

Copyright (c) 2026 SKetU

Licensed under **GPL-3.0-or-later**.

- You may use, modify, and redistribute this project under GPL terms.
- If you convey (distribute) modified versions, you must license them under GPL and provide corresponding source code.
- Full terms: [LICENSE](./LICENSE)