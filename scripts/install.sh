#!/usr/bin/env bash
set -euo pipefail

BIN_NAME="chimera-mapper"
SERVICE_LABEL="com.sketu.chimera-mapper"
REPO_OWNER="SKetU-l"
REPO_NAME="chimera-mapper"
REPO_URL="https://github.com/${REPO_OWNER}/${REPO_NAME}.git"
LINUX_MODULES_LOAD="/etc/modules-load.d/${BIN_NAME}.conf"
LINUX_UDEV_RULES="/etc/udev/rules.d/99-${BIN_NAME}.rules"

G='\033[0;32m' Y='\033[0;33m' R='\033[0;31m' B='\033[1m' D='\033[90m' N='\033[0m'
status() { echo -e "${G}✓${N} $1"; }
step()   { echo -e "\n${B}$1${N}"; }
info()   { echo -e "${D}  $1${N}"; }
warn()   { echo -e "${Y}!${N} $1"; }
error()  { echo -e "${R}✗${N} $1" >&2; }

detect_os()   { case "$(uname -s)" in Darwin) echo macos;; Linux) echo linux;; *) error "Unsupported OS"; exit 1;; esac; }
detect_arch() { case "$(uname -m)" in x86_64|amd64) echo x86_64;; aarch64|arm64) echo aarch64;; *) error "Unsupported arch"; exit 1;; esac; }

ensure_git() {
  command -v git &>/dev/null && return
  step "Installing git"
  case "$(detect_os)" in
    macos) brew install git ;;
    linux)
      if command -v apt-get &>/dev/null; then sudo apt-get install -y git
      elif command -v dnf &>/dev/null;     then sudo dnf install -y git
      elif command -v pacman &>/dev/null;  then sudo pacman -S --noconfirm git
      else error "Cannot install git: no known package manager"; exit 1
      fi ;;
  esac
  status "git installed"
}

ensure_rust() {
  if command -v cargo &>/dev/null; then
    status "Found system-wide cargo"
    return
  fi

  step "Installing rustup"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
  status "rustup installed"

  local env_path="${CARGO_HOME:-$HOME/.cargo}/env"
  if [[ -f "$env_path" ]]; then
    source "$env_path"
  else
    warn "Cargo environment file not found at $env_path. You may need to add cargo to your PATH (e.g. 'export PATH=\"\$HOME/.cargo/bin:\$PATH\"') or restart your shell."
  fi
}

build_from_source() {
  local install_dir="$1" tmp
  tmp=$(mktemp -d)
  trap "rm -rf '$tmp'" EXIT

  info "Cloning $REPO_URL" >&2
  git clone --depth=1 "$REPO_URL" "$tmp" >&2

  step "Compiling (this may take a while)" >&2
  if command -v cargo &>/dev/null; then
    cargo build --release --manifest-path "$tmp/Cargo.toml" >&2
  else
    local env_path="${CARGO_HOME:-$HOME/.cargo}/env"
    if [[ -f "$env_path" ]]; then
      source "$env_path"
      cargo build --release --manifest-path "$tmp/Cargo.toml" >&2
    else
      error "Cargo not found." >&2
      return 1
    fi
  fi

  local src="$tmp/target/release/${BIN_NAME}"
  [[ -f "$src" ]] || { error "Build failed: binary not found" >&2; return 1; }

  if [[ ! -w "$(dirname "$install_dir")" ]] || [[ ! -w "$install_dir" && -d "$install_dir" ]]; then
    sudo mkdir -p "$install_dir"
    sudo cp "$src" "${install_dir}/${BIN_NAME}" && sudo chmod +x "${install_dir}/${BIN_NAME}"
  else
    mkdir -p "$install_dir"
    cp "$src" "${install_dir}/${BIN_NAME}" && chmod +x "${install_dir}/${BIN_NAME}"
  fi

  echo "${install_dir}/${BIN_NAME}"
}

ensure_linux_input_runtime() {
  step "Preparing Linux input runtime"

  if [[ ! -e /dev/uinput ]] && command -v modprobe &>/dev/null; then
    sudo modprobe uinput 2>/dev/null || true
  fi

  if [[ ! -e /dev/uinput ]]; then
    error "/dev/uinput is not available. Chimera Mapper cannot create a virtual keyboard/mouse without the Linux uinput module."
    error "Try installing/loading your kernel's uinput module, then rerun this installer."
    return 1
  fi

  if ! getent group input >/dev/null; then
    sudo groupadd --system input
  fi

  echo "uinput" | sudo tee "$LINUX_MODULES_LOAD" >/dev/null
  sudo tee "$LINUX_UDEV_RULES" >/dev/null <<-RULES
	# Allow Chimera Mapper to create virtual input events and read the Chimera mouse.
	KERNEL=="uinput", MODE="0660", GROUP="input", OPTIONS+="static_node=uinput"
	KERNEL=="hidraw*", ATTRS{idVendor}=="248a", ATTRS{idProduct}=="5b49", MODE="0660", GROUP="input"
	KERNEL=="hidraw*", ATTRS{idVendor}=="248a", ATTRS{idProduct}=="5b4a", MODE="0660", GROUP="input"
	RULES

  sudo usermod -aG input "${USER}"
  sudo udevadm control --reload-rules 2>/dev/null || true
  sudo udevadm trigger 2>/dev/null || true

  if command -v setfacl &>/dev/null; then
    sudo setfacl -m "u:${USER}:rw" /dev/uinput 2>/dev/null || true
    for dev in /dev/hidraw*; do
      [[ -e "$dev" ]] || continue
      if udevadm info -q property -n "$dev" 2>/dev/null | grep -Eq '^(ID_VENDOR_ID|HID_ID)=.*248a'; then
        sudo setfacl -m "u:${USER}:rw" "$dev" 2>/dev/null || true
      fi
    done
  fi

  status "uinput and udev rules configured"
}

install_macos_service() {
  local bin="$1" plist="${HOME}/Library/LaunchAgents/${SERVICE_LABEL}.plist"
  mkdir -p "$(dirname "$plist")"
  cat > "$plist" <<-PLIST
	<?xml version="1.0" encoding="UTF-8"?>
	<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
	<plist version="1.0"><dict>
	  <key>Label</key><string>$SERVICE_LABEL</string>
	  <key>ProgramArguments</key><array><string>$bin</string><string>run</string></array>
	  <key>RunAtLoad</key><true/><key>KeepAlive</key><true/>
	  <key>StandardOutPath</key><string>$HOME/Library/Logs/chimera-mapper.log</string>
	  <key>StandardErrorPath</key><string>$HOME/Library/Logs/chimera-mapper.err.log</string>
	</dict></plist>
	PLIST
  launchctl bootstrap "gui/$(id -u)" "$plist" 2>/dev/null || launchctl load "$plist" 2>/dev/null || true
  status "Auto-start enabled"
}

install_linux_service() {
  local bin="$1" system_service="/etc/systemd/system/${SERVICE_LABEL}.service"
  local user_service="${HOME}/.config/systemd/user/${SERVICE_LABEL}.service"
  local user_config_home="${HOME}/.config"

  ensure_linux_input_runtime

  step "Creating system service"

  systemctl --user stop         "${SERVICE_LABEL}" 2>/dev/null || true
  systemctl --user disable      "${SERVICE_LABEL}" 2>/dev/null || true
  systemctl --user reset-failed "${SERVICE_LABEL}" 2>/dev/null || true
  [[ -f "$user_service" ]] && rm -f "$user_service"
  systemctl --user daemon-reload 2>/dev/null || true

  sudo systemctl stop         "${SERVICE_LABEL}" 2>/dev/null || true
  sudo systemctl disable      "${SERVICE_LABEL}" 2>/dev/null || true
  sudo systemctl reset-failed "${SERVICE_LABEL}" 2>/dev/null || true

  sudo tee "$system_service" > /dev/null <<-SERVICE
	[Unit]
	Description=Chimera Mapper
	Wants=systemd-modules-load.service systemd-udevd.service
	After=systemd-modules-load.service systemd-udevd.service
	StartLimitIntervalSec=0

	[Service]
	Type=simple
	Environment=XDG_CONFIG_HOME=$user_config_home
	ExecStartPre=/bin/sh -c 'modprobe uinput 2>/dev/null || true'
	ExecStartPre=/bin/sh -c 'udevadm settle --timeout=10 2>/dev/null || true'
	ExecStartPre=/bin/sh -c 'test -e /dev/uinput'
	ExecStart=$bin run
	Restart=always
	RestartSec=2
	User=root
	StandardOutput=journal
	StandardError=journal

	[Install]
	WantedBy=multi-user.target
	SERVICE

  sudo systemctl daemon-reload
  sudo systemctl enable "${SERVICE_LABEL}"
  sudo systemctl restart "${SERVICE_LABEL}"
  status "System-level auto-start enabled"
}

main() {
  local os=$(detect_os)
  local install_dir="${HOME}/.local/bin"
  [[ "$os" == "linux" ]] && install_dir="/usr/local/bin"

  local skip_service=false arch bin
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --install-dir) install_dir="$2"; shift ;;
      --skip-service) skip_service=true ;;
      -h|--help) echo "Usage: $0 [--install-dir PATH] [--skip-service]"; exit 0 ;;
      *) error "Unknown option: $1"; exit 1 ;;
    esac
    shift
  done

  arch=$(detect_arch)

  step "Setting up Chimera Mapper"
  info "System: $(uname -s) ($arch)"
  info "Install location: $install_dir"

  ensure_git
  ensure_rust

  bin=$(build_from_source "$install_dir") || { error "Installation failed"; exit 1; }
  status "Build complete → $bin"

  if [[ "$skip_service" != "true" ]]; then
    step "Configuring auto-start"
    [[ "$os" == "macos" ]] && install_macos_service "$bin" || install_linux_service "$bin"
  fi

  sleep 2

  step "Verifying"
  if [[ "$os" == "macos" ]]; then
    if launchctl list | grep -q "$SERVICE_LABEL"; then
      status "Service is running"
    else
      warn "Service may need a moment to start"
    fi
  else
    if systemctl is-active "$SERVICE_LABEL" &>/dev/null; then
      status "Service is running"
    else
      warn "System service failed to start (check: sudo systemctl status $SERVICE_LABEL)"
      sudo systemctl --no-pager --full status "$SERVICE_LABEL" || true
    fi
  fi

  step "All set!"
  info "To test: $bin run"
  if [[ "$os" == "macos" ]]; then
    info "Logs:    tail -f ~/Library/Logs/chimera-mapper.log"
  else
    info "Logs:    sudo journalctl -u $SERVICE_LABEL -f"
  fi
}

main "$@"
