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
