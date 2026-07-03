use crate::config::AppResult;
use crate::hid::Transition;

pub struct Emitter;

impl Emitter {
    pub fn new(_name: &str) -> AppResult<Self> {
        Err("this project currently supports only Linux and macOS".into())
    }

    pub fn emit(&mut self, _transition: &Transition) -> AppResult<()> {
        Err("this project currently supports only Linux and macOS".into())
    }
}

pub struct SourceGrab;

impl SourceGrab {
    pub fn acquire(_vid: Option<u16>, _pid: Option<u16>) -> AppResult<Option<Self>> {
        Ok(None)
    }
}
