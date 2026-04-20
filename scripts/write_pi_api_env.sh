#!/usr/bin/env bash
set -euo pipefail

API_ENV_FILE="${API_ENV_FILE:-/etc/keba/keba-home-api-reader.env}"
DB_PATH="${DB_PATH:-/var/lib/keba/keba.db}"
HTTP_BIND="${HTTP_BIND:-0.0.0.0:65109}"
STATUS_STATIONS="${STATUS_STATIONS:-Carport@192.168.233.99:7090;Eingang@192.168.233.98:7090}"
CORS_ALLOWED_ORIGINS="${CORS_ALLOWED_ORIGINS:-http://localhost:3000,https://invessiv.de}"
DACHS_BASE_URL="${DACHS_BASE_URL:-http://192.168.233.91:8080}"
DACHS_USERNAME="${DACHS_USERNAME:-}"
DACHS_PASSWORD="${DACHS_PASSWORD:-}"
RUST_LOG="${RUST_LOG:-info}"
LOG_FORMAT="${LOG_FORMAT:-compact}"

sudo install -d -m 0755 "$(dirname "${API_ENV_FILE}")"

tmp_file="$(mktemp)"
trap 'rm -f "${tmp_file}"' EXIT

cat >"${tmp_file}" <<EOF
DB_PATH=${DB_PATH}
HTTP_BIND=${HTTP_BIND}
STATUS_STATIONS=${STATUS_STATIONS}
CORS_ALLOWED_ORIGINS=${CORS_ALLOWED_ORIGINS}
DACHS_BASE_URL=${DACHS_BASE_URL}
DACHS_USERNAME=${DACHS_USERNAME}
DACHS_PASSWORD=${DACHS_PASSWORD}
RUST_LOG=${RUST_LOG}
LOG_FORMAT=${LOG_FORMAT}
EOF

sudo install -m 0644 "${tmp_file}" "${API_ENV_FILE}"
echo "wrote ${API_ENV_FILE}"
