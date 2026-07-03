use clap::{Parser, Subcommand};
use hidapi::HidApi;
use std::time::Duration;

mod action;
mod backend;
mod config;
mod hid;

use config::{AppConfig, AppResult, ResolvedMapping, save_config};
use hid::{
    MapperState, RunArgs, device_path_present, format_report, list_devices, mapping_from_args,
    open_device, resolve_run_args, saved_profile_from_args,
};

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

    loop {
        let api = HidApi::new()?;

        let resolved = match resolve_run_args(&api, args.clone()) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("device not found: {e}; retrying in 2s");
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
        };

        let device = match open_device(&api, &resolved) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("failed to open device: {e}; retrying in 2s");
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
        };

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

        if let Some(profile) = saved_profile_from_args(&resolved) {
            let _ = save_config(&AppConfig {
                profile: Some(profile),
            });
        }

        let cfg = mapping_from_args(&resolved).resolve()?;
        let mut buf = vec![0u8; resolved.report_len];
        eprintln!("device connected, listening for events");

        let mut last_refresh = std::time::Instant::now();
        let current_path = resolved.path.clone();
        let mut api = api;
        let mut silent_since: Option<std::time::Instant> = None;
        const SILENT_RECONNECT_AFTER: Duration = Duration::from_secs(5);

        loop {
            if last_refresh.elapsed() > Duration::from_secs(2) {
                last_refresh = std::time::Instant::now();
                let _ = api.refresh_devices();
                let has_wired = api
                    .device_list()
                    .any(|d| d.vendor_id() == 0x248a && d.product_id() == 0x5b49);
                let is_wireless_active =
                    resolved.vid == Some(0x248a) && resolved.pid == Some(0x5b4a);

                if is_wireless_active && has_wired {
                    eprintln!("wired device detected; switching over");
                    release_all(&mut state, &cfg, &mut emitter);
                    break;
                }

                if !device_path_present(&api, &current_path) {
                    eprintln!("current device no longer visible; reconnecting");
                    release_all(&mut state, &cfg, &mut emitter);
                    std::thread::sleep(Duration::from_millis(500));
                    break;
                }
            }

            const POLL_TIMEOUT_MS: i32 = 20;
            match device.read_timeout(&mut buf, POLL_TIMEOUT_MS) {
                Ok(0) => {
                    if let Some(start) = silent_since {
                        if start.elapsed() > SILENT_RECONNECT_AFTER {
                            eprintln!("device went silent; refreshing and reconnecting");
                            let _ = api.refresh_devices();
                            if !device_path_present(&api, &current_path) {
                                release_all(&mut state, &cfg, &mut emitter);
                                std::thread::sleep(Duration::from_millis(500));
                                break;
                            }
                            silent_since = None;
                        }
                    } else {
                        silent_since = Some(std::time::Instant::now());
                    }
                    continue;
                }
                Ok(size) => {
                    silent_since = None;
                    for transition in state.update(&cfg, &buf[..size]) {
                        emitter.emit(&transition)?;
                    }
                }
                Err(e) => {
                    eprintln!("device disconnected ({e}); reconnecting in 500ms");
                    release_all(&mut state, &cfg, &mut emitter);
                    std::thread::sleep(Duration::from_millis(500));
                    break;
                }
            }
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
