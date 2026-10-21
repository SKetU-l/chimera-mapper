use clap::{Args, Subcommand};
use hidapi::HidApi;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use crate::action::MouseButton;
use crate::config::{AppConfig, AppResult, MacroConfig, load_config, save_config};
use crate::device::{Chimera, Session};
use crate::protocol::{
    self, Binding, DPI_STAGES, LedMode, MACRO_SLOTS, MacroMode, PhysicalButton, PollingRate,
};

#[derive(Args)]
pub struct DpiArgs {
    /// Switch to stage N (1-6) right away, without rewriting flash
    #[arg(long, conflicts_with_all = ["stage", "values"])]
    pub active: Option<usize>,
    /// Set a single stage (1-6) instead of the whole table
    #[arg(long)]
    pub stage: Option<usize>,
    /// Either one DPI value (with --stage) or all six stage values
    pub values: Vec<u32>,
}

#[derive(Args)]
pub struct BindArgs {
    /// left, right, middle, back, forward or dpi
    pub button: Option<String>,
    /// e.g. mouse_back, ctrl+c, alt+left, volume_up, dpi_cycle, macro:0, disabled
    pub action: Option<String>,
    /// Restore the factory button layout
    #[arg(long)]
    pub reset: bool,
}

#[derive(Subcommand)]
pub enum MacroCommand {
    /// Show macros stored on the mouse
    List,
    /// Upload a macro, e.g. `macro set 0 "+ctrl, c, 20ms, -ctrl"`
    Set {
        slot: u8,
        /// Comma-separated steps: `a` taps, `+a` presses, `-a` releases, `50ms` waits
        script: String,
        /// How many times to run per click (with --mode once)
        #[arg(long, default_value_t = 1)]
        repeat: u16,
        #[arg(long, value_enum, default_value_t = MacroMode::Once)]
        mode: MacroMode,
    },
    /// Erase a macro from the mouse
    Remove { slot: u8 },
}

#[derive(Subcommand)]
pub enum ProfileCommand {
    /// Write the current device settings to a JSON file
    Save { file: PathBuf },
    /// Apply every setting in a JSON file in one go
    Load { file: PathBuf },
}

/// A full snapshot of the mouse's on-board configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceProfile {
    pub polling_rate: u32,
    pub dpi: [u32; DPI_STAGES],
    pub led: String,
    /// 1-based, like the CLI
    pub active_stage: usize,
    pub rapid_fire_ms: u32,
    pub buttons: BTreeMap<String, String>,
    #[serde(default)]
    pub macros: BTreeMap<u8, MacroConfig>,
}

fn open() -> AppResult<Chimera> {
    let api = HidApi::new()?;
    Chimera::open(&api)
}

/// Runs `writes` inside a begin/commit session so the change is persisted.
/// Everything that can fail on bad input must be checked before calling this,
/// so a session is never left open half-written.
fn apply(dry_run: bool, writes: impl FnOnce(&Session) -> AppResult<()>) -> AppResult<()> {
    let dev = if dry_run {
        Session::dry_run()
    } else {
        open()?.session()?
    };
    dev.begin()?;
    writes(&dev)?;
    dev.commit()?;
    if !dry_run {
        eprintln!("saved to mouse (it reconnects in about a second)");
    }
    Ok(())
}

/// Reads the DPI table from the mouse when wired. The dongle only echoes
/// whatever was last written through it, so wireless falls back to the copy
/// chimera-mapper keeps of the last table it wrote.
fn read_dpi(dev: &Chimera, config: &AppConfig) -> AppResult<[u32; DPI_STAGES]> {
    if dev.is_wired() {
        return Ok(protocol::decode_dpi(&dev.query(protocol::CMD_DPI)?));
    }
    config.onboard.dpi.ok_or_else(|| {
        format!(
            "the 2.4G dongle can't read the DPI table back; set all {DPI_STAGES} stages once (`dpi A B C D E F`) or connect the cable"
        )
        .into()
    })
}

fn cached_bindings(config: &AppConfig) -> AppResult<[Binding; protocol::BUTTON_COUNT]> {
    let mut out = PhysicalButton::ALL.map(PhysicalButton::default_binding);
    for button in PhysicalButton::ALL {
        if let Some(raw) = config.onboard.buttons.get(button.name()) {
            out[button.slot()] = raw
                .parse()
                .map_err(|e| format!("invalid cached binding for {}: {e}", button.name()))?;
        }
    }
    Ok(out)
}

fn store_bindings(config: &mut AppConfig, bindings: &[Binding; protocol::BUTTON_COUNT]) {
    config.onboard.buttons = PhysicalButton::ALL
        .iter()
        .map(|b| (b.name().to_string(), bindings[b.slot()].to_string()))
        .collect();
}

fn format_dpi(stages: &[u32]) -> String {
    stages
        .iter()
        .map(|d| d.to_string())
        .collect::<Vec<_>>()
        .join(" / ")
}

pub fn info() -> AppResult<()> {
    let dev = open()?;
    let config = load_config()?;
    if dev.is_bluetooth() {
        println!("connection     {}", dev.connection());
        match battery(&dev) {
            Some(b) => println!("battery        {b}"),
            None => println!("battery        unknown"),
        }
        println!("settings       not readable over Bluetooth; connect the cable or dongle to view or change them");
        return Ok(());
    }
    let polling = dev.query(protocol::CMD_POLLING)?[0];
    let stage = dev.query(protocol::CMD_DPI_STAGE)?[0] as usize;
    let led = LedMode(dev.query(protocol::CMD_LED)?[0]);
    let rapid = dev.query(protocol::CMD_RAPID_FIRE)?;
    let rapid_ms = u16::from_le_bytes([rapid[0], rapid[1]]) as u32 * 10;

    println!("connection     {}", dev.connection());
    match battery(&dev) {
        Some(b) => println!("battery        {b}"),
        None => println!("battery        unknown"),
    }
    match PollingRate::from_code(polling) {
        Some(rate) => println!("polling rate   {rate}"),
        None => println!("polling rate   unknown (code {polling})"),
    }
    match read_dpi(&dev, &config) {
        Ok(stages) if dev.is_wired() => println!("dpi stages     {}", format_dpi(&stages)),
        Ok(stages) => println!("dpi stages     {} (last written)", format_dpi(&stages)),
        Err(_) => println!("dpi stages     unknown over 2.4G until set once"),
    }
    println!("active stage   {}", stage + 1);
    println!("lighting       {led}");
    println!("rapid fire     {rapid_ms} ms");
    println!("buttons        (as last written by chimera-mapper)");
    let bindings = cached_bindings(&config)?;
    for button in PhysicalButton::ALL {
        println!("  {:<12} {}", button.name(), bindings[button.slot()]);
    }
    Ok(())
}

fn battery(dev: &Chimera) -> Option<protocol::Battery> {
    if dev.is_bluetooth() {
        // The BT battery service doesn't say whether it's charging, but a
        // charging mouse is on the cable rather than Bluetooth anyway.
        return dev.bluetooth_battery().map(|percent| protocol::Battery {
            percent,
            charging: false,
        });
    }
    let status = dev.query(protocol::CMD_STATUS).ok()?;
    protocol::decode_battery(&status)
}

pub fn show_battery() -> AppResult<()> {
    let dev = open()?;
    let b = battery(&dev).ok_or("the mouse did not report its battery level")?;
    println!("{b}");
    if !dev.is_wired() && !dev.is_bluetooth() {
        eprintln!("(over 2.4G the dongle refreshes this when the mouse is in use)");
    }
    Ok(())
}

fn check_stage(stage: usize) -> AppResult<u8> {
    if !(1..=DPI_STAGES).contains(&stage) {
        return Err(format!("stage must be 1-{DPI_STAGES}").into());
    }
    Ok((stage - 1) as u8)
}

pub fn dpi(args: DpiArgs, dry_run: bool) -> AppResult<()> {
    if let Some(stage) = args.active {
        let idx = check_stage(stage)?;
        if dry_run {
            println!("feature {}", crate::hid::format_report(&protocol::feature(protocol::CMD_DPI_STAGE, &[idx, idx])));
            return Ok(());
        }
        let dev = open()?;
        let dpi = read_dpi(&dev, &load_config()?).ok().map(|d| d[idx as usize]);
        if dev.is_wired() {
            // Live state on the wired link: skip begin/commit and the reboot they cause.
            dev.set_live(protocol::CMD_DPI_STAGE, &[idx, idx])?;
        } else {
            // The dongle only forwards changes to the mouse on commit, which it
            // handles without rebooting.
            let session = dev.session()?;
            session.begin()?;
            session.command(protocol::CMD_DPI_STAGE, &[idx, idx])?;
            session.commit()?;
        }
        match dpi {
            Some(dpi) => println!("active stage   {stage} ({dpi} DPI)"),
            None => println!("active stage   {stage}"),
        }
        return Ok(());
    }
    if args.stage.is_some() && args.values.is_empty() {
        return Err("--stage needs a DPI value, e.g. `dpi --stage 2 1200`".into());
    }
    if args.values.is_empty() {
        let dev = open()?;
        let stages = read_dpi(&dev, &load_config()?)?;
        let active = dev.query(protocol::CMD_DPI_STAGE)?[0] as usize;
        println!("{}", format_dpi(&stages));
        println!("active stage   {}", active + 1);
        return Ok(());
    }
    let stages: [u32; DPI_STAGES] = match args.stage {
        Some(stage) => {
            let idx = check_stage(stage)?;
            if args.values.len() != 1 {
                return Err("--stage takes exactly one DPI value".into());
            }
            let mut current = read_dpi(&open()?, &load_config()?)?;
            current[idx as usize] = args.values[0];
            current
        }
        None => args.values.as_slice().try_into().map_err(|_| {
            format!("give all {DPI_STAGES} stage values, or use --stage N VALUE")
        })?,
    };
    let codes = protocol::encode_dpi(&stages)?;
    apply(dry_run, |dev| dev.command(protocol::CMD_DPI, &codes))?;
    if !dry_run {
        let mut config = load_config()?;
        config.onboard.dpi = Some(stages);
        save_config(&config)?;
    }
    println!("dpi stages     {}", format_dpi(&stages));
    Ok(())
}

pub fn rate(value: Option<String>, dry_run: bool) -> AppResult<()> {
    let Some(value) = value else {
        let code = open()?.query(protocol::CMD_POLLING)?[0];
        match PollingRate::from_code(code) {
            Some(rate) => println!("{rate}"),
            None => println!("unknown (code {code})"),
        }
        return Ok(());
    };
    let rate: PollingRate = value.parse()?;
    apply(dry_run, |dev| dev.command(protocol::CMD_POLLING, &[rate.code()]))?;
    println!("polling rate   {rate}");
    Ok(())
}

pub fn led(value: Option<String>, dry_run: bool) -> AppResult<()> {
    let Some(value) = value else {
        println!("{}", LedMode(open()?.query(protocol::CMD_LED)?[0]));
        return Ok(());
    };
    let mode: LedMode = value.parse()?;
    apply(dry_run, |dev| dev.command(protocol::CMD_LED, &[mode.0]))?;
    println!("lighting       {mode}");
    Ok(())
}

pub fn rapid_fire(ms: Option<u32>, dry_run: bool) -> AppResult<()> {
    let Some(ms) = ms else {
        let raw = open()?.query(protocol::CMD_RAPID_FIRE)?;
        println!("{} ms", u16::from_le_bytes([raw[0], raw[1]]) as u32 * 10);
        return Ok(());
    };
    let payload = protocol::encode_rapid_fire(ms)?;
    apply(dry_run, |dev| dev.command(protocol::CMD_RAPID_FIRE, &payload))?;
    println!("rapid fire     {ms} ms");
    Ok(())
}

pub fn bind(args: BindArgs, dry_run: bool) -> AppResult<()> {
    let config = load_config()?;
    let mut bindings = cached_bindings(&config)?;

    if args.reset {
        bindings = PhysicalButton::ALL.map(PhysicalButton::default_binding);
    } else {
        match (args.button, args.action) {
            (None, _) => {
                for button in PhysicalButton::ALL {
                    println!("{:<12} {}", button.name(), bindings[button.slot()]);
                }
                return Ok(());
            }
            (Some(_), None) => return Err("give an action, e.g. `bind back alt+left`".into()),
            (Some(button), Some(action)) => {
                let button: PhysicalButton = button.parse()?;
                let binding: Binding = action.parse()?;
                if let Binding::Macro(slot) = binding {
                    if !config.onboard.macros.contains_key(&slot) {
                        return Err(format!(
                            "macro slot {slot} is empty; upload one with `macro set {slot} ...`"
                        )
                        .into());
                    }
                }
                bindings[button.slot()] = binding;
                if config.onboard.buttons.is_empty() {
                    eprintln!(
                        "note: no record of the mouse's current bindings, so the other buttons are written with factory defaults"
                    );
                }
            }
        }
    }

    if !bindings.contains(&Binding::Mouse(MouseButton::Left)) {
        return Err("refusing to leave no button acting as left click".into());
    }

    apply(dry_run, |dev| dev.write_buttons(&bindings))?;
    if !dry_run {
        // Reload: the write can take seconds over 2.4G.
        let mut config = load_config()?;
        store_bindings(&mut config, &bindings);
        save_config(&config)?;
    }
    for button in PhysicalButton::ALL {
        println!("{:<12} {}", button.name(), bindings[button.slot()]);
    }
    Ok(())
}

type MacroPackets = Vec<[u8; protocol::OUTPUT_PAYLOAD]>;

fn check_slot(slot: u8) -> AppResult<()> {
    if slot >= MACRO_SLOTS {
        return Err(format!("macro slot must be 0-{}", MACRO_SLOTS - 1).into());
    }
    Ok(())
}

fn macro_packets(slot: u8, cfg: &MacroConfig) -> AppResult<MacroPackets> {
    check_slot(slot)?;
    if cfg.mode == MacroMode::Once && cfg.repeat == 0 {
        return Err("--repeat must be at least 1".into());
    }
    let events = protocol::parse_macro(&cfg.script)?;
    Ok(protocol::encode_macro(slot, cfg.mode, cfg.repeat, &events)?)
}

pub fn macros(cmd: MacroCommand, dry_run: bool) -> AppResult<()> {
    let config = load_config()?;
    match cmd {
        MacroCommand::List => {
            if config.onboard.macros.is_empty() {
                println!("no macros uploaded");
            }
            for (slot, m) in &config.onboard.macros {
                match m.mode {
                    MacroMode::Once => println!("{slot}: {} (x{})", m.script, m.repeat),
                    mode => println!("{slot}: {} ({mode})", m.script),
                }
            }
        }
        MacroCommand::Set {
            slot,
            script,
            repeat,
            mode,
        } => {
            let cfg = MacroConfig {
                script,
                mode,
                repeat,
            };
            let packets = macro_packets(slot, &cfg)?;
            // The button table references macro slots, so resend it with the macro.
            let bindings = cached_bindings(&config)?;
            apply(dry_run, |dev| {
                dev.write_macro(slot, &packets)?;
                dev.write_buttons(&bindings)
            })?;
            if !dry_run {
                let mut config = load_config()?;
                config.onboard.macros.insert(slot, cfg);
                save_config(&config)?;
            }
            println!("macro {slot} uploaded; bind it with `bind <button> macro:{slot}`");
        }
        MacroCommand::Remove { slot } => {
            check_slot(slot)?;
            let bindings = cached_bindings(&config)?;
            if let Some(button) = PhysicalButton::ALL
                .into_iter()
                .find(|b| bindings[b.slot()] == Binding::Macro(slot))
            {
                return Err(format!(
                    "macro {slot} is bound to the {} button; rebind that button first",
                    button.name()
                )
                .into());
            }
            let packets = protocol::encode_macro(slot, MacroMode::Once, 1, &[])?;
            apply(dry_run, |dev| {
                dev.write_macro(slot, &packets)?;
                dev.write_buttons(&bindings)
            })?;
            if !dry_run {
                let mut config = load_config()?;
                config.onboard.macros.remove(&slot);
                save_config(&config)?;
            }
            println!("macro {slot} removed");
        }
    }
    Ok(())
}

pub fn profile(cmd: ProfileCommand, dry_run: bool) -> AppResult<()> {
    match cmd {
        ProfileCommand::Save { file } => {
            let dev = open()?;
            let config = load_config()?;
            let dpi = read_dpi(&dev, &config)?;
            let profile = read_profile(&dev, &config, dpi)?;
            fs::write(&file, serde_json::to_string_pretty(&profile)?)?;
            println!("saved profile to {}", file.display());
        }
        ProfileCommand::Load { file } => {
            let profile: DeviceProfile = serde_json::from_str(&fs::read_to_string(&file)?)?;
            apply_profile(None, &profile, dry_run)?;
            println!("applied profile {}", file.display());
        }
    }
    Ok(())
}

/// Reads every setting the mouse reports, plus the button and macro tables
/// chimera-mapper last wrote (the firmware can't report those back).
fn read_profile(
    dev: &Chimera,
    config: &AppConfig,
    dpi: [u32; DPI_STAGES],
) -> AppResult<DeviceProfile> {
    let rapid = dev.query(protocol::CMD_RAPID_FIRE)?;
    let polling = dev.query(protocol::CMD_POLLING)?[0];
    let bindings = cached_bindings(config)?;
    Ok(DeviceProfile {
        polling_rate: PollingRate::from_code(polling)
            .ok_or(format!("unknown polling code {polling}"))?
            .hz(),
        dpi,
        led: {
            // Keep unknown codes loadable as plain numbers.
            let mode = LedMode(dev.query(protocol::CMD_LED)?[0]);
            mode.name().map_or_else(|| mode.0.to_string(), str::to_owned)
        },
        active_stage: dev.query(protocol::CMD_DPI_STAGE)?[0] as usize + 1,
        rapid_fire_ms: u16::from_le_bytes([rapid[0], rapid[1]]) as u32 * 10,
        buttons: PhysicalButton::ALL
            .iter()
            .map(|b| (b.name().to_string(), bindings[b.slot()].to_string()))
            .collect(),
        macros: config.onboard.macros.clone(),
    })
}

/// Shown for the DPI table over 2.4G until chimera-mapper has written one.
pub const DPI_PLACEHOLDER: [u32; DPI_STAGES] = [800, 1600, 2400, 3200, 4800, 6400];

/// Everything a settings screen needs, read in one go.
pub struct Snapshot {
    pub connection: &'static str,
    pub battery: Option<protocol::Battery>,
    /// `None` over Bluetooth, which has no settings channel.
    pub settings: Option<DeviceProfile>,
    /// False over 2.4G until a DPI table has been written through
    /// chimera-mapper; `settings.dpi` then holds `DPI_PLACEHOLDER`.
    pub dpi_known: bool,
}

/// Connection type and battery level only; cheap enough to poll.
pub fn status() -> AppResult<(&'static str, Option<protocol::Battery>)> {
    let dev = open()?;
    Ok((dev.connection(), battery(&dev)))
}

pub fn snapshot() -> AppResult<Snapshot> {
    let dev = open()?;
    let battery = battery(&dev);
    if dev.is_bluetooth() {
        return Ok(Snapshot {
            connection: dev.connection(),
            battery,
            settings: None,
            dpi_known: false,
        });
    }
    let config = load_config()?;
    let (dpi, dpi_known) = match read_dpi(&dev, &config) {
        Ok(dpi) => (dpi, true),
        Err(_) => (DPI_PLACEHOLDER, false),
    };
    Ok(Snapshot {
        connection: dev.connection(),
        battery,
        settings: Some(read_profile(&dev, &config, dpi)?),
        dpi_known,
    })
}

/// Validates a profile and turns it into what goes on the wire.
struct Prepared {
    rate: PollingRate,
    dpi: [u8; DPI_STAGES],
    led: LedMode,
    rapid: [u8; 2],
    stage: u8,
    bindings: [Binding; protocol::BUTTON_COUNT],
}

fn prepare(profile: &DeviceProfile) -> AppResult<Prepared> {
    let mut bindings = PhysicalButton::ALL.map(PhysicalButton::default_binding);
    for (name, action) in &profile.buttons {
        let button: PhysicalButton = name.parse()?;
        bindings[button.slot()] = action
            .parse()
            .map_err(|e| format!("{} button: {e}", button.name()))?;
    }
    for binding in &bindings {
        if let Binding::Macro(slot) = binding {
            if !profile.macros.contains_key(slot) {
                return Err(format!("a button is bound to macro {slot}, which is empty").into());
            }
        }
    }
    if !bindings.contains(&Binding::Mouse(MouseButton::Left)) {
        return Err("at least one button must stay left click".into());
    }
    Ok(Prepared {
        rate: profile.polling_rate.to_string().parse()?,
        dpi: protocol::encode_dpi(&profile.dpi)?,
        led: profile.led.parse()?,
        rapid: protocol::encode_rapid_fire(profile.rapid_fire_ms)?,
        stage: check_stage(profile.active_stage)?,
        bindings,
    })
}

/// Writes `new` to the mouse in a single save. With `old` (what the mouse
/// holds now), only the settings that differ are sent, which matters over
/// 2.4G where each macro takes seconds. Returns false if nothing changed.
pub fn apply_profile(
    old: Option<&DeviceProfile>,
    new: &DeviceProfile,
    dry_run: bool,
) -> AppResult<bool> {
    let next = prepare(new)?;
    // If the current state doesn't validate (e.g. an unknown LED code),
    // rewrite everything rather than refuse.
    let prev = old.and_then(|o| prepare(o).ok());
    let changed = |f: &dyn Fn(&Prepared) -> bool| prev.as_ref().is_none_or(|p| !f(p));

    let mut macros = Vec::new();
    let slots = new
        .macros
        .keys()
        .chain(old.map(|o| o.macros.keys()).into_iter().flatten());
    for &slot in slots.collect::<std::collections::BTreeSet<_>>() {
        let before = old.and_then(|o| o.macros.get(&slot));
        match new.macros.get(&slot) {
            Some(m) if Some(m) != before => macros.push((
                slot,
                macro_packets(slot, m).map_err(|e| format!("macro {slot}: {e}"))?,
            )),
            None if before.is_some() => {
                macros.push((slot, protocol::encode_macro(slot, MacroMode::Once, 1, &[])?))
            }
            _ => {}
        }
    }
    let buttons = !macros.is_empty() || changed(&|p| p.bindings == next.bindings);
    let led = changed(&|p| p.led == next.led);
    let stage = changed(&|p| p.stage == next.stage);
    let dpi = changed(&|p| p.dpi == next.dpi);
    let rate = changed(&|p| p.rate == next.rate);
    let rapid = changed(&|p| p.rapid == next.rapid);
    if !(buttons || led || stage || dpi || rate || rapid) {
        return Ok(false);
    }

    // Same order as the vendor tool: macros, buttons, LED, stage, DPI, rate, rapid fire.
    apply(dry_run, |dev| {
        for (slot, packets) in &macros {
            dev.write_macro(*slot, packets)?;
        }
        if buttons {
            dev.write_buttons(&next.bindings)?;
        }
        if led {
            dev.command(protocol::CMD_LED, &[next.led.0])?;
        }
        if stage {
            dev.command(protocol::CMD_DPI_STAGE, &[next.stage, next.stage])?;
        }
        if dpi {
            dev.command(protocol::CMD_DPI, &next.dpi)?;
        }
        if rate {
            dev.command(protocol::CMD_POLLING, &[next.rate.code()])?;
        }
        if rapid {
            dev.command(protocol::CMD_RAPID_FIRE, &next.rapid)?;
        }
        Ok(())
    })?;
    if !dry_run {
        let mut config = load_config()?;
        store_bindings(&mut config, &next.bindings);
        config.onboard.macros = new.macros.clone();
        if dpi {
            config.onboard.dpi = Some(new.dpi);
        }
        save_config(&config)?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> DeviceProfile {
        DeviceProfile {
            polling_rate: 1000,
            dpi: [800, 1600, 2400, 3200, 4800, 10000],
            led: "lightshow".into(),
            active_stage: 2,
            rapid_fire_ms: 50,
            buttons: [("back", "alt+left"), ("forward", "macro:0")]
                .iter()
                .map(|(b, a)| (b.to_string(), a.to_string()))
                .collect(),
            macros: [(
                0,
                MacroConfig {
                    script: "h, i".into(),
                    mode: MacroMode::Once,
                    repeat: 1,
                },
            )]
            .into_iter()
            .collect(),
        }
    }

    #[test]
    fn unchanged_profile_writes_nothing() {
        let p = sample();
        assert!(!apply_profile(Some(&p), &p, true).unwrap());
    }

    #[test]
    fn changes_are_written() {
        let old = sample();
        let mut new = old.clone();
        new.led = "off".into();
        assert!(apply_profile(Some(&old), &new, true).unwrap());
    }

    #[test]
    fn rejects_binding_to_missing_macro() {
        let old = sample();
        let mut new = old.clone();
        new.macros.clear();
        assert!(apply_profile(Some(&old), &new, true).is_err());
    }

    #[test]
    fn rejects_losing_left_click() {
        let old = sample();
        let mut new = old.clone();
        new.buttons.insert("left".into(), "disabled".into());
        assert!(apply_profile(Some(&old), &new, true).is_err());
    }
}
