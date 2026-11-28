#!/usr/bin/env bash
# Lints every recipe with Redpanda Connect, no licence: an Enterprise component would fail here.
set -euo pipefail
cd "$(dirname "$0")/.."
docker run --rm -v "$PWD:/recipes:ro" -w /recipes \
  --env-file test/lint.env \
  "docker.redpanda.com/redpandadata/connect:${CONNECT_VERSION:-4.68.0}" \
  lint meteroid.yaml dead-letter-file.yaml dead-letter-kafka.yaml sources/*.yaml
