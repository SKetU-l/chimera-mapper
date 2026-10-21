use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use chimera_mapper::config::{AppResult, MacroConfig};
use chimera_mapper::configure::{self, DeviceProfile, Snapshot};
use chimera_mapper::protocol::{
    self, Battery, Binding, LedMode, MACRO_MAX_EVENTS, MACRO_SLOTS, MacroMode, PhysicalButton,
};
use slint::platform::Key;
use slint::{CloseRequestResponse, ComponentHandle, Model, ModelRc, SharedString, VecModel};

slint::include_modules!();

/// How often the connection and battery are re-read while the window is open.
const STATUS_INTERVAL: Duration = Duration::from_secs(5);
/// A wired mouse reboots after a save; give it time before reading it again.
const REBOOT_GRACE: Duration = Duration::from_millis(1500);
/// How long a confirmation stays in the save bar.
const NOTICE_TIME: Duration = Duration::from_secs(3);
/// Lighting previews: frame interval, and how long they play after a change.
const PREVIEW_FRAME: Duration = Duration::from_millis(33);
const PREVIEW_TIME: Duration = Duration::from_secs(4);

const RATES: [u32; 4] = [125, 250, 500, 1000];
const MODES: [MacroMode; 3] = [MacroMode::Once, MacroMode::Hold, MacroMode::Toggle];
const GROUPS: [&str; 6] = ["MOUSE", "NAVIGATION", "SENSITIVITY", "MEDIA", "MACROS", "OTHER"];

/// Choices in the binding sheet: label, binding and group. A binding that
/// matches none of them is shown as a keyboard shortcut.
fn presets() -> Vec<(String, Binding, usize)> {
    let fixed = [
        ("Left click", "mouse_left", 0),
        ("Right click", "mouse_right", 0),
        ("Middle click", "mouse_middle", 0),
        ("Back", "mouse_back", 0),
        ("Forward", "mouse_forward", 0),
        ("Browser back", "alt+left", 1),
        ("Browser forward", "alt+right", 1),
        ("DPI cycle", "dpi_cycle", 2),
        ("DPI up", "dpi_up", 2),
        ("DPI down", "dpi_down", 2),
        ("Play / pause", "play_pause", 3),
        ("Next track", "next_track", 3),
        ("Previous track", "prev_track", 3),
        ("Volume up", "volume_up", 3),
        ("Volume down", "volume_down", 3),
        ("Mute", "mute", 3),
    ];
    let mut out: Vec<_> = fixed
        .iter()
        .map(|&(label, b, group)| (label.to_string(), b.parse().expect("preset parses"), group))
        .collect();
    out.extend((0..MACRO_SLOTS).map(|slot| (format!("Macro {slot}"), Binding::Macro(slot), 4)));
    out.push(("Disabled".to_string(), Binding::Disabled, 5));
    out
}

enum Job {
    /// Read everything from the mouse, replacing the form.
    Load,
    /// Read the connection and battery only.
    Status,
    Apply {
        old: DeviceProfile,
        new: DeviceProfile,
        wired: bool,
    },
}

/// What the mouse currently holds, as last read or written.
#[derive(Default)]
struct State {
    baseline: Option<DeviceProfile>,
    dpi_known: bool,
    connection: &'static str,
    battery: BatteryFilter,
}

/// Readings averaged: a minute at the status poll rate.
const BATTERY_WINDOW: usize = 12;
/// Right after connecting, the mouse reports a stale level (often 100%).
const BATTERY_SETTLE: Duration = Duration::from_secs(3);
/// Readings needed before a level is shown, polled quickly until then.
const BATTERY_MIN_READINGS: usize = 2;
const BATTERY_FAST_POLL: Duration = Duration::from_secs(1);

/// The mouse estimates its charge from the battery voltage, so readings
/// wander a few percent either side of the real level, and right after
/// connecting or plugging in it reports a stale one. Wait for readings to
/// settle, then show the average of the last minute.
struct BatteryFilter {
    since: Instant,
    recent: std::collections::VecDeque<u8>,
    shown: Option<u8>,
    charging: bool,
}

impl Default for BatteryFilter {
    fn default() -> Self {
        BatteryFilter {
            since: Instant::now(),
            recent: Default::default(),
            shown: None,
            charging: false,
        }
    }
}

impl BatteryFilter {
    /// True while readings are still being collected after a change.
    fn settling(&self) -> bool {
        self.shown.is_none()
    }

    fn push(&mut self, reading: Option<Battery>) -> Option<Battery> {
        self.push_at(reading, Instant::now())
    }

    fn push_at(&mut self, reading: Option<Battery>, now: Instant) -> Option<Battery> {
        let Some(b) = reading else {
            // A failed read keeps showing the last level.
            return self.shown.map(|percent| Battery { percent, charging: self.charging });
        };
        if b.charging != self.charging {
            *self = BatteryFilter { since: now, charging: b.charging, ..BatteryFilter::default() };
        }
        if now - self.since < BATTERY_SETTLE {
            return None;
        }
        self.recent.push_back(b.percent);
        if self.recent.len() > BATTERY_WINDOW {
            self.recent.pop_front();
        }
        if self.recent.len() < BATTERY_MIN_READINGS {
            return None;
        }
        let sum: u32 = self.recent.iter().map(|&p| p as u32).sum();
        let n = self.recent.len() as u32;
        let shown = ((sum + n / 2) / n) as u8;
        self.shown = Some(shown);
        Some(Battery { percent: shown, charging: b.charging })
    }
}

#[derive(Clone)]
struct App {
    ui: slint::Weak<AppWindow>,
    jobs: mpsc::Sender<Job>,
    /// Set while a job is queued or running, so polling never piles up.
    in_flight: Arc<AtomicBool>,
    state: Arc<Mutex<State>>,
}

fn set_status(ui: &AppWindow, text: impl Into<SharedString>, error: bool) {
    ui.set_status(text.into());
    ui.set_status_error(error);
}

/// Shows a message that clears itself unless something replaced it.
fn notice(ui: &AppWindow, text: &str) {
    set_status(ui, text, false);
    let weak = ui.as_weak();
    let text = SharedString::from(text);
    slint::Timer::single_shot(NOTICE_TIME, move || {
        if let Some(ui) = weak.upgrade() {
            if ui.get_status() == text {
                set_status(&ui, "", false);
            }
        }
    });
}

fn show_battery(ui: &AppWindow, battery: Option<Battery>) {
    ui.set_battery(battery.map_or(-1, |b| b.percent as i32));
    ui.set_charging(battery.is_some_and(|b| b.charging));
}

impl App {
    fn send(&self, job: Job) -> bool {
        if self.in_flight.swap(true, Ordering::AcqRel) {
            return false;
        }
        if !matches!(job, Job::Status) {
            if let Some(ui) = self.ui.upgrade() {
                ui.set_busy(true);
            }
        }
        self.jobs.send(job).is_ok()
    }

    /// With `demo`, shows sample settings and prints the frames a save
    /// would send instead of talking to a mouse.
    fn run_worker(&self, jobs: mpsc::Receiver<Job>, demo: bool) {
        let app = self.clone();
        std::thread::spawn(move || {
            for job in jobs {
                let app = app.clone();
                match job {
                    Job::Load => {
                        let result = if demo { Ok(demo_snapshot()) } else { configure::snapshot() };
                        let result = result.map_err(|e| e.to_string());
                        app.on_ui(move |app, ui| app.loaded(ui, result));
                    }
                    Job::Status => {
                        let result = if demo {
                            Ok(("wired", demo_snapshot().battery))
                        } else {
                            configure::status()
                        };
                        let result = result.map_err(|e| e.to_string());
                        app.on_ui(move |app, ui| app.status_read(ui, result));
                    }
                    Job::Apply { old, new, wired } => {
                        let result = configure::apply_profile(Some(&old), &new, demo)
                            .map_err(|e| e.to_string());
                        if result.is_ok() && wired && !demo {
                            std::thread::sleep(REBOOT_GRACE);
                        }
                        app.on_ui(move |app, ui| app.applied(ui, new, result));
                    }
                }
            }
        });
    }

    /// Runs `f` on the UI thread once the current job's result is in.
    fn on_ui(self, f: impl FnOnce(&App, &AppWindow) + Send + 'static) {
        let _ = slint::invoke_from_event_loop(move || {
            self.in_flight.store(false, Ordering::Release);
            if let Some(ui) = self.ui.upgrade() {
                ui.set_busy(false);
                f(&self, &ui);
            }
        });
    }

    fn loaded(&self, ui: &AppWindow, result: Result<Snapshot, String>) {
        match result {
            Ok(snap) => {
                ui.set_connection(snap.connection.into());
                let battery = self.read_battery(snap.connection, snap.battery);
                show_battery(ui, battery);
                ui.set_configurable(snap.settings.is_some());
                if snap.settings.is_none() {
                    clear_form(ui);
                }
                let mut state = self.state.lock().unwrap();
                state.baseline = snap.settings;
                state.dpi_known = snap.dpi_known;
                drop(state);
                self.revert(ui);
            }
            Err(e) => self.disconnected(ui, e),
        }
    }

    /// Smooths a battery reading; a new connection starts afresh. While the
    /// level is settling, polls again soon instead of waiting for the timer.
    fn read_battery(&self, connection: &'static str, reading: Option<Battery>) -> Option<Battery> {
        let mut state = self.state.lock().unwrap();
        if state.connection != connection {
            state.connection = connection;
            state.battery = BatteryFilter::default();
        }
        let shown = state.battery.push(reading);
        if state.battery.settling() && reading.is_some() {
            let app = self.clone();
            slint::Timer::single_shot(BATTERY_FAST_POLL, move || {
                app.send(Job::Status);
            });
        }
        shown
    }

    fn disconnected(&self, ui: &AppWindow, error: String) {
        ui.set_connection(SharedString::new());
        show_battery(ui, None);
        ui.set_configurable(false);
        ui.set_dirty(false);
        clear_form(ui);
        let mut state = self.state.lock().unwrap();
        state.baseline = None;
        state.connection = "";
        state.battery = BatteryFilter::default();
        // "Not found" is already shown on the page; only report real errors.
        let missing = error.contains("not found");
        set_status(ui, if missing { String::new() } else { error }, !missing);
    }

    fn status_read(&self, ui: &AppWindow, result: Result<(&'static str, Option<Battery>), String>) {
        match result {
            Ok((connection, battery)) => {
                let battery = self.read_battery(connection, battery);
                show_battery(ui, battery);
                // Plugged in, or switched between cable, dongle and Bluetooth:
                // reload unless that would throw away unsaved edits.
                if ui.get_connection() != connection {
                    if ui.get_dirty() {
                        ui.set_connection(connection.into());
                        set_status(ui, "Connection changed · Reload to re-read (drops your edits)", false);
                    } else {
                        self.send(Job::Load);
                    }
                }
            }
            Err(e) if !ui.get_connection().is_empty() => self.disconnected(ui, e),
            Err(_) => {}
        }
    }

    fn applied(&self, ui: &AppWindow, new: DeviceProfile, result: Result<bool, String>) {
        match result {
            Ok(changed) => {
                // Keep what was written rather than re-reading: the dongle
                // answers from a cache that lags behind writes.
                let mut state = self.state.lock().unwrap();
                state.baseline = Some(new);
                state.dpi_known = true;
                drop(state);
                ui.set_dpi_known(true);
                ui.set_dirty(false);
                ui.set_changes(0);
                notice(ui, if changed { "Saved to the mouse ✓" } else { "Nothing to save" });
                self.send(Job::Status);
            }
            Err(e) => set_status(ui, format!("Not saved: {e}"), true),
        }
    }

    /// Re-derives everything that depends on the form after an edit:
    /// validation, display labels, the change count.
    fn edited(&self, ui: &AppWindow) {
        let Some(baseline) = self.state.lock().unwrap().baseline.clone() else {
            return;
        };
        let presets = presets();

        let macros = ui.get_macros();
        let mut used = 0;
        for (i, mut row) in macros.iter().enumerate() {
            let config = macro_config(&row);
            used += config.is_some() as i32;
            // Mid-recording, keys still held make the script look unbalanced.
            let recording = ui.get_recording() && ui.get_macro_sheet() == i as i32;
            let error = config
                .filter(|_| !recording)
                .map(|m| validate_macro(row.slot as u8, &m).err().unwrap_or_default())
                .unwrap_or_default();
            let steps = macro_steps(&row.script);
            let same_steps = row.steps.row_count() == steps.len()
                && row.steps.iter().zip(&steps).all(|(a, b)| a == *b);
            if row.error != error || !same_steps {
                row.error = error.into();
                if !same_steps {
                    row.steps = ModelRc::new(VecModel::from(steps));
                }
                macros.set_row_data(i, row);
            }
        }
        ui.set_macros_used(used);

        // Macro presets are only offered for slots that hold a macro.
        let preset_model = ui.get_presets();
        for (i, (_, binding, _)) in presets.iter().enumerate() {
            if let Binding::Macro(slot) = binding {
                let enabled = macros
                    .row_data(*slot as usize)
                    .is_some_and(|m| !m.script.trim().is_empty());
                if preset_model.row_data(i).is_some_and(|p| p.enabled != enabled) {
                    update_row(preset_model.clone(), i as i32, |p| p.enabled = enabled);
                }
            }
        }

        let buttons = ui.get_buttons();
        for (i, mut row) in buttons.iter().enumerate() {
            let (binding, error) = match presets.get(row.preset as usize) {
                Some((label, ..)) => (label.clone(), String::new()),
                None if row.custom.trim().is_empty() => {
                    ("Not set".into(), "Record or type a shortcut.".into())
                }
                None => match row.custom.parse::<Binding>() {
                    Ok(b) => (pretty_binding(&b), String::new()),
                    Err(e) => (row.custom.to_string(), e),
                },
            };
            if row.binding != binding || row.error != error {
                row.binding = binding.into();
                row.error = error.into();
                buttons.set_row_data(i, row);
            }
        }

        let form = read_form(ui, &baseline);
        ui.set_changes(count_changes(&baseline, &form) as i32);
        ui.set_dirty(form != baseline);
    }

    fn apply(&self, ui: &AppWindow) {
        let Some(old) = self.state.lock().unwrap().baseline.clone() else {
            return;
        };
        let new = read_form(ui, &old);
        let wired = ui.get_connection() == "wired";
        set_status(ui, if wired { "Saving…" } else { "Saving… a few seconds over 2.4 GHz" }, false);
        self.send(Job::Apply { old, new, wired });
    }

    /// Resets the form to what the mouse holds.
    fn revert(&self, ui: &AppWindow) {
        {
            let state = self.state.lock().unwrap();
            if let Some(baseline) = &state.baseline {
                fill_form(ui, baseline, state.dpi_known);
            }
        }
        set_status(ui, "", false);
        // Derives the labels and resets the change count.
        self.edited(ui);
    }
}

/// How many separate things differ, for the save bar.
fn count_changes(a: &DeviceProfile, b: &DeviceProfile) -> usize {
    let scalars = [
        a.dpi != b.dpi,
        a.active_stage != b.active_stage,
        a.polling_rate != b.polling_rate,
        a.led != b.led,
        a.rapid_fire_ms != b.rapid_fire_ms,
    ];
    let buttons = PhysicalButton::ALL
        .iter()
        .filter(|btn| a.buttons.get(btn.name()) != b.buttons.get(btn.name()))
        .count();
    let macros = (0..MACRO_SLOTS)
        .filter(|slot| a.macros.get(slot) != b.macros.get(slot))
        .count();
    scalars.iter().filter(|&&c| c).count() + buttons + macros
}

fn demo_snapshot() -> Snapshot {
    let buttons = [("back", "alt+left"), ("forward", "alt+right"), ("dpi", "macro:0")];
    let mut buttons: std::collections::BTreeMap<_, _> = buttons
        .iter()
        .map(|(b, a)| (b.to_string(), a.to_string()))
        .collect();
    for button in PhysicalButton::ALL {
        buttons
            .entry(button.name().to_string())
            .or_insert_with(|| button.default_binding().to_string());
    }
    let macros = [
        (0, "+ctrl, c, -ctrl, 50ms, +ctrl, v, -ctrl", MacroMode::Once, 1),
        (3, "g, g, enter", MacroMode::Once, 3),
    ];
    Snapshot {
        connection: "wired",
        battery: Some(Battery { percent: 78, charging: false }),
        settings: Some(DeviceProfile {
            polling_rate: 1000,
            dpi: [800, 1600, 2400, 3200, 4800, 10000],
            led: "lightshow".into(),
            active_stage: 2,
            rapid_fire_ms: 50,
            buttons,
            macros: macros
                .into_iter()
                .map(|(slot, script, mode, repeat)| {
                    (slot, MacroConfig { script: script.into(), mode, repeat })
                })
                .collect(),
        }),
        dpi_known: true,
    }
}

fn macro_config(row: &MacroRow) -> Option<MacroConfig> {
    (!row.script.trim().is_empty()).then(|| MacroConfig {
        script: row.script.to_string(),
        mode: MODES[row.mode as usize],
        repeat: row.repeat as u16,
    })
}

fn validate_macro(slot: u8, m: &MacroConfig) -> Result<(), String> {
    let events = protocol::parse_macro(&m.script)?;
    protocol::encode_macro(slot, m.mode, m.repeat, &events).map(|_| ())
}

/// Display name for a key or modifier as used in scripts and bindings.
fn pretty_key(name: &str) -> String {
    let name = name.trim().to_lowercase();
    let mac = cfg!(target_os = "macos");
    let fixed = match name.as_str() {
        "ctrl" | "control" => "Ctrl",
        "shift" => "Shift",
        "alt" | "option" if mac => "⌥",
        "alt" | "option" => "Alt",
        "meta" | "super" | "command" | "cmd" | "win" if mac => "⌘",
        "meta" | "super" | "command" | "cmd" | "win" => "Super",
        "left" | "arrowleft" => "←",
        "right" | "arrowright" => "→",
        "up" | "arrowup" => "↑",
        "down" | "arrowdown" => "↓",
        "enter" | "return" => "Enter",
        "escape" | "esc" => "Esc",
        "space" => "Space",
        "backspace" => "⌫",
        "tab" => "Tab",
        "delete" | "del" => "Del",
        "leftbracket" => "[",
        "rightbracket" => "]",
        "mouse_left" | "btn_left" => "LMB",
        "mouse_right" | "btn_right" => "RMB",
        "mouse_middle" | "btn_middle" => "MMB",
        "mouse_back" | "btn_side" | "back" => "M4",
        "mouse_forward" | "btn_extra" | "forward" => "M5",
        _ => "",
    };
    if !fixed.is_empty() {
        return fixed.into();
    }
    let mut chars = name.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn pretty_binding(binding: &Binding) -> String {
    match binding {
        Binding::Keys { .. } => binding
            .to_string()
            .split('+')
            .map(pretty_key)
            .collect::<Vec<_>>()
            .join(" + "),
        other => other.to_string(),
    }
}

/// Splits a script into display chips. Works on invalid scripts too, so the
/// chips follow along while the user types.
fn macro_steps(script: &str) -> Vec<Step> {
    script
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|token| {
            let (label, kind) = if let Some(ms) = token.strip_suffix("ms") {
                (format!("{} ms", ms.trim()), 3)
            } else if let Some(key) = token.strip_prefix('+') {
                (format!("{} ↓", pretty_key(key)), 1)
            } else if let Some(key) = token.strip_prefix('-') {
                (format!("{} ↑", pretty_key(key)), 2)
            } else {
                (pretty_key(token), 0)
            };
            Step { label: label.into(), kind }
        })
        .collect()
}

/// Maps the text Slint reports for a key press to a script/binding key name.
fn key_name(text: &str) -> Option<String> {
    let mut chars = text.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    let special = [
        (Key::Return, "enter"),
        (Key::Escape, "esc"),
        (Key::Backspace, "backspace"),
        (Key::Tab, "tab"),
        (Key::Backtab, "tab"),
        (Key::Space, "space"),
        (Key::Delete, "delete"),
        (Key::Insert, "insert"),
        (Key::Home, "home"),
        (Key::End, "end"),
        (Key::PageUp, "pageup"),
        (Key::PageDown, "pagedown"),
        (Key::LeftArrow, "left"),
        (Key::RightArrow, "right"),
        (Key::UpArrow, "up"),
        (Key::DownArrow, "down"),
        (Key::Control, "ctrl"),
        (Key::ControlR, "ctrl"),
        (Key::Shift, "shift"),
        (Key::ShiftR, "shift"),
        (Key::Alt, "alt"),
        (Key::AltGr, "alt"),
        (Key::Meta, "meta"),
        (Key::MetaR, "meta"),
        (Key::F1, "f1"),
        (Key::F2, "f2"),
        (Key::F3, "f3"),
        (Key::F4, "f4"),
        (Key::F5, "f5"),
        (Key::F6, "f6"),
        (Key::F7, "f7"),
        (Key::F8, "f8"),
        (Key::F9, "f9"),
        (Key::F10, "f10"),
        (Key::F11, "f11"),
        (Key::F12, "f12"),
    ];
    if let Some((_, name)) = special.iter().find(|(k, _)| char::from(*k) == c) {
        return Some(name.to_string());
    }
    let name = match c {
        // Ctrl+letter can arrive as a control character.
        '\u{1}'..='\u{1a}' => char::from(b'a' + c as u8 - 1).to_string(),
        'a'..='z' | '0'..='9' => c.to_string(),
        'A'..='Z' => c.to_ascii_lowercase().to_string(),
        // Shifted digits on a US layout.
        '!' => "1".into(),
        '@' => "2".into(),
        '#' => "3".into(),
        '$' => "4".into(),
        '%' => "5".into(),
        '^' => "6".into(),
        '&' => "7".into(),
        '*' => "8".into(),
        '(' => "9".into(),
        ')' => "0".into(),
        '[' | '{' => "leftbracket".into(),
        ']' | '}' => "rightbracket".into(),
        _ => return None,
    };
    Some(name)
}

const MODIFIER_KEYS: [&str; 4] = ["ctrl", "shift", "alt", "meta"];

/// Turns a key press in the shortcut recorder into a binding string, or
/// None until a non-modifier key arrives.
fn shortcut(text: &str, ctrl: bool, alt: bool, shift: bool, meta: bool) -> Option<String> {
    let key = key_name(text).filter(|k| !MODIFIER_KEYS.contains(&k.as_str()))?;
    let mods = [(ctrl, "ctrl"), (shift, "shift"), (alt, "alt"), (meta, "meta")];
    let mut parts: Vec<&str> = mods.iter().filter(|(on, _)| *on).map(|(_, m)| *m).collect();
    parts.push(&key);
    let combo = parts.join("+");
    combo.parse::<Binding>().ok().map(|_| combo)
}

/// Advances the lighting previews' clock while they should play, and stops
/// entirely otherwise so an idle window never redraws.
struct Preview {
    timer: slint::Timer,
    until: std::cell::Cell<Instant>,
}

impl Preview {
    fn kick(self: &Rc<Self>, ui: &slint::Weak<AppWindow>) {
        self.until.set(Instant::now() + PREVIEW_TIME);
        if self.timer.running() {
            return;
        }
        let (this, ui) = (Rc::downgrade(self), ui.clone());
        let mut last = Instant::now();
        self.timer.start(slint::TimerMode::Repeated, PREVIEW_FRAME, move || {
            let (Some(this), Some(ui)) = (this.upgrade(), ui.upgrade()) else { return };
            let now = Instant::now();
            ui.set_led_clock(ui.get_led_clock() + (now - last).as_secs_f32());
            last = now;
            if now >= this.until.get() && !ui.get_led_hover() {
                this.timer.stop();
            }
        });
    }
}

/// Builds a script from live key presses, keeping the pauses between them.
#[derive(Default)]
struct Recorder {
    slot: Option<usize>,
    last: Option<Instant>,
    held: Vec<String>,
    /// One per firmware event: `+key`, `-key` or `Nms`.
    tokens: Vec<String>,
}

impl Recorder {
    fn start(&mut self, slot: usize) {
        *self = Recorder {
            slot: Some(slot),
            ..Recorder::default()
        };
    }

    /// Records a key event at `now`. Returns false when it was ignored.
    fn key(&mut self, text: &str, pressed: bool, now: Instant) -> bool {
        let Some(name) = key_name(text) else {
            return false;
        };
        // Ignore auto-repeat, and releases of keys pressed before recording.
        if pressed == self.held.contains(&name) {
            return false;
        }
        // A macro can't start with a pause.
        if let (Some(last), false) = (self.last, self.tokens.is_empty()) {
            if let Some(delay) = round_delay(now - last) {
                self.tokens.push(format!("{delay}ms"));
            }
        }
        self.last = Some(now);
        if pressed {
            self.held.push(name.clone());
            self.tokens.push(format!("+{name}"));
        } else {
            self.held.retain(|k| *k != name);
            self.tokens.push(format!("-{name}"));
        }
        true
    }

    /// Firmware events the macro will need, counting releases still owed.
    fn events(&self) -> usize {
        self.tokens.len() + self.held.len()
    }

    /// Releases anything still held and returns the finished script.
    fn finish(&mut self) -> String {
        for name in std::mem::take(&mut self.held) {
            self.tokens.push(format!("-{name}"));
        }
        self.slot = None;
        self.script()
    }

    /// The script so far, with press/release pairs folded into taps (the
    /// time a key was held down is dropped).
    fn script(&self) -> String {
        let mut out: Vec<&str> = Vec::new();
        let mut i = 0;
        while i < self.tokens.len() {
            let token = &self.tokens[i];
            if let Some(key) = token.strip_prefix('+') {
                let release = format!("-{key}");
                let next = self.tokens.get(i + 1);
                if next == Some(&release) {
                    out.push(key);
                    i += 2;
                    continue;
                }
                if next.is_some_and(|t| t.ends_with("ms")) && self.tokens.get(i + 2) == Some(&release) {
                    out.push(key);
                    i += 3;
                    continue;
                }
            }
            out.push(token);
            i += 1;
        }
        out.join(", ")
    }
}

/// Rounds a pause to what the firmware can store: 10 ms steps up to 1.27 s,
/// 100 ms steps beyond. Very short pauses are dropped.
fn round_delay(d: Duration) -> Option<u32> {
    let ms = d.as_millis().min(60_000) as u32;
    match ms {
        0..5 => None,
        5..=1270 => Some(((ms + 5) / 10 * 10).max(10)),
        _ => Some((ms + 50) / 100 * 100),
    }
}

/// Empties the form when there are no settings to show (Bluetooth, or no
/// mouse), so nothing looks selected that wasn't read from the mouse.
fn clear_form(ui: &AppWindow) {
    ui.set_dpi(ModelRc::default());
    ui.set_rate_index(-1);
    ui.set_led_index(-1);
    ui.set_active_stage(-1);
    ui.set_macros(ModelRc::default());
    ui.set_macros_used(0);
    let blank = PhysicalButton::ALL.map(|button| ButtonRow {
        name: button_label(button).into(),
        binding: "—".into(),
        preset: -1,
        ..ButtonRow::default()
    });
    ui.set_buttons(ModelRc::new(VecModel::from(blank.to_vec())));
    ui.set_button_sheet(-1);
    ui.set_macro_sheet(-1);
}

/// Puts a profile into the form.
fn fill_form(ui: &AppWindow, p: &DeviceProfile, dpi_known: bool) {
    let presets = presets();

    ui.set_dpi(ModelRc::new(VecModel::from(
        p.dpi.iter().map(|&d| d as i32).collect::<Vec<_>>(),
    )));
    ui.set_dpi_known(dpi_known);
    ui.set_active_stage(p.active_stage as i32 - 1);
    ui.set_rate_index(RATES.iter().position(|&r| r == p.polling_rate).unwrap_or(3) as i32);
    ui.set_led_index(p.led.parse::<LedMode>().map_or(0, |m| m.0 as i32));

    let buttons = PhysicalButton::ALL.map(|button| {
        let binding = p
            .buttons
            .get(button.name())
            .and_then(|raw| raw.parse::<Binding>().ok())
            .unwrap_or_else(|| button.default_binding());
        let preset = presets.iter().position(|(_, b, _)| *b == binding);
        ButtonRow {
            name: button_label(button).into(),
            binding: SharedString::new(),
            preset: preset.unwrap_or(presets.len()) as i32,
            custom: if preset.is_none() { binding.to_string().into() } else { SharedString::new() },
            error: SharedString::new(),
        }
    });
    ui.set_buttons(ModelRc::new(VecModel::from(buttons.to_vec())));

    let macros = (0..MACRO_SLOTS).map(|slot| {
        let m = p.macros.get(&slot);
        MacroRow {
            slot: slot as i32,
            script: m.map(|m| m.script.as_str()).unwrap_or_default().into(),
            mode: m.map_or(0, |m| MODES.iter().position(|&x| x == m.mode).unwrap_or(0) as i32),
            repeat: m.map_or(1, |m| m.repeat as i32),
            error: SharedString::new(),
            steps: ModelRc::default(),
        }
    });
    ui.set_macros(ModelRc::new(VecModel::from(macros.collect::<Vec<_>>())));
    ui.set_button_sheet(-1);
    ui.set_macro_sheet(-1);
}

/// Builds a profile from the form, keeping settings the form doesn't show.
fn read_form(ui: &AppWindow, baseline: &DeviceProfile) -> DeviceProfile {
    let presets = presets();
    let mut p = baseline.clone();
    for (slot, dpi) in ui.get_dpi().iter().enumerate().take(p.dpi.len()) {
        p.dpi[slot] = dpi as u32;
    }
    p.active_stage = ui.get_active_stage() as usize + 1;
    p.polling_rate = RATES[ui.get_rate_index() as usize];
    p.led = LedMode(ui.get_led_index() as u8)
        .name()
        .unwrap_or("off")
        .to_string();
    p.buttons = PhysicalButton::ALL
        .iter()
        .zip(ui.get_buttons().iter())
        .map(|(button, row)| {
            let action = match presets.get(row.preset as usize) {
                Some((_, binding, _)) => binding.to_string(),
                None => row.custom.trim().to_string(),
            };
            (button.name().to_string(), action)
        })
        .collect();
    p.macros = ui
        .get_macros()
        .iter()
        .filter_map(|row| Some((row.slot as u8, macro_config(&row)?)))
        .collect();
    p
}

fn button_label(button: PhysicalButton) -> &'static str {
    match button {
        PhysicalButton::Left => "LEFT",
        PhysicalButton::Right => "RIGHT",
        PhysicalButton::Middle => "WHEEL",
        PhysicalButton::Back => "BACK SIDE",
        PhysicalButton::Forward => "FRONT SIDE",
        PhysicalButton::Dpi => "DPI BUTTON",
    }
}

/// Updates one row of a model in place.
fn update_row<T: Clone + 'static>(model: ModelRc<T>, i: i32, f: impl FnOnce(&mut T)) {
    let i = i as usize;
    if let Some(mut row) = model.row_data(i) {
        f(&mut row);
        model.set_row_data(i, row);
    }
}

fn set_macro_script(ui: &AppWindow, slot: i32, script: String) {
    update_row(ui.get_macros(), slot, |row| row.script = script.into());
}

/// Fills the fixed lists: binding presets and their groups.
fn init_presets(ui: &AppWindow) {
    let presets = presets();
    let mut in_group = [0; GROUPS.len()];
    let preset_rows: Vec<Preset> = presets
        .iter()
        .map(|(label, _, group)| {
            in_group[*group] += 1;
            Preset {
                label: label.into(),
                group: *group as i32,
                pos: in_group[*group] - 1,
                enabled: true,
            }
        })
        .collect();
    ui.set_presets(ModelRc::new(VecModel::from(preset_rows)));
    ui.set_groups(ModelRc::new(VecModel::from(
        GROUPS
            .iter()
            .zip(in_group)
            .map(|(title, count)| Group { title: (*title).into(), count })
            .collect::<Vec<_>>(),
    )));
}

fn main() -> AppResult<()> {
    let ui = AppWindow::new()?;
    init_presets(&ui);

    let (jobs, receiver) = mpsc::channel();
    let app = App {
        ui: ui.as_weak(),
        jobs,
        in_flight: Arc::new(AtomicBool::new(false)),
        state: Arc::default(),
    };
    app.run_worker(receiver, std::env::args().any(|a| a == "--demo"));
    let recorder = Rc::new(RefCell::new(Recorder::default()));
    wire_callbacks(&ui, &app, &recorder);
    let preview = Rc::new(Preview {
        timer: slint::Timer::default(),
        until: std::cell::Cell::new(Instant::now()),
    });
    {
        let (preview, weak) = (preview.clone(), ui.as_weak());
        ui.on_preview(move || preview.kick(&weak));
    }
    preview.kick(&ui.as_weak());

    // Closing mid-save would cut a session short; wait for it to finish.
    {
        let weak = ui.as_weak();
        ui.window().on_close_requested(move || match weak.upgrade() {
            Some(ui) if ui.get_busy() && ui.get_dirty() => {
                set_status(&ui, "Wait for the save to finish before closing", true);
                CloseRequestResponse::KeepWindowShown
            }
            _ => CloseRequestResponse::HideWindow,
        });
    }

    let poll = slint::Timer::default();
    {
        let app = app.clone();
        poll.start(slint::TimerMode::Repeated, STATUS_INTERVAL, move || {
            app.send(Job::Status);
        });
    }

    app.send(Job::Load);
    ui.run()?;
    Ok(())
}

fn wire_callbacks(ui: &AppWindow, app: &App, recorder: &Rc<RefCell<Recorder>>) {
    // Every callback gets the window back from a weak handle; callbacks only
    // run while the window exists.
    macro_rules! on {
        ($setter:ident, |$app:ident, $ui:ident $(, $arg:ident)*| $body:expr) => {{
            let $app = app.clone();
            ui.$setter(move |$($arg),*| {
                let $ui = $app.ui.upgrade().expect("callbacks run while the window exists");
                let $ui = &$ui;
                $body
            });
        }};
    }

    on!(on_refresh, |app, _ui| {
        app.send(Job::Load);
    });
    on!(on_apply, |app, ui| app.apply(ui));
    on!(on_revert, |app, ui| app.revert(ui));
    on!(on_edited, |app, ui| app.edited(ui));
    on!(on_dpi_changed, |app, ui, i, value| {
        // Drags are continuous; the mouse takes steps of 100.
        let dpi = ((value / 100.0).round() as i32 * 100).clamp(200, 10_000);
        let model = ui.get_dpi();
        if model.row_data(i as usize) != Some(dpi) {
            model.set_row_data(i as usize, dpi);
            app.edited(ui);
        }
    });
    on!(on_button_preset, |app, ui, i, preset| {
        update_row(ui.get_buttons(), i, |row| row.preset = preset);
        app.edited(ui);
    });
    on!(on_button_custom, |app, ui, i, text| {
        let custom = presets().len() as i32;
        update_row(ui.get_buttons(), i, |row| {
            row.preset = custom;
            row.custom = text;
        });
        app.edited(ui);
    });
    on!(on_record_shortcut, |app, ui, i, text, ctrl, alt, shift, meta| {
        let Some(combo) = shortcut(&text, ctrl, alt, shift, meta) else {
            return SharedString::new();
        };
        // A recorded combo that matches a preset selects the preset.
        let binding: Binding = combo.parse().expect("shortcut() validated it");
        let presets = presets();
        let preset = presets.iter().position(|(_, b, _)| *b == binding);
        update_row(ui.get_buttons(), i, |row| {
            row.preset = preset.unwrap_or(presets.len()) as i32;
            row.custom = if preset.is_none() { combo.clone().into() } else { SharedString::new() };
        });
        app.edited(ui);
        combo.into()
    });
    on!(on_macro_script, |app, ui, i, text| {
        set_macro_script(ui, i, text.into());
        app.edited(ui);
    });
    on!(on_macro_mode, |app, ui, i, mode| {
        update_row(ui.get_macros(), i, |row| row.mode = mode);
        app.edited(ui);
    });
    on!(on_macro_repeat, |app, ui, i, repeat| {
        update_row(ui.get_macros(), i, |row| row.repeat = repeat.clamp(1, 999));
        app.edited(ui);
    });
    on!(on_new_macro, |_app, ui| {
        ui.get_macros()
            .iter()
            .position(|m| m.script.trim().is_empty())
            .map_or(-1, |i| i as i32)
    });

    // The recorder lives on the UI thread, outside `App`, which the worker shares.
    let (a, rec) = (app.clone(), recorder.clone());
    ui.on_start_recording(move |i| {
        let Some(ui) = a.ui.upgrade() else { return };
        rec.borrow_mut().start(i as usize);
        set_macro_script(&ui, i, String::new());
        a.edited(&ui);
    });
    let (a, rec) = (app.clone(), recorder.clone());
    ui.on_stop_recording(move |i| {
        let Some(ui) = a.ui.upgrade() else { return };
        let mut r = rec.borrow_mut();
        if r.slot == Some(i as usize) {
            let script = r.finish();
            drop(r);
            set_macro_script(&ui, i, script);
            a.edited(&ui);
        }
    });
    let (a, rec) = (app.clone(), recorder.clone());
    ui.on_record_key(move |i, text, pressed| {
        let Some(ui) = a.ui.upgrade() else { return };
        let mut r = rec.borrow_mut();
        if r.slot != Some(i as usize) || !r.key(&text, pressed, Instant::now()) {
            return;
        }
        // Stop before the macro outgrows the mouse's memory.
        let full = r.events() >= MACRO_MAX_EVENTS;
        let script = if full { r.finish() } else { r.script() };
        drop(r);
        set_macro_script(&ui, i, script);
        if full {
            ui.set_recording(false);
            set_status(&ui, "Macro full: the mouse stores up to 71 steps", true);
        }
        a.edited(&ui);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window on Slint's headless test platform, plus an app whose worker
    /// never runs (jobs just queue up).
    fn setup() -> (AppWindow, App, DeviceProfile) {
        i_slint_backend_testing::init_no_event_loop();
        let ui = AppWindow::new().unwrap();
        init_presets(&ui);
        let (jobs, receiver) = mpsc::channel();
        std::mem::forget(receiver);
        let app = App {
            ui: ui.as_weak(),
            jobs,
            in_flight: Arc::default(),
            state: Arc::default(),
        };
        let profile = demo_snapshot().settings.unwrap();
        app.state.lock().unwrap().baseline = Some(profile.clone());
        app.revert(&ui);
        (ui, app, profile)
    }

    #[test]
    fn form_round_trips_profile() {
        let (ui, _, profile) = setup();
        assert_eq!(read_form(&ui, &profile), profile);
        assert!(!ui.get_dirty());
        assert_eq!(ui.get_changes(), 0);
        assert_eq!(ui.get_macros_used(), 2);
    }

    #[test]
    fn edits_count_and_revert_restores() {
        let (ui, app, profile) = setup();
        ui.set_led_index(0);
        update_row(ui.get_buttons(), 3, |row| row.preset = 0);
        app.edited(&ui);
        assert!(ui.get_dirty());
        assert_eq!(ui.get_changes(), 2);
        assert_eq!(read_form(&ui, &profile).led, "off");
        app.revert(&ui);
        assert!(!ui.get_dirty());
        assert_eq!(read_form(&ui, &profile), profile);
    }

    #[test]
    fn binding_labels_and_custom_validation() {
        let (ui, app, profile) = setup();
        let back = ui.get_buttons().row_data(3).unwrap();
        assert_eq!(back.binding, "Browser back");
        let custom = presets().len() as i32;
        update_row(ui.get_buttons(), 2, |row| {
            row.preset = custom;
            row.custom = "ctrl+nope".into();
        });
        app.edited(&ui);
        assert_ne!(ui.get_buttons().row_data(2).unwrap().error, "");
        update_row(ui.get_buttons(), 2, |row| row.custom = "ctrl+shift+t".into());
        app.edited(&ui);
        let row = ui.get_buttons().row_data(2).unwrap();
        assert_eq!(row.error, "");
        assert_eq!(row.binding, "Ctrl + Shift + T");
        assert_eq!(read_form(&ui, &profile).buttons["middle"], "ctrl+shift+t");
    }

    #[test]
    fn macro_presets_follow_defined_macros() {
        let (ui, app, _) = setup();
        let enabled = |slot: u8| {
            let i = presets().iter().position(|(_, b, _)| *b == Binding::Macro(slot)).unwrap();
            ui.get_presets().row_data(i).unwrap().enabled
        };
        assert!(enabled(0) && enabled(3) && !enabled(1));
        set_macro_script(&ui, 1, "a".into());
        app.edited(&ui);
        assert!(enabled(1));
    }

    #[test]
    fn macros_validate_and_show_steps() {
        let (ui, app, profile) = setup();
        let steps: Vec<_> = ui.get_macros().row_data(0).unwrap().steps.iter().collect();
        assert_eq!(steps.len(), 7);
        assert_eq!(steps[0], Step { label: "Ctrl ↓".into(), kind: 1 });
        assert_eq!(steps[3], Step { label: "50 ms".into(), kind: 3 });
        set_macro_script(&ui, 1, "50ms, a".into());
        app.edited(&ui);
        assert_ne!(ui.get_macros().row_data(1).unwrap().error, "");
        set_macro_script(&ui, 3, "  ".into());
        assert!(!read_form(&ui, &profile).macros.contains_key(&3));
    }

    #[test]
    fn shortcuts_from_key_events() {
        let left: SharedString = Key::LeftArrow.into();
        assert_eq!(shortcut(&left, false, true, false, false).as_deref(), Some("alt+left"));
        assert_eq!(shortcut("T", true, false, true, false).as_deref(), Some("ctrl+shift+t"));
        assert_eq!(shortcut("\u{14}", true, false, false, false).as_deref(), Some("ctrl+t"));
        let ctrl: SharedString = Key::Control.into();
        assert_eq!(shortcut(&ctrl, true, false, false, false), None);
        assert_eq!(shortcut("é", false, false, false, false), None);
    }

    #[test]
    fn recorder_folds_taps_and_keeps_pauses() {
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        let ctrl: SharedString = Key::Control.into();
        let mut rec = Recorder::default();
        rec.start(0);
        assert!(rec.key(&ctrl, true, at(0)));
        assert!(!rec.key(&ctrl, true, at(30)), "auto-repeat is ignored");
        rec.key("c", true, at(40));
        rec.key("c", false, at(90));
        rec.key(&ctrl, false, at(120));
        rec.key("v", true, at(600));
        rec.key("v", false, at(650));
        assert_eq!(rec.finish(), "+ctrl, 40ms, c, 30ms, -ctrl, 480ms, v");
        assert!(protocol::parse_macro(&rec.script()).is_ok());
    }

    #[test]
    fn recorder_releases_held_keys_and_skips_stray_releases() {
        let mut rec = Recorder::default();
        rec.start(0);
        let now = Instant::now();
        let shift: SharedString = Key::Shift.into();
        assert!(!rec.key("a", false, now), "released before recording started");
        rec.key(&shift, true, now);
        rec.key("a", true, now);
        assert_eq!(rec.events(), 4, "two presses plus the releases still owed");
        assert_eq!(rec.finish(), "+shift, +a, -shift, -a");
    }

    /// Renders the demo window to a PPM image without a display, to check
    /// layouts at sizes a tiling window manager won't give.
    /// `CHIMERA_SHOT=out.ppm CHIMERA_SHOT_SIZE=1120x800 cargo test -p chimera-mapper-gui -- --ignored render`
    #[test]
    #[ignore = "writes an image; needs its own process"]
    fn render() {
        use slint::platform::software_renderer::{
            MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
        };
        struct Headless(Rc<MinimalSoftwareWindow>);
        impl slint::platform::Platform for Headless {
            fn create_window_adapter(
                &self,
            ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
                Ok(self.0.clone())
            }
        }
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        slint::platform::set_platform(Box::new(Headless(window.clone()))).unwrap();

        let size = std::env::var("CHIMERA_SHOT_SIZE").unwrap_or("1120x800".into());
        let (w, h) = size.split_once('x').unwrap();
        let (w, h): (usize, usize) = (w.parse().unwrap(), h.parse().unwrap());
        let ui = AppWindow::new().unwrap();
        init_presets(&ui);
        let (jobs, receiver) = mpsc::channel();
        std::mem::forget(receiver);
        let app = App { ui: ui.as_weak(), jobs, in_flight: Arc::default(), state: Arc::default() };
        app.loaded(&ui, Ok(demo_snapshot()));
        if let Ok(sheet) = std::env::var("CHIMERA_SHOT_SHEET") {
            let (kind, n) = sheet.split_once(':').unwrap();
            let n = n.parse().unwrap();
            if kind == "b" { ui.set_button_sheet(n) } else { ui.set_macro_sheet(n) }
        }
        ui.set_led_clock(1.3);
        window.set_size(slint::PhysicalSize::new(w as u32, h as u32));
        ui.show().unwrap();
        let mut pixels = vec![PremultipliedRgbaColor::default(); w * h];
        window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, w);
        });
        let mut ppm = format!("P6 {w} {h} 255\n").into_bytes();
        ppm.extend(pixels.iter().flat_map(|p| [p.red, p.green, p.blue]));
        std::fs::write(std::env::var("CHIMERA_SHOT").unwrap_or("/tmp/chimera.ppm".into()), ppm).unwrap();
    }

    #[test]
    fn battery_settles_then_averages() {
        let t0 = Instant::now();
        let mut f = BatteryFilter { since: t0, ..BatteryFilter::default() };
        let mut read = |s: u64, p| {
            let b = Battery { percent: p, charging: false };
            f.push_at(Some(b), t0 + Duration::from_secs(s)).map(|b| b.percent)
        };
        assert_eq!(read(0, 100), None, "the stale reading at connect is dropped");
        assert_eq!(read(3, 89), None, "waits for a second reading");
        assert_eq!(read(4, 90), Some(90));
        // The real pattern seen over Bluetooth: it stays in the middle
        // instead of following each jump or drifting to the lowest.
        for (s, p) in [(9, 89), (14, 91), (19, 88), (24, 90), (29, 92), (34, 88)] {
            let shown = read(s, p).unwrap();
            assert!((89..=90).contains(&shown), "at {s}s: {shown}");
        }
    }

    #[test]
    fn plugging_in_starts_over() {
        let t0 = Instant::now();
        let mut f = BatteryFilter { since: t0, ..BatteryFilter::default() };
        let at = |s| t0 + Duration::from_secs(s);
        f.push_at(Some(Battery { percent: 90, charging: false }), at(5));
        f.push_at(Some(Battery { percent: 90, charging: false }), at(6));
        let charging = Battery { percent: 86, charging: true };
        assert_eq!(f.push_at(Some(charging), at(7)), None);
        assert_eq!(f.push_at(Some(charging), at(10)), None);
        assert_eq!(f.push_at(Some(charging), at(11)), Some(charging));
        assert_eq!(f.push_at(None, at(12)), Some(charging), "a failed read keeps the level");
    }

    #[test]
    fn delays_round_to_firmware_steps() {
        let ms = Duration::from_millis;
        assert_eq!(round_delay(ms(3)), None);
        assert_eq!(round_delay(ms(7)), Some(10));
        assert_eq!(round_delay(ms(1264)), Some(1260));
        assert_eq!(round_delay(ms(1449)), Some(1400));
    }
}
