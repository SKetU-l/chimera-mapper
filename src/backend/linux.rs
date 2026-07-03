use crate::action::{Action, Key, Modifier, MouseButton};
use crate::config::AppResult;
use crate::hid::Transition;
use evdev::event_variants::KeyEvent;
use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, Device as RawDevice, KeyCode, RelativeAxisCode};

pub struct Emitter {
    device: VirtualDevice,
}

pub struct SourceGrab {
    _device: RawDevice,
    path: std::path::PathBuf,
}

impl SourceGrab {
    pub fn acquire(vid: Option<u16>, pid: Option<u16>) -> AppResult<Option<Self>> {
        let (Some(vid), Some(pid)) = (vid, pid) else {
            return Ok(None);
        };

        for path in evdev_devices() {
            let Ok(mut dev) = RawDevice::open(&path) else {
                continue;
            };
            let id = dev.input_id();
            if id.vendor() != vid || id.product() != pid {
                continue;
            }

            if !is_safe_to_grab(&dev) {
                eprintln!(
                    "skipping evdev node {} (vid=0x{:04x} pid=0x{:04x}): composite pointer/click interface, grabbing would freeze the mouse",
                    path.display(),
                    vid,
                    pid
                );
                continue;
            }

            match dev.grab() {
                Ok(()) => {
                    eprintln!(
                        "grabbed isolated side-button evdev node {} (vid=0x{:04x} pid=0x{:04x}) to suppress native passthrough",
                        path.display(),
                        vid,
                        pid
                    );
                    return Ok(Some(Self { _device: dev, path }));
                }
                Err(e) => {
                    eprintln!(
                        "warning: could not grab {} (in use by another exclusive client?): {}",
                        path.display(),
                        e
                    );
                }
            }
        }
        Ok(None)
    }
}

fn is_safe_to_grab(dev: &RawDevice) -> bool {
    let keys = dev.supported_keys();
    let has_side =
        keys.is_some_and(|k| k.contains(KeyCode::BTN_SIDE) || k.contains(KeyCode::BTN_EXTRA));
    if !has_side {
        return false;
    }
    let has_primary_click = keys.is_some_and(|k| {
        k.contains(KeyCode::BTN_LEFT)
            || k.contains(KeyCode::BTN_RIGHT)
            || k.contains(KeyCode::BTN_MIDDLE)
    });
    if has_primary_click {
        return false;
    }
    let has_movement = dev.supported_relative_axes().is_some_and(|r| {
        r.contains(RelativeAxisCode::REL_X) || r.contains(RelativeAxisCode::REL_Y)
    });
    !has_movement
}

impl Drop for SourceGrab {
    fn drop(&mut self) {
        eprintln!("released source evdev grab on {}", self.path.display());
    }
}

fn evdev_devices() -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir("/dev/input") else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with("event") {
            continue;
        }
        out.push(entry.path());
    }
    out
}

variant_map! {
    fn modifier_to_evdev(Modifier) -> KeyCode {
        Ctrl  => KeyCode::KEY_LEFTCTRL,
        Shift => KeyCode::KEY_LEFTSHIFT,
        Alt   => KeyCode::KEY_LEFTALT,
        Meta  => KeyCode::KEY_LEFTMETA,
    }
}

variant_map! {
    fn key_to_evdev(Key) -> KeyCode {
        A => KeyCode::KEY_A, B => KeyCode::KEY_B, C => KeyCode::KEY_C,
        D => KeyCode::KEY_D, E => KeyCode::KEY_E, F => KeyCode::KEY_F,
        G => KeyCode::KEY_G, H => KeyCode::KEY_H, I => KeyCode::KEY_I,
        J => KeyCode::KEY_J, K => KeyCode::KEY_K, L => KeyCode::KEY_L,
        M => KeyCode::KEY_M, N => KeyCode::KEY_N, O => KeyCode::KEY_O,
        P => KeyCode::KEY_P, Q => KeyCode::KEY_Q, R => KeyCode::KEY_R,
        S => KeyCode::KEY_S, T => KeyCode::KEY_T, U => KeyCode::KEY_U,
        V => KeyCode::KEY_V, W => KeyCode::KEY_W, X => KeyCode::KEY_X,
        Y => KeyCode::KEY_Y, Z => KeyCode::KEY_Z,
        Num0 => KeyCode::KEY_0, Num1 => KeyCode::KEY_1, Num2 => KeyCode::KEY_2,
        Num3 => KeyCode::KEY_3, Num4 => KeyCode::KEY_4, Num5 => KeyCode::KEY_5,
        Num6 => KeyCode::KEY_6, Num7 => KeyCode::KEY_7, Num8 => KeyCode::KEY_8,
        Num9 => KeyCode::KEY_9,
        F1  => KeyCode::KEY_F1,  F2  => KeyCode::KEY_F2,  F3  => KeyCode::KEY_F3,
        F4  => KeyCode::KEY_F4,  F5  => KeyCode::KEY_F5,  F6  => KeyCode::KEY_F6,
        F7  => KeyCode::KEY_F7,  F8  => KeyCode::KEY_F8,  F9  => KeyCode::KEY_F9,
        F10 => KeyCode::KEY_F10, F11 => KeyCode::KEY_F11, F12 => KeyCode::KEY_F12,
        Enter     => KeyCode::KEY_ENTER,
        Space     => KeyCode::KEY_SPACE,
        Tab       => KeyCode::KEY_TAB,
        Backspace => KeyCode::KEY_BACKSPACE,
        Escape    => KeyCode::KEY_ESC,
        Delete    => KeyCode::KEY_DELETE,
        Insert    => KeyCode::KEY_INSERT,
        Home      => KeyCode::KEY_HOME,
        End       => KeyCode::KEY_END,
        PageUp    => KeyCode::KEY_PAGEUP,
        PageDown  => KeyCode::KEY_PAGEDOWN,
        Left      => KeyCode::KEY_LEFT,
        Right     => KeyCode::KEY_RIGHT,
        Up        => KeyCode::KEY_UP,
        Down      => KeyCode::KEY_DOWN,
    }
}

variant_map! {
    fn mouse_to_evdev(MouseButton) -> KeyCode {
        Left    => KeyCode::BTN_LEFT,
        Right   => KeyCode::BTN_RIGHT,
        Middle  => KeyCode::BTN_MIDDLE,
        Back    => KeyCode::BTN_SIDE,
        Forward => KeyCode::BTN_EXTRA,
    }
}

impl Emitter {
    pub fn new(name: &str) -> AppResult<Self> {
        let mut keys = AttributeSet::<KeyCode>::new();
        for code in 0..=767 {
            keys.insert(KeyCode::new(code));
        }
        let mut rel = AttributeSet::<RelativeAxisCode>::new();
        rel.insert(RelativeAxisCode::REL_X);
        rel.insert(RelativeAxisCode::REL_Y);
        let device = VirtualDevice::builder()?
            .name(name.as_bytes())
            .with_keys(&keys)?
            .with_relative_axes(&rel)?
            .build()?;
        Ok(Self { device })
    }

    pub fn emit(&mut self, transition: &Transition) -> AppResult<()> {
        let pressed = transition.pressed;
        let mut events = Vec::new();
        match &transition.action {
            Action::Keys { modifiers, key } => {
                let keycode = key_to_evdev(*key);
                if pressed {
                    for &m in modifiers {
                        events.push(KeyEvent::new(modifier_to_evdev(m), 1).into());
                    }
                    events.push(KeyEvent::new(keycode, 1).into());
                } else {
                    events.push(KeyEvent::new(keycode, 0).into());
                    for &m in modifiers.iter().rev() {
                        events.push(KeyEvent::new(modifier_to_evdev(m), 0).into());
                    }
                }
            }
            Action::Mouse(btn) => {
                let keycode = mouse_to_evdev(*btn);
                events.push(KeyEvent::new(keycode, i32::from(pressed)).into());
            }
        }
        self.device.emit(&events[..])?;
        Ok(())
    }
}
