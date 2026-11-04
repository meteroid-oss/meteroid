#!/usr/bin/env bash
# Pulls the images the integration tests start, then touches "$1". Run in the background while the
# tests build: the first test needing an image otherwise extracts it mid-run, and that disk load
# stalls the Postgres of every test running alongside it for up to a minute.
# Keep in sync with the image constants of meteroid-testkit and the analytics tests.
set -u
images=(
  clickhouse/clickhouse-server:25.6.2-alpine
  redpandadata/redpanda:v23.3.2
  zookeeper:3.9
  redis:8-alpine
)
for image in "${images[@]}"; do
  docker pull -q "$image" || echo "could not pull $image; the test that needs it will"
done
touch "$1"
