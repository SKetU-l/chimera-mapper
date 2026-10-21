//! On-device configuration protocol for the Kreo Chimera V1 (Telink TLSR8278).
//!
//! Every command is an 8-byte feature report `07 CMD p0..p5` on the vendor
//! collection (usage page 0xFF01). Reads are `07 (0x80|CMD)` followed by a
//! GET_FEATURE whose second byte echoes the query. Bulk data (button table,
//! macros) goes out as 32-byte output reports after a feature-report header.
//! There is no checksum. See PROTOCOL.md for how this was recovered.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::action::{Action, Key, Modifier, MouseButton};

pub const REPORT_ID: u8 = 0x07;
pub const VENDOR_USAGE_PAGE: u16 = 0xFF01;
pub const FEATURE_LEN: usize = 8;
pub const OUTPUT_LEN: usize = 33;
pub const OUTPUT_PAYLOAD: usize = OUTPUT_LEN - 1;

pub const CMD_POLLING: u8 = 0x01;
/// Active DPI stage (0-based). Applies immediately, no commit needed.
pub const CMD_DPI_STAGE: u8 = 0x02;
pub const CMD_LED: u8 = 0x03;
pub const CMD_DPI: u8 = 0x04;
pub const CMD_BUTTONS: u8 = 0x05;
pub const CMD_RAPID_FIRE: u8 = 0x06;
pub const CMD_MACRO: u8 = 0x07;
/// Read-only status block: battery %, power state, polling code, …
/// The vendor tool never uses it; decoded from live readings.
pub const CMD_STATUS: u8 = 0x09;
/// Opens a write session. Sent first by the vendor tool.
pub const CMD_BEGIN: u8 = 0xF5;
/// Persists to flash and reboots the mouse (it re-enumerates on USB).
pub const CMD_COMMIT: u8 = 0xFA;
pub const QUERY: u8 = 0x80;

pub const DPI_STAGES: usize = 6;
pub const DPI_MIN: u32 = 200;
pub const DPI_MAX: u32 = 10_000;
pub const DPI_STEP: u32 = 100;

pub const BUTTON_COUNT: usize = 6;
const BUTTON_TABLE_LEN: usize = 64;

pub const MACRO_SLOTS: u8 = 8;
pub const MACRO_PACKETS: u8 = 9;
const MACRO_BLOB_LEN: usize = MACRO_PACKETS as usize * OUTPUT_PAYLOAD;
pub const MACRO_MAX_EVENTS: usize = (MACRO_BLOB_LEN - 4) / 4;

pub fn feature(cmd: u8, payload: &[u8]) -> [u8; FEATURE_LEN] {
    let mut frame = [0u8; FEATURE_LEN];
    frame[0] = REPORT_ID;
    frame[1] = cmd;
    let n = payload.len().min(FEATURE_LEN - 2);
    frame[2..2 + n].copy_from_slice(&payload[..n]);
    frame
}

pub fn output(chunk: &[u8]) -> [u8; OUTPUT_LEN] {
    let mut frame = [0u8; OUTPUT_LEN];
    frame[0] = REPORT_ID;
    let n = chunk.len().min(OUTPUT_PAYLOAD);
    frame[1..1 + n].copy_from_slice(&chunk[..n]);
    frame
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PollingRate {
    Hz125,
    Hz250,
    Hz500,
    Hz1000,
}

impl PollingRate {
    pub fn code(self) -> u8 {
        match self {
            PollingRate::Hz125 => 8,
            PollingRate::Hz250 => 4,
            PollingRate::Hz500 => 2,
            PollingRate::Hz1000 => 1,
        }
    }

    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            8 => Some(PollingRate::Hz125),
            4 => Some(PollingRate::Hz250),
            2 => Some(PollingRate::Hz500),
            1 => Some(PollingRate::Hz1000),
            _ => None,
        }
    }

    pub fn hz(self) -> u32 {
        1000 / self.code() as u32
    }
}

impl FromStr for PollingRate {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().trim_end_matches("hz").trim_end_matches("Hz") {
            "125" => Ok(PollingRate::Hz125),
            "250" => Ok(PollingRate::Hz250),
            "500" => Ok(PollingRate::Hz500),
            "1000" => Ok(PollingRate::Hz1000),
            other => Err(format!("unsupported polling rate {other:?}; use 125, 250, 500 or 1000")),
        }
    }
}

impl fmt::Display for PollingRate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} Hz", self.hz())
    }
}

const LED_MODES: [&str; 6] = ["off", "neon", "breathing", "flash", "lightshow", "ambilight"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LedMode(pub u8);

impl LedMode {
    pub fn name(self) -> Option<&'static str> {
        LED_MODES.get(self.0 as usize).copied()
    }
}

impl FromStr for LedMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim().to_lowercase();
        if let Some(idx) = LED_MODES.iter().position(|m| *m == s) {
            return Ok(LedMode(idx as u8));
        }
        // With the mouse's lighting switch on RGB, mode 0 shows a rainbow.
        if s == "rainbow" {
            return Ok(LedMode(0));
        }
        match s.parse::<u8>() {
            Ok(n) if (n as usize) < LED_MODES.len() => Ok(LedMode(n)),
            _ => Err(format!(
                "unknown LED mode {s:?}; use one of {} or 0-{}",
                LED_MODES.join(", "),
                LED_MODES.len() - 1
            )),
        }
    }
}

impl fmt::Display for LedMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(name) => write!(f, "{name}"),
            None => write!(f, "unknown ({})", self.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
}

/// Decodes the `0x89` status reply. The percentage is a voltage estimate, so
/// it reads high while charging.
pub fn decode_battery(status: &[u8]) -> Option<Battery> {
    let (&percent, &power) = (status.first()?, status.get(1)?);
    if percent > 100 || !matches!(power, 1 | 2) {
        return None;
    }
    Some(Battery {
        percent,
        charging: power == 2,
    })
}

impl fmt::Display for Battery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}%", self.percent)?;
        if self.charging {
            write!(f, " (charging)")?;
        }
        Ok(())
    }
}

pub fn dpi_to_code(dpi: u32) -> Result<u8, String> {
    if !(DPI_MIN..=DPI_MAX).contains(&dpi) || dpi % DPI_STEP != 0 {
        return Err(format!(
            "DPI {dpi} out of range; use {DPI_MIN}-{DPI_MAX} in steps of {DPI_STEP}"
        ));
    }
    Ok((dpi / DPI_STEP - 2) as u8)
}

pub fn code_to_dpi(code: u8) -> u32 {
    (code as u32 + 2) * DPI_STEP
}

pub fn encode_dpi(stages: &[u32; DPI_STAGES]) -> Result<[u8; DPI_STAGES], String> {
    let mut out = [0u8; DPI_STAGES];
    for (slot, dpi) in out.iter_mut().zip(stages) {
        *slot = dpi_to_code(*dpi)?;
    }
    Ok(out)
}

pub fn decode_dpi(codes: &[u8]) -> [u32; DPI_STAGES] {
    let mut out = [0u32; DPI_STAGES];
    for (dpi, code) in out.iter_mut().zip(codes) {
        *dpi = code_to_dpi(*code);
    }
    out
}

/// Rapid-fire interval is sent as a little-endian count of 10 ms ticks.
pub fn encode_rapid_fire(ms: u32) -> Result<[u8; 2], String> {
    if ms < 10 || ms > 2550 || ms % 10 != 0 {
        return Err(format!("rapid-fire interval {ms} ms out of range; use 10-2550 in steps of 10"));
    }
    Ok(((ms / 10) as u16).to_le_bytes())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysicalButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
    Dpi,
}

impl PhysicalButton {
    pub const ALL: [PhysicalButton; BUTTON_COUNT] = [
        PhysicalButton::Left,
        PhysicalButton::Right,
        PhysicalButton::Middle,
        PhysicalButton::Back,
        PhysicalButton::Forward,
        PhysicalButton::Dpi,
    ];

    pub fn slot(self) -> usize {
        match self {
            PhysicalButton::Left => 0,
            PhysicalButton::Right => 1,
            PhysicalButton::Middle => 2,
            PhysicalButton::Back => 3,
            PhysicalButton::Forward => 4,
            PhysicalButton::Dpi => 5,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            PhysicalButton::Left => "left",
            PhysicalButton::Right => "right",
            PhysicalButton::Middle => "middle",
            PhysicalButton::Back => "back",
            PhysicalButton::Forward => "forward",
            PhysicalButton::Dpi => "dpi",
        }
    }

    pub fn default_binding(self) -> Binding {
        match self {
            PhysicalButton::Left => Binding::Mouse(MouseButton::Left),
            PhysicalButton::Right => Binding::Mouse(MouseButton::Right),
            PhysicalButton::Middle => Binding::Mouse(MouseButton::Middle),
            PhysicalButton::Back => Binding::Mouse(MouseButton::Back),
            PhysicalButton::Forward => Binding::Mouse(MouseButton::Forward),
            PhysicalButton::Dpi => Binding::DpiCycle,
        }
    }
}

impl FromStr for PhysicalButton {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "left" | "1" => Ok(PhysicalButton::Left),
            "right" | "2" => Ok(PhysicalButton::Right),
            "middle" | "wheel" | "3" => Ok(PhysicalButton::Middle),
            "back" | "side" | "4" => Ok(PhysicalButton::Back),
            "forward" | "extra" | "5" => Ok(PhysicalButton::Forward),
            "dpi" | "6" => Ok(PhysicalButton::Dpi),
            other => Err(format!(
                "unknown button {other:?}; use left, right, middle, back, forward or dpi"
            )),
        }
    }
}

define_str_enum! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum Media {
        PlayPause   => "play_pause" | "playpause",
        Stop        => "stop",
        Prev        => "prev_track" | "previous",
        Next        => "next_track" | "next",
        VolumeUp    => "volume_up" | "vol_up",
        VolumeDown  => "volume_down" | "vol_down",
        Mute        => "mute",
        MediaPlayer => "media_player",
    }
}

impl Media {
    fn usage(self) -> u16 {
        match self {
            Media::PlayPause => 0x00CD,
            Media::Stop => 0x00B7,
            Media::Prev => 0x00B6,
            Media::Next => 0x00B5,
            Media::VolumeUp => 0x00E9,
            Media::VolumeDown => 0x00EA,
            Media::Mute => 0x00E2,
            Media::MediaPlayer => 0x0183,
        }
    }
}

/// What a physical button does when handled by the mouse firmware itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Binding {
    Disabled,
    Mouse(MouseButton),
    Keys { modifiers: Vec<Modifier>, key: Key },
    Media(Media),
    DpiUp,
    DpiDown,
    DpiCycle,
    Macro(u8),
}

fn mouse_code(btn: MouseButton) -> u8 {
    match btn {
        MouseButton::Left => 0xF0,
        MouseButton::Right => 0xF1,
        MouseButton::Middle => 0xF2,
        MouseButton::Back => 0xF3,
        MouseButton::Forward => 0xF4,
    }
}

pub fn modifier_bit(m: Modifier) -> u8 {
    match m {
        Modifier::Ctrl => 0x01,
        Modifier::Shift => 0x02,
        Modifier::Alt => 0x04,
        Modifier::Meta => 0x08,
    }
}

/// HID keyboard usage for the left-hand variant of each modifier.
fn modifier_usage(m: Modifier) -> u8 {
    0xE0 + modifier_bit(m).trailing_zeros() as u8
}

#[rustfmt::skip]
pub fn key_usage(key: Key) -> u8 {
    match key {
        Key::A => 0x04, Key::B => 0x05, Key::C => 0x06, Key::D => 0x07,
        Key::E => 0x08, Key::F => 0x09, Key::G => 0x0A, Key::H => 0x0B,
        Key::I => 0x0C, Key::J => 0x0D, Key::K => 0x0E, Key::L => 0x0F,
        Key::M => 0x10, Key::N => 0x11, Key::O => 0x12, Key::P => 0x13,
        Key::Q => 0x14, Key::R => 0x15, Key::S => 0x16, Key::T => 0x17,
        Key::U => 0x18, Key::V => 0x19, Key::W => 0x1A, Key::X => 0x1B,
        Key::Y => 0x1C, Key::Z => 0x1D,
        Key::Num1 => 0x1E, Key::Num2 => 0x1F, Key::Num3 => 0x20,
        Key::Num4 => 0x21, Key::Num5 => 0x22, Key::Num6 => 0x23,
        Key::Num7 => 0x24, Key::Num8 => 0x25, Key::Num9 => 0x26,
        Key::Num0 => 0x27,
        Key::Enter => 0x28, Key::Escape => 0x29, Key::Backspace => 0x2A,
        Key::Tab => 0x2B, Key::Space => 0x2C,
        Key::F1 => 0x3A, Key::F2 => 0x3B, Key::F3 => 0x3C, Key::F4 => 0x3D,
        Key::F5 => 0x3E, Key::F6 => 0x3F, Key::F7 => 0x40, Key::F8 => 0x41,
        Key::F9 => 0x42, Key::F10 => 0x43, Key::F11 => 0x44, Key::F12 => 0x45,
        Key::Insert => 0x49, Key::Home => 0x4A, Key::PageUp => 0x4B,
        Key::Delete => 0x4C, Key::End => 0x4D, Key::PageDown => 0x4E,
        Key::Right => 0x4F, Key::Left => 0x50, Key::Down => 0x51, Key::Up => 0x52,
        Key::LeftBracket => 0x2F, Key::RightBracket => 0x30,
    }
}

impl Binding {
    pub fn encode(&self) -> [u8; 4] {
        match self {
            Binding::Disabled => [0, 0, 0, 0],
            Binding::Mouse(btn) => [0x01, 0x00, mouse_code(*btn), 0x00],
            Binding::Keys { modifiers, key } => {
                let mods = modifiers.iter().fold(0u8, |acc, m| acc | modifier_bit(*m));
                [0x00, mods, key_usage(*key), 0x00]
            }
            Binding::Media(media) => {
                let [lo, hi] = media.usage().to_le_bytes();
                [0x03, 0x00, lo, hi]
            }
            Binding::DpiUp => [0x04, 0x00, 0x01, 0x00],
            Binding::DpiDown => [0x04, 0x00, 0x02, 0x00],
            Binding::DpiCycle => [0x07, 0x00, 0x01, 0x00],
            Binding::Macro(slot) => [0x09, 0x00, *slot, 0x00],
        }
    }
}

impl FromStr for Binding {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim().to_lowercase();
        match s.as_str() {
            "disable" | "disabled" | "none" => return Ok(Binding::Disabled),
            "dpi_up" => return Ok(Binding::DpiUp),
            "dpi_down" => return Ok(Binding::DpiDown),
            "dpi_cycle" => return Ok(Binding::DpiCycle),
            _ => {}
        }
        if let Some(slot) = s.strip_prefix("macro:") {
            let slot: u8 = slot
                .parse()
                .map_err(|_| format!("invalid macro slot {slot:?}"))?;
            if slot >= MACRO_SLOTS {
                return Err(format!("macro slot must be 0-{}", MACRO_SLOTS - 1));
            }
            return Ok(Binding::Macro(slot));
        }
        if let Some(media) = Media::from_str_lower(&s) {
            return Ok(Binding::Media(media));
        }
        match s.parse::<Action>()? {
            Action::Mouse(btn) => Ok(Binding::Mouse(btn)),
            Action::Keys { modifiers, key } => Ok(Binding::Keys { modifiers, key }),
        }
    }
}

impl fmt::Display for Binding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Binding::Disabled => write!(f, "disabled"),
            Binding::Mouse(btn) => write!(f, "{}", Action::Mouse(*btn)),
            Binding::Keys { modifiers, key } => write!(
                f,
                "{}",
                Action::Keys {
                    modifiers: modifiers.clone(),
                    key: *key
                }
            ),
            Binding::Media(media) => write!(f, "{}", media.canonical_name()),
            Binding::DpiUp => write!(f, "dpi_up"),
            Binding::DpiDown => write!(f, "dpi_down"),
            Binding::DpiCycle => write!(f, "dpi_cycle"),
            Binding::Macro(slot) => write!(f, "macro:{slot}"),
        }
    }
}

/// Builds the two 32-byte output packets that carry the 16-slot button table.
pub fn encode_buttons(bindings: &[Binding; BUTTON_COUNT]) -> [[u8; OUTPUT_PAYLOAD]; 2] {
    let mut table = [0u8; BUTTON_TABLE_LEN];
    for (idx, binding) in bindings.iter().enumerate() {
        table[idx * 4..idx * 4 + 4].copy_from_slice(&binding.encode());
    }
    let mut packets = [[0u8; OUTPUT_PAYLOAD]; 2];
    for (pkt, chunk) in packets.iter_mut().zip(table.chunks(OUTPUT_PAYLOAD)) {
        pkt.copy_from_slice(chunk);
    }
    packets
}

pub fn buttons_header(packet: u8) -> [u8; FEATURE_LEN] {
    feature(CMD_BUTTONS, &[0x02, packet, OUTPUT_PAYLOAD as u8])
}

/// How a macro runs when its button is pressed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum MacroMode {
    /// Run `repeat` times per click
    #[default]
    Once,
    /// Loop while the button is held
    Hold,
    /// Click to start looping, click again to stop
    Toggle,
}

impl MacroMode {
    fn code(self) -> u8 {
        match self {
            MacroMode::Hold => 0,
            MacroMode::Toggle => 1,
            MacroMode::Once => 2,
        }
    }
}

impl fmt::Display for MacroMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            MacroMode::Once => "once",
            MacroMode::Hold => "hold",
            MacroMode::Toggle => "toggle",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MacroEvent {
    Press(u8),
    Release(u8),
    Delay(u32),
}

fn macro_key_usage(name: &str) -> Result<u8, String> {
    if let Some(m) = Modifier::from_str_lower(name) {
        return Ok(modifier_usage(m));
    }
    if let Some(btn) = MouseButton::from_str_lower(name) {
        return Ok(mouse_code(btn));
    }
    Key::from_str_lower(name)
        .map(key_usage)
        .ok_or_else(|| format!("unknown macro key {name:?}"))
}

/// Parses a macro script: comma-separated steps where `a` taps a key,
/// `+ctrl` presses, `-ctrl` releases and `50ms` waits.
pub fn parse_macro(script: &str) -> Result<Vec<MacroEvent>, String> {
    let mut events = Vec::new();
    for step in script.split(',').map(|s| s.trim().to_lowercase()) {
        if step.is_empty() {
            continue;
        }
        if let Some(ms) = step.strip_suffix("ms") {
            let ms: u32 = ms
                .trim()
                .parse()
                .map_err(|_| format!("invalid delay {step:?}"))?;
            events.push(MacroEvent::Delay(ms));
        } else if let Some(key) = step.strip_prefix('+') {
            events.push(MacroEvent::Press(macro_key_usage(key)?));
        } else if let Some(key) = step.strip_prefix('-') {
            events.push(MacroEvent::Release(macro_key_usage(key)?));
        } else {
            let usage = macro_key_usage(&step)?;
            events.push(MacroEvent::Press(usage));
            events.push(MacroEvent::Release(usage));
        }
    }
    if events.is_empty() {
        return Err("macro is empty".into());
    }
    if matches!(events.first(), Some(MacroEvent::Delay(_))) {
        return Err("a macro can't start with a delay".into());
    }
    let mut held = Vec::new();
    for event in &events {
        match event {
            MacroEvent::Delay(ms) => check_delay(*ms)?,
            MacroEvent::Press(usage) => held.push(*usage),
            MacroEvent::Release(usage) => held.retain(|u| u != usage),
        }
    }
    if !held.is_empty() {
        // A key left down would stay stuck after the macro ends.
        return Err("every `+key` needs a matching `-key` later in the macro".into());
    }
    Ok(events)
}

/// The firmware counts short delays in 10 ms ticks (up to 1270 ms) and long
/// ones in 100 ms ticks, so other values can't be stored exactly.
fn check_delay(ms: u32) -> Result<(), String> {
    match ms {
        10..=1270 if ms % 10 == 0 => Ok(()),
        1271..=600_000 if ms % 100 == 0 => Ok(()),
        10..=1270 => Err(format!("delay {ms} ms: use a multiple of 10 ms")),
        1271..=600_000 => Err(format!("delay {ms} ms: above 1270 ms use a multiple of 100 ms")),
        _ => Err(format!("delay {ms} ms: must be between 10 ms and 600000 ms")),
    }
}

fn encode_delay(ms: u32) -> Result<[u8; 4], String> {
    check_delay(ms)?;
    let (unit, tick_ms) = if ms <= 1270 { (0x01, 10) } else { (0x0A, 100) };
    let [lo, hi] = ((ms / tick_ms) as u16).to_le_bytes();
    Ok([unit, 0x00, lo, hi])
}

/// Builds the 288-byte macro blob: `[slot, mode, repeat lo, repeat hi]` + events.
pub fn encode_macro(
    slot: u8,
    mode: MacroMode,
    repeat: u16,
    events: &[MacroEvent],
) -> Result<Vec<[u8; OUTPUT_PAYLOAD]>, String> {
    if slot >= MACRO_SLOTS {
        return Err(format!("macro slot must be 0-{}", MACRO_SLOTS - 1));
    }
    if events.len() > MACRO_MAX_EVENTS {
        return Err(format!(
            "macro has {} events; the device holds at most {MACRO_MAX_EVENTS}",
            events.len()
        ));
    }
    let mut blob = vec![0u8; MACRO_BLOB_LEN];
    let [lo, hi] = repeat.to_le_bytes();
    blob[..4].copy_from_slice(&[slot, mode.code(), lo, hi]);
    for (idx, event) in events.iter().enumerate() {
        let bytes = match event {
            MacroEvent::Press(usage) => [0x00, *usage, 0x00, 0x00],
            MacroEvent::Release(usage) => [0x80, *usage, 0x00, 0x00],
            MacroEvent::Delay(ms) => encode_delay(*ms)?,
        };
        blob[4 + idx * 4..8 + idx * 4].copy_from_slice(&bytes);
    }
    Ok(blob
        .chunks(OUTPUT_PAYLOAD)
        .map(|c| c.try_into().expect("blob is a whole number of packets"))
        .collect())
}

pub fn macro_header(packet: u8, slot: u8) -> [u8; FEATURE_LEN] {
    feature(CMD_MACRO, &[0x09, packet, OUTPUT_PAYLOAD as u8, slot])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpi_round_trip_matches_factory_table() {
        let factory = [0x06, 0x0e, 0x16, 0x1e, 0x2e, 0x62];
        let dpi = decode_dpi(&factory);
        assert_eq!(dpi, [800, 1600, 2400, 3200, 4800, 10000]);
        assert_eq!(encode_dpi(&dpi).unwrap(), factory);
    }

    #[test]
    fn dpi_rejects_out_of_range_and_unaligned() {
        assert!(dpi_to_code(100).is_err());
        assert!(dpi_to_code(10_100).is_err());
        assert!(dpi_to_code(850).is_err());
    }

    #[test]
    fn battery_status_decodes_observed_replies() {
        let on_battery = decode_battery(&[0x4c, 0x01, 0x01, 0x02, 0x04, 0x00]).unwrap();
        assert_eq!(on_battery, Battery { percent: 76, charging: false });
        let charging = decode_battery(&[0x58, 0x02, 0x01, 0x02, 0x04, 0x00]).unwrap();
        assert_eq!(charging.to_string(), "88% (charging)");
        assert!(decode_battery(&[0xff, 0xff]).is_none());
    }

    #[test]
    fn polling_codes() {
        assert_eq!("1000".parse::<PollingRate>().unwrap().code(), 1);
        assert_eq!("125hz".parse::<PollingRate>().unwrap().code(), 8);
        assert_eq!(PollingRate::from_code(2), Some(PollingRate::Hz500));
        assert!("300".parse::<PollingRate>().is_err());
    }

    #[test]
    fn feature_frame_layout() {
        assert_eq!(feature(CMD_POLLING, &[1]), [7, 1, 1, 0, 0, 0, 0, 0]);
        assert_eq!(buttons_header(1), [7, 5, 2, 1, 0x20, 0, 0, 0]);
        assert_eq!(macro_header(3, 2), [7, 7, 9, 3, 0x20, 2, 0, 0]);
    }

    #[test]
    fn binding_encodings_match_vendor_tool() {
        let enc = |s: &str| s.parse::<Binding>().unwrap().encode();
        assert_eq!(enc("ctrl+c"), [0x00, 0x01, 0x06, 0x00]);
        assert_eq!(enc("alt+f4"), [0x00, 0x04, 0x3d, 0x00]);
        assert_eq!(enc("meta+d"), [0x00, 0x08, 0x07, 0x00]);
        assert_eq!(enc("mouse_left"), [0x01, 0x00, 0xf0, 0x00]);
        assert_eq!(enc("play_pause"), [0x03, 0x00, 0xcd, 0x00]);
        assert_eq!(enc("media_player"), [0x03, 0x00, 0x83, 0x01]);
        assert_eq!(enc("dpi_down"), [0x04, 0x00, 0x02, 0x00]);
        assert_eq!(enc("macro:2"), [0x09, 0x00, 0x02, 0x00]);
        assert_eq!(enc("none"), [0, 0, 0, 0]);
    }

    #[test]
    fn binding_display_round_trips() {
        for s in ["ctrl+shift+t", "mouse_back", "volume_up", "dpi_cycle", "macro:0", "disabled"] {
            let b: Binding = s.parse().unwrap();
            assert_eq!(b.to_string().parse::<Binding>().unwrap(), b);
        }
    }

    #[test]
    fn default_button_table_matches_vendor_tool() {
        let defaults = PhysicalButton::ALL.map(PhysicalButton::default_binding);
        let [pkt0, pkt1] = encode_buttons(&defaults);
        assert_eq!(
            pkt0[..24],
            [
                0x01, 0, 0xf0, 0, 0x01, 0, 0xf1, 0, 0x01, 0, 0xf2, 0, 0x01, 0, 0xf3, 0, 0x01, 0,
                0xf4, 0, 0x07, 0, 0x01, 0
            ]
        );
        assert!(pkt0[24..].iter().all(|&b| b == 0));
        assert!(pkt1.iter().all(|&b| b == 0));
    }

    #[test]
    fn macro_script_parses_and_encodes() {
        let events = parse_macro("+ctrl, c, 20ms, -ctrl").unwrap();
        assert_eq!(
            events,
            vec![
                MacroEvent::Press(0xE0),
                MacroEvent::Press(0x06),
                MacroEvent::Release(0x06),
                MacroEvent::Delay(20),
                MacroEvent::Release(0xE0),
            ]
        );
        let packets = encode_macro(1, MacroMode::Once, 1, &events).unwrap();
        assert_eq!(packets.len(), MACRO_PACKETS as usize);
        assert_eq!(
            packets[0][..24],
            [
                1, 2, 1, 0, 0x00, 0xE0, 0, 0, 0x00, 0x06, 0, 0, 0x80, 0x06, 0, 0, 0x01, 0, 2, 0,
                0x80, 0xE0, 0, 0
            ]
        );
    }

    #[test]
    fn macro_rejects_unstorable_scripts() {
        assert!(parse_macro("20ms, a").is_err());
        assert!(parse_macro("a, 5ms, b").is_err());
        assert!(parse_macro("a, 25ms, b").is_err());
        assert!(parse_macro("a, 1350ms, b").is_err());
        assert!(parse_macro("+ctrl, c").is_err());
        assert!(parse_macro("a, 1300ms, b").is_ok());
    }

    #[test]
    fn macro_rejects_overflow() {
        let events = vec![MacroEvent::Press(4); MACRO_MAX_EVENTS + 1];
        assert!(encode_macro(0, MacroMode::Once, 1, &events).is_err());
    }
}
