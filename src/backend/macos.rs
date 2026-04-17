use crate::hid::{ActionKind, Transition};
use crate::config::AppResult;
use core_graphics::event::{
    CGEvent, CGEventTapLocation, CGEventType, CGMouseButton, EventField,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

pub struct Emitter {
    source: CGEventSource,
}

impl Emitter {
    pub fn new(_name: &str) -> AppResult<Self> {
        let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .map_err(|_| "failed to create macOS event source")?;
        Ok(Self { source })
    }

    pub fn emit(&mut self, transition: Transition) -> AppResult<()> {
        let (event_type, button_number) = match transition.kind {
            ActionKind::Forward => (
                if transition.pressed { CGEventType::OtherMouseDown } else { CGEventType::OtherMouseUp },
                4_i64,
            ),
            ActionKind::Back => (
                if transition.pressed { CGEventType::OtherMouseDown } else { CGEventType::OtherMouseUp },
                3_i64,
            ),
        };

        let location = CGEvent::new(self.source.clone())
            .map_err(|_| "failed to read macOS pointer location")?
            .location();
        let event = CGEvent::new_mouse_event(
            self.source.clone(),
            event_type,
            location,
            CGMouseButton::Center,
        )
        .map_err(|_| "failed to create macOS mouse event")?;
        event.set_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER, button_number);
        event.post(CGEventTapLocation::HID);
        Ok(())
    }
}
