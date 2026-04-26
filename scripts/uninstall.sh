#!/usr/bin/env bash
set -euo pipefail

GREEN='\033[0;32m'; YELLOW='\033[0;33m'; RED='\033[0;31m'; BOLD='\033[1m'; DIM='\033[90m'; RESET='\033[0m'

BIN_NAME="chimera-mapper"
SERVICE_LABEL="xyz.sketu.chimera-mapper"
USER_BIN="${HOME}/.local/bin/${BIN_NAME}"
SYSTEM_BIN="/usr/local/bin/${BIN_NAME}"
SYSTEMD_SERVICE="${HOME}/.config/systemd/user/chimera-mapper.service"

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

main() {
  local purge=false keep_binary=false
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --purge) purge=true ;;
      --keep-binary) keep_binary=true ;;
      -h|--help) echo "Usage: $0 [--purge] [--keep-binary]"; exit 0 ;;
      *) error "Unknown option: $1"; exit 1 ;;
    esac
    shift
  done

  local os=$(detect_os)

  step "Removing Chimera Mapper"
  info "System: $(uname -s)"

  step "Stopping the app"
  if [[ "$os" == "macos" ]]; then
    local plist="${HOME}/Library/LaunchAgents/${SERVICE_LABEL}.plist"
    launchctl bootout "gui/$(id -u)/${SERVICE_LABEL}" 2>/dev/null || true
    launchctl unload "$plist" 2>/dev/null || true
    rm -f "$plist"
    rm -f "${HOME}/Library/Logs/chimera-mapper.log" "${HOME}/Library/Logs/chimera-mapper.err.log"
  else
    systemctl --user stop chimera-mapper 2>/dev/null || true
    systemctl --user disable chimera-mapper 2>/dev/null || true
    systemctl --user daemon-reload 2>/dev/null || true
    rm -f "$SYSTEMD_SERVICE"
  fi
  status "Auto-start disabled"

  if [[ "$keep_binary" != "true" ]]; then
    step "Removing files"
    rm -f "$USER_BIN" "$SYSTEM_BIN" && status "Application removed" || warn "Some files not found (already removed?)"
  else
    info "Keeping application file"
  fi

  if [[ "$purge" == "true" ]] && [[ -d "${HOME}/.config/chimera-mapper" ]]; then
    rm -rf "${HOME}/.config/chimera-mapper"
    status "Settings removed"
  fi

  echo ""
  step "Done"
  info "Chimera Mapper has been uninstalled"
  info "To reinstall: curl -fsSL https://raw.githubusercontent.com/SKetU-l/chimera-mapper/main/scripts/install.sh | bash"
  echo ""
}

main "$@"
