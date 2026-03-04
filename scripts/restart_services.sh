#!/usr/bin/env bash
set -euo pipefail

sudo systemctl restart keba-home-service@carport
sudo systemctl restart keba-home-service@eingang
sudo systemctl restart keba-home-api-reader

sudo systemctl --no-pager --full status \
  keba-home-service@carport \
  keba-home-service@eingang \
  keba-home-api-reader
