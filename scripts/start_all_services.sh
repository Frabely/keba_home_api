#!/usr/bin/env bash
set -euo pipefail

required_envs=(
  "/etc/keba/keba-home-service-carport.env"
  "/etc/keba/keba-home-service-eingang.env"
)

for env_file in "${required_envs[@]}"; do
  if [[ ! -f "${env_file}" ]]; then
    echo "missing required env file: ${env_file}" >&2
    exit 1
  fi
  if grep -Eq '^KEBA_IP=REPLACE_WITH_' "${env_file}"; then
    echo "please set KEBA_IP in ${env_file} before start" >&2
    exit 1
  fi
done

sudo systemctl start keba-home-service@carport
sudo systemctl start keba-home-service@eingang
sudo systemctl start keba-home-api-reader

sudo systemctl --no-pager --full status keba-home-service@carport keba-home-service@eingang keba-home-api-reader
