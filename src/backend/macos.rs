use crate::action::{Action, Key, Modifier, MouseButton};
use crate::config::AppResult;
use crate::hid::Transition;
use core_graphics::event::{CGEvent, CGEventTapLocation, CGEventType, CGMouseButton, EventField};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

pub struct Emitter {
    source: CGEventSource,
}

pub struct SourceGrab;

impl SourceGrab {
    pub fn acquire(_vid: Option<u16>, _pid: Option<u16>) -> AppResult<Option<Self>> {
        Ok(None)
    }
}

variant_map! {
    fn modifier_to_mac(Modifier) -> u16 {
        Ctrl  => 59,  // kVK_Control
        Shift => 56,  // kVK_Shift
        Alt   => 58,  // kVK_Option
        Meta  => 55,  // kVK_Command
    }
}

variant_map! {
    fn key_to_mac(Key) -> u16 {
        A => 0,  B => 11, C => 8,  D => 2,  E => 14,
        F => 3,  G => 5,  H => 4,  I => 34, J => 38,
        K => 40, L => 37, M => 46, N => 45, O => 31,
        P => 35, Q => 12, R => 15, S => 1,  T => 17,
        U => 32, V => 9,  W => 13, X => 7,  Y => 16,
        Z => 6,
        Num0 => 29, Num1 => 18, Num2 => 19, Num3 => 20,
        Num4 => 21, Num5 => 23, Num6 => 22, Num7 => 26,
        Num8 => 28, Num9 => 25,
        F1  => 122, F2  => 120, F3  => 99,  F4  => 118,
        F5  => 96,  F6  => 97,  F7  => 98,  F8  => 100,
        F9  => 101, F10 => 109, F11 => 103, F12 => 111,
        Enter     => 36,
        Space     => 49,
        Tab       => 48,
        Backspace => 51,
        Escape    => 53,
        Delete    => 117,
        Insert    => 114, // Help/Insert key
        Home      => 115,
        End       => 119,
        PageUp    => 116,
        PageDown  => 121,
        Left      => 123,
        Right     => 124,
        Up        => 126,
        Down      => 125,
    }
}

impl Emitter {
    pub fn new(_name: &str) -> AppResult<Self> {
        let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .map_err(|_| "failed to create macOS event source")?;
        Ok(Self { source })
    }

    pub fn emit(&mut self, transition: &Transition) -> AppResult<()> {
        let pressed = transition.pressed;
        match &transition.action {
            Action::Keys { modifiers, key } => {
                let keycode = key_to_mac(*key);
                if pressed {
                    for &m in modifiers {
                        let ev = CGEvent::new_keyboard_event(
                            self.source.clone(),
                            modifier_to_mac(m),
                            true,
                        )
                        .map_err(|_| "failed to create macOS keyboard event")?;
                        ev.post(CGEventTapLocation::HID);
                    }
                    let ev = CGEvent::new_keyboard_event(self.source.clone(), keycode, true)
                        .map_err(|_| "failed to create macOS keyboard event")?;
                    ev.post(CGEventTapLocation::HID);
                } else {
                    let ev = CGEvent::new_keyboard_event(self.source.clone(), keycode, false)
                        .map_err(|_| "failed to create macOS keyboard event")?;
                    ev.post(CGEventTapLocation::HID);
                    for &m in modifiers.iter().rev() {
                        let ev = CGEvent::new_keyboard_event(
                            self.source.clone(),
                            modifier_to_mac(m),
                            false,
                        )
                        .map_err(|_| "failed to create macOS keyboard event")?;
                        ev.post(CGEventTapLocation::HID);
                    }
                }
            }
            Action::Mouse(btn) => {
                let location = CGEvent::new(self.source.clone())
                    .map_err(|_| "failed to read macOS pointer location")?
                    .location();

                let (event_type, button_type, button_number) = match btn {
                    MouseButton::Left => (
                        if pressed {
                            CGEventType::LeftMouseDown
                        } else {
                            CGEventType::LeftMouseUp
                        },
                        CGMouseButton::Left,
                        0_i64,
                    ),
                    MouseButton::Right => (
                        if pressed {
                            CGEventType::RightMouseDown
                        } else {
                            CGEventType::RightMouseUp
                        },
                        CGMouseButton::Right,
                        1_i64,
                    ),
                    MouseButton::Middle => (
                        if pressed {
                            CGEventType::OtherMouseDown
                        } else {
                            CGEventType::OtherMouseUp
                        },
                        CGMouseButton::Center,
                        2_i64,
                    ),
                    MouseButton::Back => (
                        if pressed {
                            CGEventType::OtherMouseDown
                        } else {
                            CGEventType::OtherMouseUp
                        },
                        CGMouseButton::Center,
                        3_i64,
                    ),
                    MouseButton::Forward => (
                        if pressed {
                            CGEventType::OtherMouseDown
                        } else {
                            CGEventType::OtherMouseUp
                        },
                        CGMouseButton::Center,
                        4_i64,
                    ),
                };

                let event = CGEvent::new_mouse_event(
                    self.source.clone(),
                    event_type,
                    location,
                    button_type,
                )
                .map_err(|_| "failed to create macOS mouse event")?;
                event.set_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER, button_number);
                event.post(CGEventTapLocation::HID);
            }
        }
        Ok(())
    }
}
