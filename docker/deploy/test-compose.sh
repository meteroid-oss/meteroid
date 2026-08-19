#!/usr/bin/env bash

set -euo pipefail

deploy_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
compose_file="${deploy_dir}/docker-compose.yml"
env_file="${deploy_dir}/.env.example"

rendered_config="$(
  docker compose \
    --file "${compose_file}" \
    --env-file "${env_file}" \
    config \
    --format json
)"

clickhouse_dependency="$(
  jq --raw-output \
    '.services.clickhouse.depends_on["clickhouse-volume-init"].condition // empty' \
    <<<"${rendered_config}"
)"

if [[ "${clickhouse_dependency}" != "service_completed_successfully" ]]; then
  echo "expected ClickHouse to wait for clickhouse-volume-init" >&2
  exit 1
fi

echo "deployment Compose configuration is valid"
