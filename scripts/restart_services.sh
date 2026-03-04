#!/usr/bin/env bash
set -euo pipefail

SERVICES=(
  "keba-home-service@carport"
  "keba-home-service@eingang"
  "keba-home-api-reader"
)

assert_service_active() {
  local unit="$1"
  local retries=10
  local delay_s=1
  local i

  for ((i = 1; i <= retries; i++)); do
    if sudo systemctl is-active --quiet "${unit}"; then
      return 0
    fi
    sleep "${delay_s}"
  done

  echo "service did not become active: ${unit}" >&2
  sudo systemctl --no-pager --full status "${unit}" || true
  return 1
}

echo "[1/3] Restart services"
for unit in "${SERVICES[@]}"; do
  sudo systemctl restart "${unit}"
done

echo "[2/3] Verify active status"
for unit in "${SERVICES[@]}"; do
  assert_service_active "${unit}"
done
sudo systemctl --no-pager --full status "${SERVICES[@]}"

echo "[3/3] Show recent logs"
for unit in "${SERVICES[@]}"; do
  echo "--- ${unit} (last 20 lines) ---"
  sudo journalctl -u "${unit}" -n 20 --no-pager
done
