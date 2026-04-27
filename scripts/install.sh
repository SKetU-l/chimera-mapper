#!/usr/bin/env bash
set -euo pipefail

REPO_OWNER="SKetU-l"
REPO_NAME="chimera-mapper"
BIN_NAME="chimera-mapper"
SERVICE_LABEL="com.sketu.chimera-mapper"
REPO_URL="https://github.com/${REPO_OWNER}/${REPO_NAME}.git"

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
  cargo build --release --manifest-path "$tmp/Cargo.toml" >&2

  local src="$tmp/target/release/${BIN_NAME}"
  [[ -f "$src" ]] || { error "Build failed: binary not found" >&2; return 1; }
  mkdir -p "$install_dir"
  cp "$src" "${install_dir}/${BIN_NAME}" && chmod +x "${install_dir}/${BIN_NAME}"
  echo "${install_dir}/${BIN_NAME}"  # only this goes to stdout → captured into $bin
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
  local bin="$1" service="${HOME}/.config/systemd/user/${SERVICE_LABEL}.service"
  mkdir -p "$(dirname "$service")"
  cat > "$service" <<-SERVICE
	[Unit]
	Description=Chimera Mapper
	After=default.target
	[Service]
	Type=simple
	ExecStart=$bin run
	Restart=always
	RestartSec=2
	[Install]
	WantedBy=default.target
	SERVICE
  systemctl --user daemon-reload 2>/dev/null || true
  systemctl --user enable --now "$SERVICE_LABEL" 2>/dev/null || true
  status "Auto-start enabled"
}

main() {
  local install_dir="${HOME}/.local/bin" skip_service=false os arch bin
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --install-dir) install_dir="$2"; shift ;;
      --skip-service) skip_service=true ;;
      -h|--help) echo "Usage: $0 [--install-dir PATH] [--skip-service]"; exit 0 ;;
      *) error "Unknown option: $1"; exit 1 ;;
    esac
    shift
  done

  os=$(detect_os); arch=$(detect_arch)

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
  if   { [[ "$os" == "macos" ]] && launchctl list | grep -q "$SERVICE_LABEL"; } || \
       { [[ "$os" == "linux" ]] && systemctl --user is-active "$SERVICE_LABEL" &>/dev/null; }; then
    status "Service is running"
  else
    warn "Service may need a moment to start"
  fi

  step "All set!"
  info "To test: $bin run"
  info "Logs:    tail -f ~/Library/Logs/chimera-mapper.log"
}

main "$@"
