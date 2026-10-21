use crate::action::{Action, Key, Modifier, MouseButton};
use crate::config::AppResult;
use crate::hid::Transition;
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, EventField};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;
use core_graphics::sys::{CGEventRef, CGEventSourceRef};
use foreign_types::ForeignType;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreateMouseEvent(
        source: CGEventSourceRef,
        mouse_type: CGEventType,
        mouse_cursor_position: CGPoint,
        mouse_button: u32,
    ) -> CGEventRef;
    fn CGPreflightPostEventAccess() -> bool;
    fn CGRequestPostEventAccess() -> bool;
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOHIDCheckAccess(request_type: i32) -> i32;
    fn IOHIDRequestAccess(request_type: i32) -> bool;
}

const MAC_BUTTON_LEFT: u32 = 0;
const MAC_BUTTON_RIGHT: u32 = 1;
const MAC_BUTTON_MIDDLE: u32 = 2;
const MAC_BUTTON_BACK: u32 = 3;
const MAC_BUTTON_FORWARD: u32 = 4;
const HID_REQUEST_LISTEN_EVENT: i32 = 1;
const HID_ACCESS_GRANTED: i32 = 0;
const HID_ACCESS_UNKNOWN: i32 = 2;

pub struct Emitter {
    source: CGEventSource,
    log_events: bool,
}

pub struct SourceGrab;

impl SourceGrab {
    pub fn acquire(_vid: Option<u16>, _pid: Option<u16>) -> AppResult<Option<Self>> {
        Ok(None)
    }
}

#[derive(Clone, Copy, Debug)]
struct MouseEventSpec {
    event_type: CGEventType,
    button_number: u32,
}

fn mouse_event_spec(button: MouseButton, pressed: bool) -> MouseEventSpec {
    let (down, up, button_number) = match button {
        MouseButton::Left => (
            CGEventType::LeftMouseDown,
            CGEventType::LeftMouseUp,
            MAC_BUTTON_LEFT,
        ),
        MouseButton::Right => (
            CGEventType::RightMouseDown,
            CGEventType::RightMouseUp,
            MAC_BUTTON_RIGHT,
        ),
        MouseButton::Middle => (
            CGEventType::OtherMouseDown,
            CGEventType::OtherMouseUp,
            MAC_BUTTON_MIDDLE,
        ),
        MouseButton::Back => (
            CGEventType::OtherMouseDown,
            CGEventType::OtherMouseUp,
            MAC_BUTTON_BACK,
        ),
        MouseButton::Forward => (
            CGEventType::OtherMouseDown,
            CGEventType::OtherMouseUp,
            MAC_BUTTON_FORWARD,
        ),
    };

    MouseEventSpec {
        event_type: if pressed { down } else { up },
        button_number,
    }
}

fn create_mouse_event(
    source: &CGEventSource,
    spec: MouseEventSpec,
    location: CGPoint,
) -> Result<CGEvent, ()> {
    // core-graphics 0.25 only exposes enum variants for buttons 0 through 2.
    // Quartz accepts USB-order button numbers 3 through 31 in this argument.
    let event_ref = unsafe {
        CGEventCreateMouseEvent(
            source.as_ptr(),
            spec.event_type,
            location,
            spec.button_number,
        )
    };

    if event_ref.is_null() {
        Err(())
    } else {
        // CGEventCreateMouseEvent returns a retained event. CGEvent takes ownership here.
        Ok(unsafe { CGEvent::from_ptr(event_ref) })
    }
}

fn can_post_events() -> bool {
    unsafe { CGPreflightPostEventAccess() || CGRequestPostEventAccess() }
}

fn can_read_hid_events() -> bool {
    match unsafe { IOHIDCheckAccess(HID_REQUEST_LISTEN_EVENT) } {
        HID_ACCESS_GRANTED => true,
        HID_ACCESS_UNKNOWN => unsafe { IOHIDRequestAccess(HID_REQUEST_LISTEN_EVENT) },
        _ => false,
    }
}

fn modifier_to_flag(m: Modifier) -> CGEventFlags {
    match m {
        Modifier::Ctrl => CGEventFlags::CGEventFlagControl,
        Modifier::Shift => CGEventFlags::CGEventFlagShift,
        Modifier::Alt => CGEventFlags::CGEventFlagAlternate,
        Modifier::Meta => CGEventFlags::CGEventFlagCommand,
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
        LeftBracket  => 33,
        RightBracket => 30,
    }
}

impl Emitter {
    pub fn new(_name: &str) -> AppResult<Self> {
        if !can_post_events() {
            return Err(
                "macOS denied event posting; allow Chimera Mapper in System Settings > Privacy & Security > Accessibility, then restart the service"
                    .into(),
            );
        }
        if !can_read_hid_events() {
            return Err(
                "macOS denied HID input; allow Chimera Mapper in System Settings > Privacy & Security > Input Monitoring, then restart the service"
                    .into(),
            );
        }

        let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .map_err(|_| "failed to create macOS event source")?;
        Ok(Self {
            source,
            log_events: std::env::var_os("CHIMERA_MAPPER_LOG_EVENTS").is_some(),
        })
    }

    pub fn emit(&mut self, transition: &Transition) -> AppResult<()> {
        let pressed = transition.pressed;
        match &transition.action {
            Action::Keys { modifiers, key } => {
                let keycode = key_to_mac(*key);
                // Carry modifiers as flags on the key event itself; separate
                // modifier key events can be dropped or left stuck.
                let flags = modifiers
                    .iter()
                    .fold(CGEventFlags::empty(), |acc, &m| acc | modifier_to_flag(m));
                let ev = CGEvent::new_keyboard_event(self.source.clone(), keycode, pressed)
                    .map_err(|_| "failed to create macOS keyboard event")?;
                if !flags.is_empty() {
                    ev.set_flags(flags);
                }
                ev.post(CGEventTapLocation::HID);
            }
            Action::Mouse(btn) => {
                let location = CGEvent::new(self.source.clone())
                    .map_err(|_| "failed to read macOS pointer location")?
                    .location();
                let spec = mouse_event_spec(*btn, pressed);
                let event = create_mouse_event(&self.source, spec, location)
                    .map_err(|_| "failed to create macOS mouse event")?;
                event.set_integer_value_field(
                    EventField::MOUSE_EVENT_BUTTON_NUMBER,
                    i64::from(spec.button_number),
                );

                if self.log_events {
                    eprintln!(
                        "macOS mouse event: action={} phase={} type={:?} button={} tap=hid",
                        btn.canonical_name(),
                        if pressed { "down" } else { "up" },
                        spec.event_type,
                        spec.button_number,
                    );
                }

                event.post(CGEventTapLocation::HID);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auxiliary_buttons_use_macos_usb_order() {
        let cases = [
            (MouseButton::Back, true, CGEventType::OtherMouseDown, 3),
            (MouseButton::Back, false, CGEventType::OtherMouseUp, 3),
            (MouseButton::Forward, true, CGEventType::OtherMouseDown, 4),
            (MouseButton::Forward, false, CGEventType::OtherMouseUp, 4),
        ];

        for (button, pressed, event_type, button_number) in cases {
            let spec = mouse_event_spec(button, pressed);
            assert_eq!(spec.event_type as u32, event_type as u32);
            assert_eq!(spec.button_number, button_number);
        }
    }

    #[test]
    fn auxiliary_event_contains_native_button_number() {
        let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState).unwrap();

        for button in [MouseButton::Back, MouseButton::Forward] {
            for pressed in [true, false] {
                let spec = mouse_event_spec(button, pressed);
                let event = create_mouse_event(&source, spec, CGPoint::new(0.0, 0.0)).unwrap();
                assert_eq!(event.get_type() as u32, spec.event_type as u32);
                assert_eq!(
                    event.get_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER),
                    i64::from(spec.button_number),
                );
            }
        }
    }
}
