#!/usr/bin/env bash
set -euo pipefail

GREEN='\033[0;32m'; YELLOW='\033[0;33m'; RED='\033[0;31m'; BOLD='\033[1m'; DIM='\033[90m'; RESET='\033[0m'

REPO_OWNER="SKetU-l"
REPO_NAME="chimera-mapper"
BIN_NAME="chimera-mapper"
SERVICE_LABEL="xyz.sketu.chimera-mapper"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

status() { echo -e "${GREEN}✓${RESET} $1"; }
step()  { echo -e "\n${BOLD}$1${RESET}"; }
info()  { echo -e "${DIM}  $1${RESET}"; }
warn()  { echo -e "${YELLOW}!${RESET} $1"; }
error() { echo -e "${RED}✗${RESET} $1" >&2; }

detect_os() {
  case "$(uname -s)" in
    Darwin) echo "macos" ;;
    Linux)  echo "linux" ;;
    *)      error "Unsupported OS"; exit 1 ;;
  esac
}

detect_arch() {
  case "$(uname -m)" in
    x86_64|amd64)  echo "x86_64" ;;
    aarch64|arm64) echo "aarch64" ;;
    *)             error "Unsupported arch"; exit 1 ;;
  esac
}

download_binary() {
  local os="$1" arch="$2" install_dir="$3" version="$4"
  local url="https://github.com/${REPO_OWNER}/${REPO_NAME}/releases/download/v${version}/${BIN_NAME}-${os}-${arch}"
  local dest="${install_dir}/${BIN_NAME}"
  mkdir -p "$install_dir"
  curl -fsSL -o "$dest" "$url" 2>/dev/null && chmod +x "$dest" && echo "$dest" && return 0
  return 1
}

build_from_source() {
  local install_dir="$1"
  command -v cargo &>/dev/null || { error "Cargo not found"; return 1; }
  cd "$REPO_ROOT" && cargo build --release 2>&1 | grep "Finished" >&2 || true
  local src="$REPO_ROOT/target/release/${BIN_NAME}"
  [[ ! -f "$src" ]] && { error "Build failed"; return 1; }
  mkdir -p "$install_dir" && cp "$src" "${install_dir}/${BIN_NAME}" && chmod +x "${install_dir}/${BIN_NAME}"
  echo "${install_dir}/${BIN_NAME}"
}

install_macos_service() {
  local bin="$1"
  local plist="${HOME}/Library/LaunchAgents/${SERVICE_LABEL}.plist"
  mkdir -p "$(dirname "$plist")"
  cat > "$plist" << PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>$SERVICE_LABEL</string>
  <key>ProgramArguments</key>
  <array><string>$bin</string><string>run</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>$HOME/Library/Logs/chimera-mapper.log</string>
  <key>StandardErrorPath</key><string>$HOME/Library/Logs/chimera-mapper.err.log</string>
</dict>
</plist>
PLIST
  launchctl bootstrap gui/$(id -u) "$plist" 2>/dev/null || launchctl load "$plist" 2>/dev/null || true
  status "Auto-start enabled (runs on startup)"
}

install_linux_service() {
  local bin="$1"
  local service="${HOME}/.config/systemd/user/chimera-mapper.service"
  mkdir -p "$(dirname "$service")"
  cat > "$service" << SERVICE
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
  systemctl --user enable --now chimera-mapper 2>/dev/null || true
  status "Auto-start enabled (runs on startup)"
}

main() {
  local install_dir="${HOME}/.local/bin" version="" skip_service=false
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --install-dir) install_dir="$2"; shift ;;
      --skip-service) skip_service=true ;;
      --version) version="$2"; shift ;;
      -h|--help) echo "Usage: $0 [--install-dir PATH] [--skip-service] [--version VERSION]"; exit 0 ;;
      *) error "Unknown option: $1"; exit 1 ;;
    esac
    shift
  done

  local os arch binary_path
  os=$(detect_os)
  arch=$(detect_arch)

  step "Setting up Chimera Mapper"
  info "System: $(uname -s) ($arch)"
  info "Install location: $install_dir"

  [[ -z "$version" ]] && version=$(curl -fsSL "https://api.github.com/repos/${REPO_OWNER}/${REPO_NAME}/releases/latest" 2>/dev/null | grep -o '"tag_name":"v[^"]*' | cut -d'"' -f4 | sed 's/^v//' || echo "")

  step "Getting the application"
  if [[ -n "$version" ]]; then
    info "Version: $version"
    binary_path=$(download_binary "$os" "$arch" "$install_dir" "$version" 2>/dev/null) || binary_path=""
    [[ -n "$binary_path" ]] && status "Downloaded pre-built version"
  fi

  if [[ -z "$binary_path" ]]; then
    warn "Pre-built not available for this system"
    step "Building from source"
    binary_path=$(build_from_source "$install_dir") || { error "Installation failed"; exit 1; }
    status "Compilation complete"
  fi

  if [[ "$skip_service" != "true" ]]; then
    step "Configuring auto-start"
    case "$os" in
      macos) install_macos_service "$binary_path" ;;
      linux) install_linux_service "$binary_path" ;;
    esac
  fi

  sleep 2

  step "Verifying installation"
  if [[ "$os" == "macos" ]] && launchctl list | grep -q "$SERVICE_LABEL"; then
    status "Service is running"
  elif [[ "$os" == "linux" ]] && systemctl --user is-active chimera-mapper &>/dev/null; then
    status "Service is running"
  else
    warn "Service may need a moment to start"
  fi

  echo ""
  step "All set!"
  info "Your device buttons are ready to use"
  info "The app will start automatically on next restart"
  info ""
  info "To test now: $binary_path run"
  info "View logs: tail -f ~/Library/Logs/chimera-mapper.log"
  echo ""
}

main "$@"
