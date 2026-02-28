#!/usr/bin/env bash
set -euo pipefail

DB_PATH="${DB_PATH:-/var/lib/keba/keba.db}"
BACKUP_DIR="${BACKUP_DIR:-/var/backups/keba}"
KEEP_DAYS="${KEEP_DAYS:-7}"

if ! command -v sqlite3 >/dev/null 2>&1; then
  echo "sqlite3 is required for consistent online backups" >&2
  exit 1
fi

mkdir -p "${BACKUP_DIR}"

timestamp="$(date +%Y%m%d-%H%M%S)"
backup_file="${BACKUP_DIR}/keba-${timestamp}.db"

sqlite3 "${DB_PATH}" ".timeout 5000" ".backup '${backup_file}'"

# keep only recent backups
find "${BACKUP_DIR}" -type f -name 'keba-*.db' -mtime "+${KEEP_DAYS}" -delete

echo "backup created: ${backup_file}"
