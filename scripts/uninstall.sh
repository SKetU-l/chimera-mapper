#!/usr/bin/env bash
set -euo pipefail

BIN_NAME="chimera-mapper"
SERVICE_LABEL="com.sketu.chimera-mapper"
LINUX_MODULES_LOAD="/etc/modules-load.d/${BIN_NAME}.conf"
LINUX_UDEV_RULES="/etc/udev/rules.d/99-${BIN_NAME}.rules"
USER_BIN="${HOME}/.local/bin/${BIN_NAME}"
SYSTEM_BIN="/usr/local/bin/${BIN_NAME}"
SYSTEM_SERVICE="/etc/systemd/system/${SERVICE_LABEL}.service"
USER_SERVICE="${HOME}/.config/systemd/user/${SERVICE_LABEL}.service"

G='\033[0;32m' Y='\033[0;33m' R='\033[0;31m' B='\033[1m' D='\033[90m' N='\033[0m'
status() { echo -e "${G}✓${N} $1"; }
step()   { echo -e "\n${B}$1${N}"; }
info()   { echo -e "${D}  $1${N}"; }
warn()   { echo -e "${Y}!${N} $1"; }
error()  { echo -e "${R}✗${N} $1" >&2; }

detect_os() { case "$(uname -s)" in Darwin) echo macos;; Linux) echo linux;; *) error "Unsupported OS"; exit 1;; esac; }

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
    rm -f "${HOME}/Library/Logs/${BIN_NAME}.log" "${HOME}/Library/Logs/${BIN_NAME}.err.log"
  else
    sudo systemctl stop         "$SERVICE_LABEL" 2>/dev/null || true
    sudo systemctl disable      "$SERVICE_LABEL" 2>/dev/null || true
    sudo systemctl reset-failed "$SERVICE_LABEL" 2>/dev/null || true
    systemctl --user stop         "$SERVICE_LABEL" 2>/dev/null || true
    systemctl --user disable      "$SERVICE_LABEL" 2>/dev/null || true
    systemctl --user reset-failed "$SERVICE_LABEL" 2>/dev/null || true

    [[ -f "$SYSTEM_SERVICE" ]] && sudo rm -f "$SYSTEM_SERVICE" && sudo systemctl daemon-reload
    [[ -f "$USER_SERVICE" ]] && rm -f "$USER_SERVICE" && systemctl --user daemon-reload
    [[ -f "$LINUX_MODULES_LOAD" ]] && { sudo rm -f "$LINUX_MODULES_LOAD" || true; }
    [[ -f "$LINUX_UDEV_RULES" ]] && sudo rm -f "$LINUX_UDEV_RULES" && sudo udevadm control --reload-rules 2>/dev/null || true
  fi
  status "Auto-start disabled"

  if [[ "$keep_binary" != "true" ]]; then
    step "Removing files"
    local removed=false
    if [[ -f "$SYSTEM_BIN" ]]; then
      sudo rm -f "$SYSTEM_BIN" && status "System binary removed"
      removed=true
    fi
    if [[ -f "$USER_BIN" ]]; then
      rm -f "$USER_BIN" && status "User binary removed"
      removed=true
    fi
    [[ "$removed" == "false" ]] && warn "Binary not found (already removed?)"
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
  echo ""
}

main "$@"
