use clap::{Args, Parser, Subcommand};
use hidapi::{DeviceInfo, HidApi, HidDevice};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

type AppResult<T> = Result<T, Box<dyn Error>>;

#[derive(Parser)]
#[command(author, version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    List,
    Dump(RunArgs),
    Run(RunArgs),
}

#[derive(Args, Clone)]
struct RunArgs {
    #[arg(long)]
    path: Option<String>,
    #[arg(long, value_parser = parse_u16)]
    vid: Option<u16>,
    #[arg(long, value_parser = parse_u16)]
    pid: Option<u16>,
    #[arg(long)]
    serial: Option<String>,
    #[arg(long, value_parser = parse_u16)]
    usage_page: Option<u16>,
    #[arg(long, value_parser = parse_u16)]
    usage: Option<u16>,
    #[arg(long)]
    interface_number: Option<i32>,
    #[arg(long, default_value_t = 64)]
    report_len: usize,
    #[arg(long, default_value_t = 1)]
    button_byte: usize,
    #[arg(long, value_parser = parse_u8, default_value = "0x10")]
    side_mask: u8,
    #[arg(long, value_parser = parse_u8, default_value = "0x08")]
    extra_mask: u8,
    #[arg(long, default_value_t = 250)]
    timeout_ms: i32,
    #[arg(long, default_value = "chimera-mapper")]
    name: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct MappingConfig {
    button_byte: usize,
    side_mask: u8,
    extra_mask: u8,
}

#[derive(Default)]
struct MapperState {
    prev_forward: bool,
    prev_back: bool,
}

#[derive(Clone, Copy)]
enum ActionKind {
    Forward,
    Back,
}

#[derive(Clone, Copy)]
struct Transition {
    kind: ActionKind,
    pressed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SavedProfile {
    path: String,
    vid: u16,
    pid: u16,
    serial: Option<String>,
    usage_page: u16,
    usage: u16,
    interface_number: i32,
    mapping: MappingConfig,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct AppConfig {
    profile: Option<SavedProfile>,
}

#[derive(Clone)]
struct AutodetectCandidate {
    score: i32,
    device: DeviceInfo,
    last_report: Vec<u8>,
}

impl MapperState {
    fn update(&mut self, cfg: MappingConfig, report: &[u8]) -> Vec<Transition> {
        if report.len() <= cfg.button_byte {
            return Vec::new();
        }

        let byte = report[cfg.button_byte];
        let forward = (byte & cfg.side_mask) != 0;
        let back = (byte & cfg.extra_mask) != 0;
        let mut out = Vec::with_capacity(2);

        if forward != self.prev_forward {
            out.push(Transition {
                kind: ActionKind::Forward,
                pressed: forward,
            });
            self.prev_forward = forward;
        }

        if back != self.prev_back {
            out.push(Transition {
                kind: ActionKind::Back,
                pressed: back,
            });
            self.prev_back = back;
        }

        out
    }
}

fn parse_prefixed_u32(input: &str) -> Result<u32, String> {
    let trimmed = input.trim();
    if let Some(rest) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        u32::from_str_radix(rest, 16).map_err(|e| format!("invalid hex value {trimmed:?}: {e}"))
    } else {
        trimmed
            .parse::<u32>()
            .map_err(|e| format!("invalid integer value {trimmed:?}: {e}"))
    }
}

fn parse_u16(input: &str) -> Result<u16, String> {
    let value = parse_prefixed_u32(input)?;
    u16::try_from(value).map_err(|_| format!("value {input:?} does not fit into u16"))
}

fn parse_u8(input: &str) -> Result<u8, String> {
    let value = parse_prefixed_u32(input)?;
    u8::try_from(value).map_err(|_| format!("value {input:?} does not fit into u8"))
}

fn config_path() -> AppResult<PathBuf> {
    let mut base =
        dirs::config_dir().ok_or("unable to locate config directory for current user")?;
    base.push("chimera-mapper");
    Ok(base.join("config.json"))
}

fn ensure_parent_dir(path: &Path) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn load_config() -> AppResult<AppConfig> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(AppConfig::default());
    }

    let raw = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

fn save_config(config: &AppConfig) -> AppResult<()> {
    let path = config_path()?;
    ensure_parent_dir(&path)?;
    fs::write(path, serde_json::to_string_pretty(config)?)?;
    Ok(())
}

fn format_report(report: &[u8]) -> String {
    let mut out = String::new();
    for (idx, byte) in report.iter().enumerate() {
        if idx > 0 {
            out.push(' ');
        }
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn list_devices() -> AppResult<()> {
    let api = HidApi::new()?;

    for device in api.device_list() {
        println!("{}", format_device(device));
    }

    Ok(())
}

fn format_device(device: &DeviceInfo) -> String {
    format!(
        "path={} vid=0x{:04x} pid=0x{:04x} usage_page=0x{:04x} usage=0x{:04x} iface={} product={} manufacturer={} serial={}",
        device.path().to_string_lossy(),
        device.vendor_id(),
        device.product_id(),
        device.usage_page(),
        device.usage(),
        device.interface_number(),
        device.product_string().unwrap_or("-"),
        device.manufacturer_string().unwrap_or("-"),
        device.serial_number().unwrap_or("-"),
    )
}

fn has_explicit_device_selector(args: &RunArgs) -> bool {
    args.path.is_some()
        || args.vid.is_some()
        || args.pid.is_some()
        || args.serial.is_some()
        || args.usage_page.is_some()
        || args.usage.is_some()
        || args.interface_number.is_some()
}

fn candidate_haystack(device: &DeviceInfo) -> String {
    let mut haystack = String::new();
    haystack.push_str(device.path().to_string_lossy().as_ref());
    haystack.push(' ');
    if let Some(value) = device.product_string() {
        haystack.push_str(value);
        haystack.push(' ');
    }
    if let Some(value) = device.manufacturer_string() {
        haystack.push_str(value);
        haystack.push(' ');
    }
    if let Some(value) = device.serial_number() {
        haystack.push_str(value);
    }
    haystack.to_ascii_lowercase()
}

fn autodetect_score(device: &DeviceInfo) -> Option<i32> {
    let haystack = candidate_haystack(device);
    let mut score = 0;

    if haystack.contains("chimera") {
        score += 10_000;
    }
    if haystack.contains("mouse") {
        score += 500;
    }
    if device.usage_page() == 0x0001 {
        score += 200;
    }
    if device.usage() == 0x0002 {
        score += 200;
    }
    if device.interface_number() == 0 {
        score += 100;
    }
    if device.path().to_string_lossy().contains("event") {
        score += 25;
    }

    if score == 0 { None } else { Some(score) }
}

fn read_report_snapshot(
    device: &HidDevice,
    report_len: usize,
    timeout_ms: i32,
) -> AppResult<Vec<u8>> {
    let mut buf = vec![0u8; report_len];
    let size = device.read_timeout(&mut buf, timeout_ms)?;
    if size < report_len {
        buf[size..].fill(0);
    }
    Ok(buf)
}

fn first_pressed_bit(previous: &[u8], current: &[u8]) -> Option<(usize, u8)> {
    for (index, (&before, &after)) in previous.iter().zip(current.iter()).enumerate() {
        let pressed = (!before) & after;
        if pressed != 0 {
            return Some((index, pressed & pressed.wrapping_neg()));
        }
    }

    None
}

fn build_autodetect_candidates(api: &HidApi, report_len: usize) -> Vec<AutodetectCandidate> {
    let mut candidates = Vec::new();

    for device in api.device_list() {
        let Some(score) = autodetect_score(device) else {
            continue;
        };

        let last_report = match device.open_device(api) {
            Ok(handle) => read_report_snapshot(&handle, report_len, 20)
                .unwrap_or_else(|_| vec![0; report_len]),
            Err(_) => vec![0; report_len],
        };

        candidates.push(AutodetectCandidate {
            score,
            device: device.clone(),
            last_report,
        });
    }

    candidates.sort_by(|left, right| right.score.cmp(&left.score));
    candidates
}

fn mapping_from_args(args: &RunArgs) -> MappingConfig {
    MappingConfig {
        button_byte: args.button_byte,
        side_mask: args.side_mask,
        extra_mask: args.extra_mask,
    }
}

fn saved_profile_from_args(args: &RunArgs) -> Option<SavedProfile> {
    Some(SavedProfile {
        path: args.path.clone()?,
        vid: args.vid?,
        pid: args.pid?,
        serial: args.serial.clone(),
        usage_page: args.usage_page?,
        usage: args.usage?,
        interface_number: args.interface_number?,
        mapping: mapping_from_args(args),
    })
}

fn apply_saved_profile(args: &RunArgs, profile: &SavedProfile) -> RunArgs {
    let mut resolved = args.clone();
    resolved.path = Some(profile.path.clone());
    resolved.vid = Some(profile.vid);
    resolved.pid = Some(profile.pid);
    resolved.serial = profile.serial.clone();
    resolved.usage_page = Some(profile.usage_page);
    resolved.usage = Some(profile.usage);
    resolved.interface_number = Some(profile.interface_number);
    resolved.button_byte = profile.mapping.button_byte;
    resolved.side_mask = profile.mapping.side_mask;
    resolved.extra_mask = profile.mapping.extra_mask;
    resolved
}

fn apply_detected_device(args: &RunArgs, device: &DeviceInfo) -> RunArgs {
    let mut resolved = args.clone();
    resolved.path = Some(device.path().to_string_lossy().into_owned());
    resolved.vid = Some(device.vendor_id());
    resolved.pid = Some(device.product_id());
    resolved.serial = device.serial_number().map(ToOwned::to_owned);
    resolved.usage_page = Some(device.usage_page());
    resolved.usage = Some(device.usage());
    resolved.interface_number = Some(device.interface_number());
    resolved
}

fn find_device_by_path(api: &HidApi, path: &str) -> Option<DeviceInfo> {
    api.device_list()
        .find(|device| device.path().to_string_lossy() == path)
        .cloned()
}

fn resolve_by_behavior(
    api: &HidApi,
    candidates: &mut [AutodetectCandidate],
    report_len: usize,
) -> AppResult<Option<DeviceInfo>> {
    if candidates.is_empty() {
        return Ok(None);
    }

    eprintln!("multiple plausible HID interfaces found");
    eprintln!("press either side button repeatedly to confirm the correct device");
    io::stderr().flush()?;

    let deadline = Instant::now() + Duration::from_secs(6);
    while Instant::now() < deadline {
        for candidate in candidates.iter_mut() {
            let Ok(handle) = candidate.device.open_device(api) else {
                continue;
            };
            let Ok(current) = read_report_snapshot(&handle, report_len, 20) else {
                continue;
            };

            if first_pressed_bit(&candidate.last_report, &current).is_some() {
                return Ok(Some(candidate.device.clone()));
            }

            candidate.last_report = current;
        }
    }

    Ok(None)
}

fn autodetect_args(api: &HidApi, args: &RunArgs) -> AppResult<RunArgs> {
    let mut candidates = build_autodetect_candidates(api, args.report_len);
    if candidates.is_empty() {
        return Err(
            "no likely Chimera HID device found; run `list` and pass selectors manually".into(),
        );
    }

    let best_score = candidates[0].score;
    let plausible_len = candidates
        .iter()
        .take_while(|candidate| candidate.score + 200 >= best_score)
        .count();

    let device = if plausible_len <= 1 {
        candidates[0].device.clone()
    } else {
        match resolve_by_behavior(api, &mut candidates[..plausible_len], args.report_len)? {
            Some(device) => device,
            None => candidates[0].device.clone(),
        }
    };

    let resolved = apply_detected_device(args, &device);

    eprintln!(
        "auto-selected device: product={} vid=0x{:04x} pid=0x{:04x} usage_page=0x{:04x} usage=0x{:04x} iface={}",
        device.product_string().unwrap_or("-"),
        device.vendor_id(),
        device.product_id(),
        device.usage_page(),
        device.usage(),
        device.interface_number(),
    );

    Ok(resolved)
}

fn resolve_run_args(api: &HidApi, args: RunArgs) -> AppResult<RunArgs> {
    if has_explicit_device_selector(&args) {
        return Ok(args);
    }

    if let Ok(config) = load_config() {
        if let Some(profile) = config.profile {
            let saved_args = apply_saved_profile(&args, &profile);
            if open_device(api, &saved_args).is_ok() {
                return Ok(saved_args);
            }
        }
    }

    let resolved = autodetect_args(api, &args)?;
    if let Some(profile) = saved_profile_from_args(&resolved) {
        let _ = save_config(&AppConfig {
            profile: Some(profile),
        });
    }
    Ok(resolved)
}

fn matches_filters(device: &DeviceInfo, args: &RunArgs) -> bool {
    if let Some(path) = &args.path {
        if device.path().to_string_lossy() != path.as_str() {
            return false;
        }
    }

    if let Some(vid) = args.vid {
        if device.vendor_id() != vid {
            return false;
        }
    }

    if let Some(pid) = args.pid {
        if device.product_id() != pid {
            return false;
        }
    }

    if let Some(serial) = &args.serial {
        if device.serial_number() != Some(serial.as_str()) {
            return false;
        }
    }

    if let Some(usage_page) = args.usage_page {
        if device.usage_page() != usage_page {
            return false;
        }
    }

    if let Some(usage) = args.usage {
        if device.usage() != usage {
            return false;
        }
    }

    if let Some(interface_number) = args.interface_number {
        if device.interface_number() != interface_number {
            return false;
        }
    }

    true
}

fn open_device(api: &HidApi, args: &RunArgs) -> AppResult<HidDevice> {
    if let Some(path) = &args.path {
        let device = find_device_by_path(api, path)
            .ok_or("no HID device matched the supplied --path selector")?;
        return Ok(device.open_device(api)?);
    }

    if args.vid.is_none() || args.pid.is_none() {
        return Err(
            "select a device with --path or with both --vid and --pid; use `list` first".into(),
        );
    }

    let matches: Vec<_> = api
        .device_list()
        .filter(|device| matches_filters(device, args))
        .cloned()
        .collect();

    match matches.as_slice() {
        [] => Err("no HID device matched the supplied filters".into()),
        [device] => Ok(device.open_device(api)?),
        many => {
            eprintln!(
                "multiple devices matched; add --serial, --usage-page, --usage, --interface-number, or --path"
            );
            for device in many {
                eprintln!(
                    "  path={} vid=0x{:04x} pid=0x{:04x} usage_page=0x{:04x} usage=0x{:04x} iface={} product={} serial={}",
                    device.path().to_string_lossy(),
                    device.vendor_id(),
                    device.product_id(),
                    device.usage_page(),
                    device.usage(),
                    device.interface_number(),
                    device.product_string().unwrap_or("-"),
                    device.serial_number().unwrap_or("-"),
                );
            }
            Err("device selection was ambiguous".into())
        }
    }
}

fn run_dump(args: RunArgs) -> AppResult<()> {
    let api = HidApi::new()?;
    let device = open_device(&api, &args)?;
    let cfg = mapping_from_args(&args);
    let mut buf = vec![0u8; args.report_len];

    loop {
        let size = device.read_timeout(&mut buf, args.timeout_ms)?;
        if size == 0 {
            continue;
        }

        let report = &buf[..size];
        let byte = report.get(cfg.button_byte).copied().unwrap_or_default();
        println!(
            "report=[{}] byte[{}]=0x{:02x} forward={} back={}",
            format_report(report),
            cfg.button_byte,
            byte,
            (byte & cfg.side_mask) != 0,
            (byte & cfg.extra_mask) != 0
        );
    }
}

fn run_mapper(args: RunArgs) -> AppResult<()> {
    let api = HidApi::new()?;
    let args = resolve_run_args(&api, args)?;
    let device = open_device(&api, &args)?;
    let cfg = mapping_from_args(&args);
    let mut state = MapperState::default();
    let mut emitter = backend::Emitter::new(&args.name)?;
    let mut buf = vec![0u8; args.report_len];

    loop {
        let size = device.read_timeout(&mut buf, args.timeout_ms)?;
        if size == 0 {
            continue;
        }

        for transition in state.update(cfg, &buf[..size]) {
            emitter.emit(transition)?;
        }
    }
}

#[cfg(target_os = "linux")]
mod backend {
    use super::{ActionKind, AppResult, Transition};
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
}

#[cfg(target_os = "macos")]
mod backend {
    use super::{ActionKind, AppResult, Transition};
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
                    if transition.pressed {
                        CGEventType::OtherMouseDown
                    } else {
                        CGEventType::OtherMouseUp
                    },
                    4_i64,
                ),
                ActionKind::Back => (
                    if transition.pressed {
                        CGEventType::OtherMouseDown
                    } else {
                        CGEventType::OtherMouseUp
                    },
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
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod backend {
    use super::{AppResult, Transition};

    pub struct Emitter;

    impl Emitter {
        pub fn new(_name: &str) -> AppResult<Self> {
            Err("this project currently supports only Linux and macOS".into())
        }

        pub fn emit(&mut self, _transition: Transition) -> AppResult<()> {
            Err("this project currently supports only Linux and macOS".into())
        }
    }
}

fn main() {
    let result = match Cli::parse().command {
        Command::List => list_devices(),
        Command::Dump(args) => run_dump(args),
        Command::Run(args) => run_mapper(args),
    };

    if let Err(err) = result {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
