#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(git -C "${SCRIPT_DIR}" rev-parse --show-toplevel)"

cd "${REPO_DIR}"

echo "[1/6] Update repository"
git pull --ff-only origin master

echo "[2/6] Build release binaries"
cargo build --release -p keba-service -p keba-api

echo "[3/6] Install binaries"
sudo install -m 0755 ./target/release/keba_service "${REPO_DIR}/keba_service"
sudo install -m 0755 ./target/release/keba_api "${REPO_DIR}/keba_api"

echo "[4/6] Restart services"
bash scripts/restart_services.sh

echo "[5/6] Check service status"
sudo systemctl status keba-home-service@carport keba-home-service@eingang keba-home-api-reader --no-pager

echo "[6/6] API checks"
curl -s http://127.0.0.1:8080/health
curl -s http://127.0.0.1:8080/sessions/carport/latest

echo "post deploy check finished"
