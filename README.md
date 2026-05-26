# chimera-mapper

HID side-button mapper for **Kreo Chimera V1** mouse.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/SKetU-l/chimera-mapper/main/scripts/install.sh | bash
```

The installer will:
- Build from source
- Set up auto-start on system boot
- Configure the app to run in the background

---

## Uninstall

```bash
curl -fsSL https://raw.githubusercontent.com/SKetU-l/chimera-mapper/main/scripts/uninstall.sh | bash
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

## Custom Key Mapping

By default, the side buttons are mapped to standard browser forward (`mouse_forward` / `btn_extra`) and backward (`mouse_back` / `btn_side`) actions. You can configure custom keyboard shortcuts or mouse events to be triggered instead.

### 1. Via Command Line Arguments

You can pass `--side-action` and `--extra-action` flags when running the mapper:

```bash
# Map side button to Ctrl+Shift+T, and extra button to Alt+Tab
chimera-mapper run --side-action "ctrl+shift+t" --extra-action "alt+tab"
```

### 2. Via Configuration File

Alternatively, you can save custom mapping actions in your config file (typically located at `~/.config/chimera-mapper/config.json`):

```json
{
  "profile": {
    "path": "/dev/hidraw1",
    "vid": 9354,
    "pid": 23370,
    "usage_page": 1,
    "usage": 2,
    "interface_number": 0,
    "mapping": {
      "button_byte": 1,
      "side_mask": 16,
      "extra_mask": 8,
      "side_action": "ctrl+shift+t",
      "extra_action": "mouse_back"
    }
  }
}
```

### Supported Format

An action can be:
- **A key combination**: One or more modifiers (`ctrl`, `shift`, `alt`/`option`, `meta`/`super`/`command`/`win`) separated by `+`, followed by a base key. Example: `"ctrl+alt+delete"`, `"shift+space"`, `"f5"`, `"esc"`, `"a"`.
- **A mouse button override**:
  - `mouse_left` / `btn_left`
  - `mouse_right` / `btn_right`
  - `mouse_middle` / `btn_middle`
  - `mouse_back` / `btn_side` / `back`
  - `mouse_forward` / `btn_extra` / `forward`

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

This project is ready for daily use on an early stage.
The first goal is a stable base version.

Current development/testing device: **Kreo Chimera V1**

Other brands/models are **not tested yet**, so compatibility is **not confirmed**.

> [!NOTE]
> Currently, the Hackintosh system that was used to test this on macOS has been removed. Consequently, the macOS version is quite untested.

---

## Roadmap (after stable base version)

- [x] Custom key mapping (allow users to set custom shortcuts or options instead of just Forward/Back)
- [ ] Graphical User Interface (GUI)
- [ ] Probably RGB lighting control

---

## License

Copyright (c) 2026 SKetU

Licensed under **GPL-3.0-or-later**.

- You may use, modify, and redistribute this project under GPL terms.
- If you convey (distribute) modified versions, you must license them under GPL and provide corresponding source code.
- Full terms: [LICENSE](./LICENSE)