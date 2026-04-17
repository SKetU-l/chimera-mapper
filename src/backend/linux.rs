use crate::hid::{ActionKind, Transition};
use crate::config::AppResult;
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
            KeyCode::KEY_FORWARD,
            KeyCode::KEY_BACK,
        ]);
        let device = VirtualDevice::builder()?
            .name(name.as_bytes())
            .with_keys(&keys)?
            .build()?;
        Ok(Self { device })
    }

    pub fn emit(&mut self, transition: Transition) -> AppResult<()> {
        let value = i32::from(transition.pressed);
        let events = match transition.kind {
            ActionKind::Forward => [
                KeyEvent::new(KeyCode::BTN_EXTRA, value).into(),
                KeyEvent::new(KeyCode::KEY_FORWARD, value).into(),
            ],
            ActionKind::Back => [
                KeyEvent::new(KeyCode::BTN_SIDE, value).into(),
                KeyEvent::new(KeyCode::KEY_BACK, value).into(),
            ],
        };
        self.device.emit(&events)?;
        Ok(())
    }
}
