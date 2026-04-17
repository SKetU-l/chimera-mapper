use clap::{Parser, Subcommand};
use hidapi::HidApi;
use std::time::Duration;

mod backend;
mod config;
mod hid;
mod platform;

use config::{AppConfig, AppResult, save_config, write_pid};
use hid::{
    MapperState, RunArgs, default_run_args, detect_and_save, format_report, list_devices,
    mapping_from_args, open_device, resolve_run_args, saved_profile_from_args,
};
use platform::{restart_daemon, run_install, run_uninstall, stop_existing};

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
    Reload,
    Install,
    Uninstall,
}

fn run_dump(args: RunArgs) -> AppResult<()> {
    let api = HidApi::new()?;
    let args = resolve_run_args(&api, args)?;
    let device = open_device(&api, &args)?;
    let cfg = mapping_from_args(&args);
    let mut buf = vec![0u8; args.report_len];

    loop {
        let size = device.read_timeout(&mut buf, args.timeout_ms)?;
        if size == 0 { continue; }
        let report = &buf[..size];
        let byte = report.get(cfg.button_byte).copied().unwrap_or_default();
        println!(
            "report=[{}] byte[{}]=0x{:02x} forward={} back={}",
            format_report(report),
            cfg.button_byte, byte,
            (byte & cfg.side_mask) != 0,
            (byte & cfg.extra_mask) != 0,
        );
    }
}

fn run_mapper(args: RunArgs) -> AppResult<()> {
    let _pid_guard = write_pid()?;
    let mut state = MapperState::default();
    let mut emitter = backend::Emitter::new(&args.name)?;

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

        if let Some(profile) = saved_profile_from_args(&resolved) {
            let _ = save_config(&AppConfig { profile: Some(profile) });
        }

        let cfg = mapping_from_args(&resolved);
        let mut buf = vec![0u8; resolved.report_len];
        eprintln!("device connected, listening for events");

        loop {
            match device.read_timeout(&mut buf, resolved.timeout_ms) {
                Ok(0) => continue,
                Ok(size) => {
                    for transition in state.update(cfg, &buf[..size]) {
                        emitter.emit(transition)?;
                    }
                }
                Err(e) => {
                    eprintln!("device disconnected ({e}); reconnecting in 500ms");
                    for transition in state.synthesize_releases() {
                        let _ = emitter.emit(transition);
                    }
                    std::thread::sleep(Duration::from_millis(500));
                    break;
                }
            }
        }
    }
}

fn run_reload() -> AppResult<()> {
    eprintln!("re-detecting device — press a side button to confirm...");
    let api = HidApi::new()?;
    detect_and_save(&api, &default_run_args())?;
    eprintln!("device profile updated in json");
    stop_existing();
    restart_daemon()
}

fn main() {
    let result = match Cli::parse().command {
        Command::List => list_devices(),
        Command::Dump(args) => run_dump(args),
        Command::Run(args) => run_mapper(args),
        Command::Reload => run_reload(),
        Command::Install => run_install(),
        Command::Uninstall => run_uninstall(),
    };

    if let Err(err) = result {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
