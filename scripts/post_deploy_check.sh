#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(git -C "${SCRIPT_DIR}" rev-parse --show-toplevel)"
SERVICES=(
  "keba-home-service@carport"
  "keba-home-service@eingang"
  "keba-home-api-reader"
)

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing required command: $1" >&2
    exit 1
  fi
}

resolve_exec_path() {
  local unit="$1"
  local fallback="$2"
  local execstart
  local path

  execstart="$(sudo systemctl cat "${unit}" 2>/dev/null | awk -F= '/^ExecStart=/{print $2; exit}')"
  if [[ -z "${execstart}" ]]; then
    echo "${fallback}"
    return
  fi

  path="$(awk '{print $1}' <<<"${execstart}")"
  path="${path#-}"
  path="${path#\"}"
  path="${path%\"}"

  if [[ -n "${path}" ]]; then
    echo "${path}"
  else
    echo "${fallback}"
  fi
}

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

parse_json_field() {
  local json="$1"
  local field="$2"
  python3 - "$json" "$field" <<'PY'
import json
import sys

raw = sys.argv[1]
field = sys.argv[2]
obj = json.loads(raw)
if field not in obj:
    raise SystemExit(1)
value = obj[field]
if isinstance(value, str):
    print(value)
elif value is None:
    print("null")
else:
    print(str(value))
PY
}

cd "${REPO_DIR}"

require_cmd git
require_cmd cargo
require_cmd curl
require_cmd python3
require_cmd sudo
require_cmd systemctl

SERVICE_BIN_TARGET="$(resolve_exec_path "keba-home-service@carport" "/opt/keba_home_api/keba_service")"
API_BIN_TARGET="$(resolve_exec_path "keba-home-api-reader" "/opt/keba_home_api/keba_api")"
SERVICE_TARGET_DIR="$(dirname "${SERVICE_BIN_TARGET}")"
API_TARGET_DIR="$(dirname "${API_BIN_TARGET}")"
DEPLOY_SCRIPTS_DIR="${API_TARGET_DIR}/scripts"

echo "[1/8] Update repository"
git pull --ff-only origin master

echo "[2/8] Build release binaries"
cargo build --release -p keba-service -p keba-api

echo "[3/8] Ensure deploy directories"
sudo mkdir -p "${SERVICE_TARGET_DIR}" "${API_TARGET_DIR}" "${DEPLOY_SCRIPTS_DIR}"

echo "[4/8] Install binaries"
sudo install -m 0755 ./target/release/keba_service "${SERVICE_BIN_TARGET}"
sudo install -m 0755 ./target/release/keba_api "${API_BIN_TARGET}"

echo "[5/8] Verify installed binaries"
cmp -s ./target/release/keba_service "${SERVICE_BIN_TARGET}" || {
  echo "installed service binary differs from built artifact" >&2
  exit 1
}
cmp -s ./target/release/keba_api "${API_BIN_TARGET}" || {
  echo "installed api binary differs from built artifact" >&2
  exit 1
}

echo "[6/8] Install helper scripts"
sudo install -m 0755 ./scripts/post_deploy_check.sh "${DEPLOY_SCRIPTS_DIR}/post_deploy_check.sh"
sudo install -m 0755 ./scripts/restart_services.sh "${DEPLOY_SCRIPTS_DIR}/restart_services.sh"
sudo install -m 0755 ./scripts/start_all_services.sh "${DEPLOY_SCRIPTS_DIR}/start_all_services.sh"

echo "[7/8] Restart services"
bash scripts/restart_services.sh

for unit in "${SERVICES[@]}"; do
  assert_service_active "${unit}"
done

echo "[8/8] API checks"
health_json="$(curl -fsS http://127.0.0.1:8080/api/v1/health)"
health_status="$(parse_json_field "${health_json}" "status" || true)"
if [[ "${health_status}" != "ok" ]]; then
  echo "health check failed: ${health_json}" >&2
  exit 1
fi
echo "${health_json}"

carport_json="$(curl -fsS http://127.0.0.1:8080/api/v1/sessions/carport/latest)"
if parse_json_field "${carport_json}" "error" >/dev/null 2>&1; then
  echo "carport latest returned error payload: ${carport_json}"
else
  parse_json_field "${carport_json}" "kWh" >/dev/null
  parse_json_field "${carport_json}" "started" >/dev/null
  parse_json_field "${carport_json}" "ended" >/dev/null
  echo "${carport_json}"
fi

echo "post deploy check finished (repo=${REPO_DIR}, service_bin=${SERVICE_BIN_TARGET}, api_bin=${API_BIN_TARGET})"
