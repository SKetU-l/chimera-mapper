rust/README.md
# chimera-mapper

HID side-button mapper for **Kreo Chimera V1** mouse.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/SKetU-l/chimera-mapper/main/scripts/install.sh | bash
```

The installer will:
- Download the latest version or build from source
- Set up auto-start on system boot
- Configure the app to run in the background

---

## Uninstall

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/SKetU-l/chimera-mapper/main/scripts/uninstall.sh)
```

This removes the app and auto-start configuration.

---

## Usage

```bash
# List all connected HID devices
chimera-mapper list

# Run the mapper (daemon mode)
chimera-mapper run

# Dump raw HID reports (debugging)
chimera-mapper dump

# Show help
chimera-mapper --help
```

---

## How it works

`chimera-mapper` runs as a small HID event translation layer:

1. **Device selection** – Uses saved profile when available, otherwise auto-detects a likely Kreo Chimera V1 HID interface
2. **Report reading** – Continuously reads HID input reports from the selected interface
3. **Button-state parsing** – Inspects configured report byte + bit masks for side-button states
4. **Transition detection** – Tracks previous/current state to detect only press/release transitions
5. **Action emission** – Emits mapped actions for `Forward` and `Back`
6. **Recovery loop** – On disconnect, retries until device is available again and resumes with clean state handling after reconnect

---

## Status

This project is in active development and is **not ready for download or regular use yet**.  
The first goal is a stable base version.

Current development/testing device: **Kreo Chimera V1**

Other brands/models are **not tested yet**, so compatibility is **not confirmed**.

---

## Roadmap (after stable base version)

- [ ] Custom key mapping (allow users to set custom shortcuts or options instead of just Forward/Back)
- [ ] Graphical User Interface (GUI)
- [ ] Better diagnostics and troubleshooting logs
- [ ] Cleaner end-user setup/usage documentation

---

## License

Copyright (c) 2026 SKetU

Licensed under **GPL-3.0-or-later**.

- You may use, modify, and redistribute this project under GPL terms.
- If you convey (distribute) modified versions, you must license them under GPL and provide corresponding source code.
- Full terms: [LICENSE](./LICENSE)