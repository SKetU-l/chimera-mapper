use clap::{Args, Parser, Subcommand};
use hidapi::{DeviceInfo, HidApi, HidDevice};
use std::error::Error;
use std::ffi::CString;
use std::fmt::Write as _;

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

#[derive(Clone, Copy, Debug)]
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
        println!(
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
        );
    }

    Ok(())
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
        let c_path = CString::new(path.as_str())?;
        return Ok(api.open_path(&c_path)?);
    }

    if args.vid.is_none() || args.pid.is_none() {
        return Err("select a device with --path or with both --vid and --pid; use `list` first".into());
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
            eprintln!("multiple devices matched; add --serial, --usage-page, --usage, --interface-number, or --path");
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
    let cfg = MappingConfig {
        button_byte: args.button_byte,
        side_mask: args.side_mask,
        extra_mask: args.extra_mask,
    };
    let mut state = MapperState::default();
    let mut buf = vec![0u8; args.report_len];

    loop {
        let size = device.read_timeout(&mut buf, args.timeout_ms)?;
        if size == 0 {
            continue;
        }

        let report = &buf[..size];
        let transitions = state.update(cfg, report);
        let byte = report.get(cfg.button_byte).copied().unwrap_or_default();
        println!(
            "report=[{}] byte[{}]=0x{:02x} forward={} back={}",
            format_report(report),
            cfg.button_byte,
            byte,
            (byte & cfg.side_mask) != 0,
            (byte & cfg.extra_mask) != 0
        );

        for transition in transitions {
            println!(
                "  {} {}",
                match transition.kind {
                    ActionKind::Forward => "forward",
                    ActionKind::Back => "back",
                },
                if transition.pressed { "pressed" } else { "released" }
            );
        }
    }
}

fn run_mapper(args: RunArgs) -> AppResult<()> {
    let api = HidApi::new()?;
    let device = open_device(&api, &args)?;
    let cfg = MappingConfig {
        button_byte: args.button_byte,
        side_mask: args.side_mask,
        extra_mask: args.extra_mask,
    };
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
