use std::time::Duration;

use crate::config::{AppResult, pid_path, read_pid};

#[cfg(unix)]
pub fn kill_and_wait(pid: u32) -> bool {
    let pid_str = pid.to_string();
    let alive = std::process::Command::new("kill")
        .args(["-0", &pid_str])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !alive {
        return true;
    }
    let _ = std::process::Command::new("kill").args(["-9", &pid_str]).status();
    for _ in 0..20 {
        let dead = std::process::Command::new("kill")
            .args(["-0", &pid_str])
            .status()
            .map(|s| !s.success())
            .unwrap_or(true);
        if dead { return true; }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

#[cfg(not(unix))]
pub fn kill_and_wait(pid: u32) -> bool {
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .status();
    std::thread::sleep(Duration::from_millis(500));
    true
}

pub fn stop_existing() {
    if let Ok(pid) = read_pid() {
        if kill_and_wait(pid) {
            eprintln!("stopped previous instance (pid {pid})");
        } else {
            eprintln!("warning: previous instance (pid {pid}) may still be running");
        }
    }
    if let Ok(p) = pid_path() {
        let _ = std::fs::remove_file(p);
    }
}

pub fn restart_daemon() -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        let plist = launchd_plist_path()?;
        if plist.exists() {
            let restarted = match launchd_gui_target() {
                Some(target) => std::process::Command::new("launchctl")
                    .args(["bootstrap", &target, &plist.to_string_lossy()])
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false),
                None => std::process::Command::new("launchctl")
                    .args(["load", "-w", &plist.to_string_lossy()])
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false),
            };
            if restarted {
                println!("restarted chimera-mapper via launchd");
            } else {
                eprintln!("warning: failed to restart via launchd");
            }
            return Ok(());
        }
    }

    #[cfg(target_os = "linux")]
    {
        let service = systemd_service_path()?;
        if service.exists() {
            let _ = std::process::Command::new("systemctl")
                .args(["--user", "start", "chimera-mapper.service"])
                .status();
            println!("restarted chimera-mapper via systemd");
            return Ok(());
        }
    }

    let exe = std::env::current_exe()?;
    let child = std::process::Command::new(exe)
        .arg("run")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    println!("restarted chimera-mapper (pid {})", child.id());
    Ok(())
}

// ── macOS ────────────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
pub fn launchd_gui_target() -> Option<String> {
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| format!("gui/{}", s.trim()))
        .filter(|s| s != "gui/")
}

#[cfg(target_os = "macos")]
pub fn launchd_plist_path() -> AppResult<std::path::PathBuf> {
    let mut base = dirs::home_dir().ok_or("unable to locate home directory")?;
    base.push("Library");
    base.push("LaunchAgents");
    Ok(base.join("com.chimera-mapper.plist"))
}

#[cfg(target_os = "macos")]
pub fn run_install() -> AppResult<()> {
    use crate::config::ensure_parent_dir;
    let exe = std::env::current_exe()?;
    let exe_str = exe.to_string_lossy();
    let log_dir = dirs::home_dir().ok_or("unable to locate home directory")?.join("Library/Logs");
    let log_out = log_dir.join("chimera-mapper.out.log").to_string_lossy().into_owned();
    let log_err = log_dir.join("chimera-mapper.err.log").to_string_lossy().into_owned();

    let plist = format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.chimera-mapper</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe_str}</string>
        <string>run</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>ProcessType</key>
    <string>Interactive</string>
    <key>LimitLoadToSessionType</key>
    <string>Aqua</string>
    <key>StandardOutPath</key>
    <string>{log_out}</string>
    <key>StandardErrorPath</key>
    <string>{log_err}</string>
</dict>
</plist>
"#);

    let path = launchd_plist_path()?;
    ensure_parent_dir(&path)?;
    std::fs::write(&path, plist)?;

    let bootstrapped = match launchd_gui_target() {
        Some(target) => std::process::Command::new("launchctl")
            .args(["bootstrap", &target, &path.to_string_lossy()])
            .status()
            .map(|s| s.success())
            .unwrap_or(false),
        None => false,
    };
    if !bootstrapped {
        eprintln!(
            "warning: launchctl bootstrap failed; try manually: launchctl bootstrap gui/$(id -u) {}",
            path.display()
        );
    }
    println!("installed launchd agent at {}", path.display());
    println!("chimera-mapper will start automatically on login");
    println!("logs: ~/Library/Logs/chimera-mapper.{{out,err}}.log");
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn run_uninstall() -> AppResult<()> {
    let path = launchd_plist_path()?;
    stop_existing();
    if path.exists() {
        let unloaded = match launchd_gui_target() {
            Some(target) => std::process::Command::new("launchctl")
                .args(["bootout", &format!("{target}/com.chimera-mapper")])
                .status()
                .map(|s| s.success())
                .unwrap_or(false),
            None => false,
        };
        if !unloaded {
            let _ = std::process::Command::new("launchctl")
                .args(["unload", "-w", &path.to_string_lossy()])
                .status();
        }
        std::fs::remove_file(&path)?;
        println!("removed launchd agent from {}", path.display());
    } else {
        println!("no launchd agent found at {}", path.display());
    }
    Ok(())
}

// ── Linux ────────────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
pub fn systemd_service_path() -> AppResult<std::path::PathBuf> {
    let mut base = dirs::config_dir().ok_or("unable to locate config directory")?;
    base.push("systemd");
    base.push("user");
    Ok(base.join("chimera-mapper.service"))
}

#[cfg(target_os = "linux")]
pub fn run_install() -> AppResult<()> {
    use crate::config::ensure_parent_dir;
    let exe = std::env::current_exe()?;
    let exe_str = exe.to_string_lossy();
    let service = format!(r#"[Unit]
Description=Chimera Mapper HID daemon

[Service]
ExecStart={exe_str} run
Restart=on-failure

[Install]
WantedBy=default.target
"#);
    let path = systemd_service_path()?;
    ensure_parent_dir(&path)?;
    std::fs::write(&path, service)?;

    let status = std::process::Command::new("systemctl").args(["--user", "daemon-reload"]).status()?;
    if !status.success() { eprintln!("warning: systemctl daemon-reload failed"); }

    let status = std::process::Command::new("systemctl")
        .args(["--user", "enable", "--now", "chimera-mapper.service"])
        .status()?;
    if !status.success() {
        eprintln!("warning: systemctl enable failed; try manually: systemctl --user enable --now chimera-mapper.service");
    }
    println!("installed systemd user service at {}", path.display());
    println!("chimera-mapper will start automatically on login");
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn run_uninstall() -> AppResult<()> {
    let path = systemd_service_path()?;
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "stop", "chimera-mapper.service"])
        .status();
    stop_existing();
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "disable", "chimera-mapper.service"])
        .status();
    if path.exists() {
        std::fs::remove_file(&path)?;
        let _ = std::process::Command::new("systemctl").args(["--user", "daemon-reload"]).status();
        println!("removed systemd service from {}", path.display());
    } else {
        println!("no systemd service found at {}", path.display());
    }
    Ok(())
}

// ── Unsupported ──────────────────────────────────────────────────────────────

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn run_install() -> AppResult<()> {
    Err("auto-start installation is only supported on macOS and Linux".into())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn run_uninstall() -> AppResult<()> {
    Err("auto-start uninstallation is only supported on macOS and Linux".into())
}
