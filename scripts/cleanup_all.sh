#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

ASSUME_YES=0
KEEP_DATA=0

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing required command: $1" >&2
    exit 1
  fi
}

print_usage() {
  cat <<'EOF'
Usage: bash ./scripts/cleanup_all.sh [--yes] [--keep-data]

Options:
  --yes        Skip confirmation prompt.
  --keep-data  Keep runtime data in /var/lib/keba and backups in /var/backups/keba.
EOF
}

for arg in "$@"; do
  case "$arg" in
    --yes)
      ASSUME_YES=1
      ;;
    --keep-data)
      KEEP_DATA=1
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

require_cmd sudo
require_cmd systemctl
require_cmd rm

if [[ "$ASSUME_YES" -ne 1 ]]; then
  echo "This will remove generated artifacts and runtime installation:"
  echo "- ${REPO_ROOT}/target"
  echo "- /opt/keba_home_api"
  echo "- /etc/systemd/system/keba-home-service@.service"
  echo "- /etc/systemd/system/keba-home-api-reader.service"
  echo "- /etc/systemd/system/keba-db-backup.service"
  echo "- /etc/systemd/system/keba-db-backup.timer"
  echo "- /etc/keba/keba-home-service-carport.env"
  echo "- /etc/keba/keba-home-service-eingang.env"
  echo "- /etc/keba/keba-home-api-reader.env"
  echo "- /etc/keba/keba-db-backup.env"
  if [[ "$KEEP_DATA" -eq 0 ]]; then
    echo "- /var/lib/keba"
    echo "- /var/backups/keba"
  fi
  echo
  read -r -p "Type YES to continue: " confirmation
  if [[ "$confirmation" != "YES" ]]; then
    echo "aborted"
    exit 1
  fi
fi

echo "[1/5] stopping and disabling services..."
for unit in \
  keba-home-api-reader \
  keba-home-service@carport \
  keba-home-service@eingang \
  keba-db-backup.timer \
  keba-db-backup.service
  do
  sudo systemctl disable --now "$unit" >/dev/null 2>&1 || true
done

echo "[2/5] removing deployed files and systemd units..."
sudo rm -rf /opt/keba_home_api
sudo rm -f /etc/systemd/system/keba-home-service@.service
sudo rm -f /etc/systemd/system/keba-home-api-reader.service
sudo rm -f /etc/systemd/system/keba-db-backup.service
sudo rm -f /etc/systemd/system/keba-db-backup.timer
sudo systemctl daemon-reload

echo "[3/5] removing generated env files..."
sudo rm -f /etc/keba/keba-home-service-carport.env
sudo rm -f /etc/keba/keba-home-service-eingang.env
sudo rm -f /etc/keba/keba-home-api-reader.env
sudo rm -f /etc/keba/keba-db-backup.env
sudo rmdir /etc/keba >/dev/null 2>&1 || true

echo "[4/5] removing local cargo build artifacts..."
rm -rf "${REPO_ROOT}/target"

echo "[5/5] removing runtime data directories..."
if [[ "$KEEP_DATA" -eq 0 ]]; then
  sudo rm -rf /var/lib/keba /var/backups/keba
else
  echo "keeping /var/lib/keba and /var/backups/keba"
fi

echo

echo "Cleanup complete."
