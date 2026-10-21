use clap::{Parser, Subcommand};
use hidapi::HidApi;
use std::time::{Duration, Instant};

#[macro_use]
mod action;
mod backend;
mod config;
mod configure;
mod device;
mod hid;
mod protocol;

use action::{Action, MouseButton};
use config::{AppResult, ResolvedMapping, load_config, save_config};
use hid::{
    MapperState, RunArgs, device_path_present, format_report, list_devices, mapping_from_args,
    open_device, resolve_run_args, saved_profile_from_args,
};

#[derive(Parser)]
#[command(author, version, about)]
struct Cli {
    /// Print the frames that would be sent instead of writing to the mouse
    #[arg(long, global = true)]
    dry_run: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List all HID devices
    List,
    /// Print raw input reports (debugging)
    Dump(RunArgs),
    /// Run the side-button mapper daemon
    Run(RunArgs),
    /// Show the mouse's current on-board settings
    Info,
    /// Show the battery level
    Battery,
    /// Show or set the six DPI stages
    Dpi(configure::DpiArgs),
    /// Show or set the polling rate (125, 250, 500, 1000)
    Rate { hz: Option<String> },
    /// Show or set the lighting mode
    Led { mode: Option<String> },
    /// Show or set the rapid-fire interval in ms
    RapidFire { ms: Option<u32> },
    /// Show or remap buttons in the mouse's own memory
    Bind(configure::BindArgs),
    /// Manage on-board macros
    #[command(subcommand)]
    Macro(configure::MacroCommand),
    /// Save or load a complete settings profile
    #[command(subcommand)]
    Profile(configure::ProfileCommand),
}

fn release_all(state: &mut MapperState, cfg: &ResolvedMapping, emitter: &mut backend::Emitter) {
    for transition in state.synthesize_releases(cfg) {
        let _ = emitter.emit(&transition);
    }
}

fn run_dump(args: RunArgs) -> AppResult<()> {
    let api = HidApi::new()?;
    let args = resolve_run_args(&api, args)?;
    let device = open_device(&api, &args)?;
    let cfg = mapping_from_args(&args).resolve()?;
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
            (byte & cfg.extra_mask) != 0,
        );
    }
}

/// How long a read waits before housekeeping runs. A button press still
/// wakes the read immediately, so this only sets the idle wakeup rate.
const READ_TIMEOUT_MS: i32 = 500;
/// How often wireless connections check for the cable or a lost device.
const DEVICE_CHECK: Duration = Duration::from_secs(2);
const RETRY_MIN: Duration = Duration::from_secs(2);
const RETRY_MAX: Duration = Duration::from_secs(10);

fn retry_wait(delay: &mut Duration, what: &str, err: &dyn std::fmt::Display) {
    eprintln!("{what}: {err}; retrying in {}s", delay.as_secs());
    std::thread::sleep(*delay);
    *delay = (*delay * 2).min(RETRY_MAX);
}

/// Saves the connected device as the profile, but only when it changed, and
/// never over a config file that failed to load (that would drop the
/// on-board button/macro cache the mouse can't report back).
fn remember_profile(resolved: &RunArgs) {
    let Some(profile) = saved_profile_from_args(resolved) else {
        return;
    };
    let mut config = match load_config() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("not updating saved device profile: config unreadable: {e}");
            return;
        }
    };
    if config.profile.as_ref() == Some(&profile) {
        return;
    }
    config.profile = Some(profile);
    if let Err(e) = save_config(&config) {
        eprintln!("could not save device profile: {e}");
    }
}

/// Over Bluetooth the mouse declares all five buttons, so the OS already
/// delivers back/forward. Emitting them again would fire every click twice.
fn natively_handled(cfg: &ResolvedMapping) -> bool {
    cfg.side_action == Action::Mouse(MouseButton::Forward)
        && cfg.extra_action == Action::Mouse(MouseButton::Back)
}

fn run_mapper(args: RunArgs) -> AppResult<()> {
    let mut state = MapperState::default();
    let mut emitter = loop {
        match backend::Emitter::new(&args.name) {
            Ok(emitter) => break emitter,
            Err(e) => {
                eprintln!("failed to initialize virtual input device: {e}; retrying in 2s");
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    };

    let mut retry = RETRY_MIN;
    loop {
        let mut api = match HidApi::new() {
            Ok(api) => api,
            Err(e) => {
                retry_wait(&mut retry, "HID subsystem unavailable", &e);
                continue;
            }
        };
        let resolved = match resolve_run_args(&api, args.clone()) {
            Ok(r) => r,
            Err(e) => {
                retry_wait(&mut retry, "device not found", &e);
                continue;
            }
        };
        let device = match open_device(&api, &resolved) {
            Ok(d) => d,
            Err(e) => {
                retry_wait(&mut retry, "failed to open device", &e);
                continue;
            }
        };
        retry = RETRY_MIN;

        remember_profile(&resolved);
        let cfg = mapping_from_args(&resolved).resolve()?;
        let wireless = hid::is_wireless(resolved.vid, resolved.pid);
        let current_path = resolved.path.clone();

        // Re-checks the device list; true when the mapper should reconnect.
        let device_changed = |api: &mut HidApi| {
            let _ = api.refresh_devices();
            if hid::wired_present(api) {
                eprintln!("wired device detected; switching over");
                return true;
            }
            if !device_path_present(api, &current_path) {
                eprintln!("device no longer present; reconnecting");
                return true;
            }
            false
        };

        if resolved.pid == Some(hid::PID_BLUETOOTH) && natively_handled(&cfg) {
            eprintln!("bluetooth: side buttons are handled natively; idling until the cable or dongle is used");
            drop(device);
            while !device_changed(&mut api) {
                std::thread::sleep(DEVICE_CHECK);
            }
            continue;
        }
        if resolved.pid == Some(hid::PID_BLUETOOTH) {
            eprintln!("bluetooth: note that the native back/forward buttons also fire alongside the custom actions");
        }

        let _source_grab = if args.no_grab_source {
            None
        } else {
            match backend::SourceGrab::acquire(resolved.vid, resolved.pid) {
                Ok(grab) => grab,
                Err(e) => {
                    eprintln!("warning: source grab failed: {e}; continuing without suppression");
                    None
                }
            }
        };

        let report_id = hid::mouse_report_id(resolved.pid);
        let mut buf = vec![0u8; resolved.report_len];
        let mut last_check = Instant::now();
        eprintln!("device connected, listening for events");

        loop {
            match device.read_timeout(&mut buf, READ_TIMEOUT_MS) {
                Ok(0) => {}
                Ok(size) => {
                    for transition in state.update(&cfg, report_id, &buf[..size]) {
                        if let Err(e) = emitter.emit(&transition) {
                            release_all(&mut state, &cfg, &mut emitter);
                            return Err(e);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("device disconnected ({e}); reconnecting");
                    release_all(&mut state, &cfg, &mut emitter);
                    std::thread::sleep(Duration::from_millis(500));
                    break;
                }
            }

            // A wired device reports removal through a read error; wireless
            // links can go quiet instead, and should hand over to the cable.
            if wireless && last_check.elapsed() >= DEVICE_CHECK {
                last_check = Instant::now();
                if device_changed(&mut api) {
                    release_all(&mut state, &cfg, &mut emitter);
                    break;
                }
            }
        }
    }
}

fn main() {
    let cli = Cli::parse();
    let dry_run = cli.dry_run;
    let result = match cli.command {
        Command::List => list_devices(),
        Command::Dump(args) => run_dump(args),
        Command::Run(args) => run_mapper(args),
        Command::Info => configure::info(),
        Command::Battery => configure::show_battery(),
        Command::Dpi(args) => configure::dpi(args, dry_run),
        Command::Rate { hz } => configure::rate(hz, dry_run),
        Command::Led { mode } => configure::led(mode, dry_run),
        Command::RapidFire { ms } => configure::rapid_fire(ms, dry_run),
        Command::Bind(args) => configure::bind(args, dry_run),
        Command::Macro(cmd) => configure::macros(cmd, dry_run),
        Command::Profile(cmd) => configure::profile(cmd, dry_run),
    };

    if let Err(err) = result {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
