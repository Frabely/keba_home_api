#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing required command: $1" >&2
    exit 1
  fi
}

require_cmd cargo
require_cmd sudo
require_cmd systemctl
require_cmd install

echo "[1/6] building release binaries..."
cd "${REPO_ROOT}"
cargo build --release -p keba-service -p keba-api

echo "[2/6] creating runtime directories..."
sudo mkdir -p /opt/keba_home_api /opt/keba_home_api/scripts /etc/keba /var/lib/keba /var/backups/keba

if id -u keba >/dev/null 2>&1; then
  sudo chown -R keba:keba /opt/keba_home_api /var/lib/keba /var/backups/keba
else
  echo "warning: user 'keba' does not exist yet; ownership changes skipped" >&2
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
