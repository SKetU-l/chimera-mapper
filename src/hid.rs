use clap::Args;
use hidapi::{DeviceInfo, HidApi, HidDevice};
use std::fmt::Write as _;
use std::io::{self, Write};
use std::time::{Duration, Instant};

use crate::config::{
    AppConfig, AppResult, MappingConfig, SavedProfile, load_config, save_config,
};

#[derive(Args, Clone)]
pub struct RunArgs {
    #[arg(long)]
    pub path: Option<String>,
    #[arg(long, value_parser = parse_u16)]
    pub vid: Option<u16>,
    #[arg(long, value_parser = parse_u16)]
    pub pid: Option<u16>,
    #[arg(long)]
    pub serial: Option<String>,
    #[arg(long, value_parser = parse_u16)]
    pub usage_page: Option<u16>,
    #[arg(long, value_parser = parse_u16)]
    pub usage: Option<u16>,
    #[arg(long)]
    pub interface_number: Option<i32>,
    #[arg(long, default_value_t = 64)]
    pub report_len: usize,
    #[arg(long, default_value_t = 1)]
    pub button_byte: usize,
    #[arg(long, value_parser = parse_u8, default_value = "0x10")]
    pub side_mask: u8,
    #[arg(long, value_parser = parse_u8, default_value = "0x08")]
    pub extra_mask: u8,
    #[arg(long, default_value_t = 250)]
    pub timeout_ms: i32,
    #[arg(long, default_value = "chimera-mapper")]
    pub name: String,
}

pub fn default_run_args() -> RunArgs {
    RunArgs {
        path: None, vid: None, pid: None, serial: None,
        usage_page: None, usage: None, interface_number: None,
        report_len: 64, button_byte: 1, side_mask: 0x10, extra_mask: 0x08,
        timeout_ms: 250, name: "chimera-mapper".into(),
    }
}

#[derive(Default)]
pub struct MapperState {
    pub prev_forward: bool,
    pub prev_back: bool,
}

#[derive(Clone, Copy)]
pub enum ActionKind {
    Forward,
    Back,
}

#[derive(Clone, Copy)]
pub struct Transition {
    pub kind: ActionKind,
    pub pressed: bool,
}

impl MapperState {
    pub fn update(&mut self, cfg: MappingConfig, report: &[u8]) -> Vec<Transition> {
        if report.len() <= cfg.button_byte {
            return Vec::new();
        }
        let byte = report[cfg.button_byte];
        let forward = (byte & cfg.side_mask) != 0;
        let back = (byte & cfg.extra_mask) != 0;
        let mut out = Vec::with_capacity(2);
        if forward != self.prev_forward {
            out.push(Transition { kind: ActionKind::Forward, pressed: forward });
            self.prev_forward = forward;
        }
        if back != self.prev_back {
            out.push(Transition { kind: ActionKind::Back, pressed: back });
            self.prev_back = back;
        }
        out
    }

    pub fn synthesize_releases(&mut self) -> Vec<Transition> {
        let mut out = Vec::new();
        if self.prev_forward {
            out.push(Transition { kind: ActionKind::Forward, pressed: false });
            self.prev_forward = false;
        }
        if self.prev_back {
            out.push(Transition { kind: ActionKind::Back, pressed: false });
            self.prev_back = false;
        }
        out
    }
}

#[derive(Clone)]
pub struct AutodetectCandidate {
    pub score: i32,
    pub device: DeviceInfo,
    pub last_report: Vec<u8>,
}

fn parse_prefixed_u32(input: &str) -> Result<u32, String> {
    let trimmed = input.trim();
    if let Some(rest) = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")) {
        u32::from_str_radix(rest, 16).map_err(|e| format!("invalid hex value {trimmed:?}: {e}"))
    } else {
        trimmed.parse::<u32>().map_err(|e| format!("invalid integer value {trimmed:?}: {e}"))
    }
}

pub fn parse_u16(input: &str) -> Result<u16, String> {
    let value = parse_prefixed_u32(input)?;
    u16::try_from(value).map_err(|_| format!("value {input:?} does not fit into u16"))
}

pub fn parse_u8(input: &str) -> Result<u8, String> {
    let value = parse_prefixed_u32(input)?;
    u8::try_from(value).map_err(|_| format!("value {input:?} does not fit into u8"))
}

pub fn format_report(report: &[u8]) -> String {
    let mut out = String::new();
    for (idx, byte) in report.iter().enumerate() {
        if idx > 0 { out.push(' '); }
        let _ = write!(out, "{byte:02x}");
    }
    out
}

pub fn format_device(device: &DeviceInfo) -> String {
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

pub fn list_devices() -> AppResult<()> {
    let api = HidApi::new()?;
    for device in api.device_list() {
        println!("{}", format_device(device));
    }
    Ok(())
}

pub fn has_explicit_device_selector(args: &RunArgs) -> bool {
    args.path.is_some()
        || args.vid.is_some()
        || args.pid.is_some()
        || args.serial.is_some()
        || args.usage_page.is_some()
        || args.usage.is_some()
        || args.interface_number.is_some()
}

pub fn mapping_from_args(args: &RunArgs) -> MappingConfig {
    MappingConfig {
        button_byte: args.button_byte,
        side_mask: args.side_mask,
        extra_mask: args.extra_mask,
    }
}

pub fn saved_profile_from_args(args: &RunArgs) -> Option<SavedProfile> {
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

pub fn apply_saved_profile(args: &RunArgs, profile: &SavedProfile) -> RunArgs {
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

fn candidate_haystack(device: &DeviceInfo) -> String {
    let mut s = device.path().to_string_lossy().into_owned();
    s.push(' ');
    if let Some(v) = device.product_string() { s.push_str(v); s.push(' '); }
    if let Some(v) = device.manufacturer_string() { s.push_str(v); s.push(' '); }
    if let Some(v) = device.serial_number() { s.push_str(v); }
    s.to_ascii_lowercase()
}

fn autodetect_score(device: &DeviceInfo) -> Option<i32> {
    let h = candidate_haystack(device);
    let mut score = 0;
    if h.contains("chimera") { score += 10_000; }
    if h.contains("mouse") { score += 500; }
    if device.usage_page() == 0x0001 { score += 200; }
    if device.usage() == 0x0002 { score += 200; }
    if device.interface_number() == 0 { score += 100; }
    if device.path().to_string_lossy().contains("event") { score += 25; }
    if score == 0 { None } else { Some(score) }
}

fn read_report_snapshot(device: &HidDevice, report_len: usize, timeout_ms: i32) -> AppResult<Vec<u8>> {
    let mut buf = vec![0u8; report_len];
    let size = device.read_timeout(&mut buf, timeout_ms)?;
    if size < report_len { buf[size..].fill(0); }
    Ok(buf)
}

fn any_bit_newly_pressed(previous: &[u8], current: &[u8]) -> bool {
    previous.iter().zip(current.iter()).any(|(&b, &a)| (!b) & a != 0)
}

fn build_autodetect_candidates(api: &HidApi, report_len: usize) -> Vec<AutodetectCandidate> {
    let mut candidates: Vec<AutodetectCandidate> = api
        .device_list()
        .filter_map(|device| {
            let score = autodetect_score(device)?;
            let last_report = device
                .open_device(api)
                .ok()
                .and_then(|h| read_report_snapshot(&h, report_len, 20).ok())
                .unwrap_or_else(|| vec![0; report_len]);
            Some(AutodetectCandidate { score, device: device.clone(), last_report })
        })
        .collect();
    candidates.sort_by(|a, b| b.score.cmp(&a.score));
    candidates
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
            let Ok(handle) = candidate.device.open_device(api) else { continue };
            let Ok(current) = read_report_snapshot(&handle, report_len, 20) else { continue };
            if any_bit_newly_pressed(&candidate.last_report, &current) {
                return Ok(Some(candidate.device.clone()));
            }
            candidate.last_report = current;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    Ok(None)
}

pub fn autodetect_args(api: &HidApi, args: &RunArgs) -> AppResult<RunArgs> {
    let mut candidates = build_autodetect_candidates(api, args.report_len);
    if candidates.is_empty() {
        return Err("no likely Chimera HID device found; run `list` and pass selectors manually".into());
    }

    let best_score = candidates[0].score;
    let plausible_len = candidates.iter().take_while(|c| c.score + 200 >= best_score).count();

    let device = if plausible_len <= 1 {
        candidates[0].device.clone()
    } else {
        match resolve_by_behavior(api, &mut candidates[..plausible_len], args.report_len)? {
            Some(d) => d,
            None => candidates[0].device.clone(),
        }
    };

    let mut resolved = args.clone();
    resolved.path = Some(device.path().to_string_lossy().into_owned());
    resolved.vid = Some(device.vendor_id());
    resolved.pid = Some(device.product_id());
    resolved.serial = device.serial_number().map(ToOwned::to_owned);
    resolved.usage_page = Some(device.usage_page());
    resolved.usage = Some(device.usage());
    resolved.interface_number = Some(device.interface_number());

    eprintln!(
        "auto-selected device: product={} vid=0x{:04x} pid=0x{:04x} usage_page=0x{:04x} usage=0x{:04x} iface={}",
        device.product_string().unwrap_or("-"),
        device.vendor_id(), device.product_id(),
        device.usage_page(), device.usage(), device.interface_number(),
    );

    Ok(resolved)
}

pub fn detect_and_save(api: &HidApi, args: &RunArgs) -> AppResult<RunArgs> {
    let resolved = autodetect_args(api, args)?;
    if let Some(profile) = saved_profile_from_args(&resolved) {
        if let Err(e) = save_config(&AppConfig { profile: Some(profile) }) {
            eprintln!("warning: failed to save device profile: {e}");
        }
    }
    Ok(resolved)
}

pub fn resolve_run_args(api: &HidApi, args: RunArgs) -> AppResult<RunArgs> {
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
    detect_and_save(api, &args)
}

fn matches_filters(device: &DeviceInfo, args: &RunArgs) -> bool {
    if let Some(path) = &args.path {
        if device.path().to_string_lossy() != path.as_str() { return false; }
    }
    if let Some(vid) = args.vid {
        if device.vendor_id() != vid { return false; }
    }
    if let Some(pid) = args.pid {
        if device.product_id() != pid { return false; }
    }
    if let Some(serial) = &args.serial {
        if device.serial_number() != Some(serial.as_str()) { return false; }
    }
    if let Some(usage_page) = args.usage_page {
        if device.usage_page() != usage_page { return false; }
    }
    if let Some(usage) = args.usage {
        if device.usage() != usage { return false; }
    }
    if let Some(interface_number) = args.interface_number {
        if device.interface_number() != interface_number { return false; }
    }
    true
}

pub fn open_device(api: &HidApi, args: &RunArgs) -> AppResult<HidDevice> {
    // Try path first, but if stale fall through to VID+PID matching
    if let Some(path) = &args.path {
        if let Some(device) = api.device_list().find(|d| d.path().to_string_lossy() == path.as_str()) {
            return Ok(device.open_device(api)?);
        }
    }

    if args.vid.is_none() || args.pid.is_none() {
        return Err("select a device with --path or with both --vid and --pid; use `list` first".into());
    }

    let matches: Vec<_> = api.device_list().filter(|d| matches_filters(d, args)).cloned().collect();
    match matches.as_slice() {
        [] => Err("no HID device matched the supplied filters".into()),
        [device] => Ok(device.open_device(api)?),
        many => {
            eprintln!("multiple devices matched; add --serial, --usage-page, --usage, --interface-number, or --path");
            for device in many {
                eprintln!(
                    "  path={} vid=0x{:04x} pid=0x{:04x} usage_page=0x{:04x} usage=0x{:04x} iface={} product={} serial={}",
                    device.path().to_string_lossy(),
                    device.vendor_id(), device.product_id(),
                    device.usage_page(), device.usage(), device.interface_number(),
                    device.product_string().unwrap_or("-"),
                    device.serial_number().unwrap_or("-"),
                );
            }
            Err("device selection was ambiguous".into())
        }
    }
}
