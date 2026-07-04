#!/usr/bin/env bash
set -euo pipefail

# When run via `curl | bash`, BASH_SOURCE[0] is empty/unset; guard with `:-` so `set -u`
# doesn't kill the script before the curl fallback can run.
if [[ -n "${BASH_SOURCE[0]:-}" ]]; then
  SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
else
  SCRIPT_DIR=""
fi
if [[ -n "$SCRIPT_DIR" && -f "$SCRIPT_DIR/lib.sh" ]]; then
  source "$SCRIPT_DIR/lib.sh"
else
  source <(curl -fsSL https://raw.githubusercontent.com/SKetU-l/chimera-mapper/main/scripts/lib.sh)
fi

USER_BIN="${HOME}/.local/bin/${BIN_NAME}"
SYSTEM_BIN="/usr/local/bin/${BIN_NAME}"
SYSTEM_SERVICE="/etc/systemd/system/${SERVICE_LABEL}.service"
USER_SERVICE="${HOME}/.config/systemd/user/${SERVICE_LABEL}.service"

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
