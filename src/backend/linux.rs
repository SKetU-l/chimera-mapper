use crate::config::AppResult;
use crate::hid::{ActionKind, Transition};
use evdev::event_variants::KeyEvent;
use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, KeyCode};

pub struct Emitter {
    device: VirtualDevice,
}

impl Emitter {
    pub fn new(name: &str) -> AppResult<Self> {
        let keys = AttributeSet::<KeyCode>::from_iter([
            KeyCode::BTN_EXTRA,
            KeyCode::BTN_SIDE,
        ]);
        let device = VirtualDevice::builder()?
            .name(name.as_bytes())
            .with_keys(&keys)?
            .build()?;
        Ok(Self { device })
    }

    pub fn emit(&mut self, transition: Transition) -> AppResult<()> {
        let value = i32::from(transition.pressed);
        let events: Vec<_> = match transition.kind {
            ActionKind::Forward => vec![KeyEvent::new(KeyCode::BTN_EXTRA, value).into()],
            ActionKind::Back => vec![KeyEvent::new(KeyCode::BTN_SIDE, value).into()],
        };
        self.device.emit(&events[..])?;
        Ok(())
    }
}
