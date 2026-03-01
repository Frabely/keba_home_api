#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
CREATE_KEBA_USER=1

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing required command: $1" >&2
    exit 1
  fi
}

is_dir_writable() {
  local dir="$1"
  mkdir -p "${dir}" >/dev/null 2>&1 || return 1
  local probe_file
  probe_file="$(mktemp "${dir}/.keba-write-test.XXXXXX" 2>/dev/null)" || return 1
  rm -f "${probe_file}" >/dev/null 2>&1 || return 1
  return 0
}

print_usage() {
  cat <<'EOF'
Usage: bash ./scripts/setup_all.sh [--no-create-user]

Options:
  --no-create-user  Do not create the 'keba' system user automatically.
EOF
}

for arg in "$@"; do
  case "$arg" in
    --no-create-user)
      CREATE_KEBA_USER=0
      ;;
    -h|--help)
      print_usage
      exit 0
      ;;
    *)
      echo "unknown argument: $arg" >&2
      print_usage
      exit 1
      ;;
  esac
done

ensure_keba_user() {
  if id -u keba >/dev/null 2>&1; then
    return
  fi

  if [[ "${CREATE_KEBA_USER}" -eq 1 ]]; then
    require_cmd useradd
    echo "creating missing system user 'keba'..."
    sudo useradd --system --home /var/lib/keba --create-home --shell /usr/sbin/nologin keba
    return
  fi

  echo "warning: user 'keba' does not exist; continuing without ownership changes (--no-create-user)" >&2
}

ensure_cargo() {
  if command -v cargo >/dev/null 2>&1; then
    return
  fi

  if [[ -x "${HOME}/.cargo/bin/cargo" ]]; then
    export PATH="${HOME}/.cargo/bin:${PATH}"
    return
  fi

  echo "cargo not found, installing Rust toolchain via rustup..."
  require_cmd curl

  local rustup_home="${RUSTUP_HOME:-${HOME}/.rustup}"
  local cargo_home="${CARGO_HOME:-${HOME}/.cargo}"

  if ! is_dir_writable "${rustup_home}" || ! is_dir_writable "${cargo_home}"; then
    echo "warning: ${rustup_home} or ${cargo_home} is not writable; falling back to /tmp" >&2
    rustup_home="/tmp/keba-rustup-${USER:-user}"
    cargo_home="/tmp/keba-cargo-${USER:-user}"
    if ! is_dir_writable "${rustup_home}" || ! is_dir_writable "${cargo_home}"; then
      echo "cargo installation failed: neither HOME nor /tmp is writable." >&2
      echo "Check disk/filesystem health (df -h, dmesg, fsck)." >&2
      exit 1
    fi
  fi

  RUSTUP_HOME="${rustup_home}" CARGO_HOME="${cargo_home}" \
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
    RUSTUP_HOME="${rustup_home}" CARGO_HOME="${cargo_home}" \
    sh -s -- -y --profile minimal

  export RUSTUP_HOME="${rustup_home}"
  export CARGO_HOME="${cargo_home}"
  export PATH="${CARGO_HOME}/bin:${PATH}"

  if [[ -f "${CARGO_HOME}/env" ]]; then
    # shellcheck disable=SC1090
    source "${CARGO_HOME}/env"
  fi

  if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo installation failed (cargo still not found in PATH)" >&2
    echo "If you still see I/O errors, verify storage with: df -h, dmesg | tail -n 50, sudo fsck." >&2
    exit 1
  fi

  # cargo may exist as rustup shim without a configured default toolchain.
  if ! cargo -V >/dev/null 2>&1; then
    require_cmd rustup
    echo "cargo is present but no default Rust toolchain is configured; setting stable..."
    rustup default stable
    if ! cargo -V >/dev/null 2>&1; then
      echo "cargo is still not usable after 'rustup default stable'." >&2
      exit 1
    fi
  fi
}

ensure_cargo
require_cmd sudo
require_cmd systemctl
require_cmd install

echo "[1/6] building release binaries..."
cd "${REPO_ROOT}"
cargo build --release -p keba-service -p keba-api

echo "[2/6] creating runtime directories..."
sudo mkdir -p /opt/keba_home_api /opt/keba_home_api/scripts /etc/keba /var/lib/keba /var/backups/keba

ensure_keba_user
if id -u keba >/dev/null 2>&1; then
  sudo chown -R keba:keba /opt/keba_home_api /var/lib/keba /var/backups/keba
else
  echo "warning: ownership changes skipped because user 'keba' does not exist" >&2
fi

echo "[3/6] installing binaries and scripts..."
sudo install -m 0755 "${REPO_ROOT}/target/release/keba_service" /opt/keba_home_api/keba_service
sudo install -m 0755 "${REPO_ROOT}/target/release/keba_api" /opt/keba_home_api/keba_api
sudo install -m 0755 "${REPO_ROOT}/scripts/backup_keba_db.sh" /opt/keba_home_api/scripts/backup_keba_db.sh

copy_if_missing() {
  local src="$1"
  local dst="$2"
  if [[ -f "${dst}" ]]; then
    echo "keeping existing ${dst}"
  else
    sudo cp "${src}" "${dst}"
  fi
  sudo chown root:root "${dst}"
  sudo chmod 0640 "${dst}"
}

echo "[4/6] installing env files (keeps existing)..."
copy_if_missing "${REPO_ROOT}/deploy/systemd/keba-home-service-carport.env.example" /etc/keba/keba-home-service-carport.env
copy_if_missing "${REPO_ROOT}/deploy/systemd/keba-home-service-eingang.env.example" /etc/keba/keba-home-service-eingang.env
copy_if_missing "${REPO_ROOT}/deploy/systemd/keba-home-api-reader.env.example" /etc/keba/keba-home-api-reader.env
copy_if_missing "${REPO_ROOT}/deploy/systemd/keba-db-backup.env.example" /etc/keba/keba-db-backup.env

echo "[5/6] installing systemd units..."
sudo cp "${REPO_ROOT}/deploy/systemd/keba-home-service@.service" /etc/systemd/system/keba-home-service@.service
sudo cp "${REPO_ROOT}/deploy/systemd/keba-home-api-reader.service" /etc/systemd/system/keba-home-api-reader.service
sudo cp "${REPO_ROOT}/deploy/systemd/keba-db-backup.service" /etc/systemd/system/keba-db-backup.service
sudo cp "${REPO_ROOT}/deploy/systemd/keba-db-backup.timer" /etc/systemd/system/keba-db-backup.timer
sudo systemctl daemon-reload


echo "[6/6] enabling services and timer..."
sudo systemctl enable keba-home-service@carport keba-home-service@eingang keba-home-api-reader keba-db-backup.timer

echo
echo "Setup complete."
echo "Next steps:"
echo "1) Edit KEBA_IP in:"
echo "   /etc/keba/keba-home-service-carport.env"
echo "   /etc/keba/keba-home-service-eingang.env"
echo "2) Start all processes with: bash ${REPO_ROOT}/scripts/start_all_services.sh"
