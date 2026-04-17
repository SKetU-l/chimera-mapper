use crate::hid::Transition;
use crate::config::AppResult;

pub struct Emitter;

impl Emitter {
    pub fn new(_name: &str) -> AppResult<Self> {
        Err("this project currently supports only Linux and macOS".into())
    }

    pub fn emit(&mut self, _transition: Transition) -> AppResult<()> {
        Err("this project currently supports only Linux and macOS".into())
    }
}
