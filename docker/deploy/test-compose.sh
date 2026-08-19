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

custom_config="$(
  METEROID_IMAGE_REGISTRY=ghcr.io/tritondatacenter \
    METEROID_IMAGE_TAG=sha-test \
    docker compose \
      --file "${compose_file}" \
      --env-file "${env_file}" \
      config \
      --format json
)"

actual_application_images="$(
  jq --raw-output \
    '[
      .services["meteroid-api"].image,
      .services["meteroid-scheduler"].image,
      .services["metering-api"].image,
      .services["meteroid-web"].image
    ] | sort | .[]' \
    <<<"${custom_config}"
)"

expected_application_images="$(printf '%s\n' \
  ghcr.io/tritondatacenter/metering-api:sha-test \
  ghcr.io/tritondatacenter/meteroid-api:sha-test \
  ghcr.io/tritondatacenter/meteroid-scheduler:sha-test \
  ghcr.io/tritondatacenter/meteroid-web:sha-test)"

if [[ "${actual_application_images}" != "${expected_application_images}" ]]; then
  echo "application images did not use the requested registry and tag" >&2
  diff \
    <(printf '%s\n' "${expected_application_images}") \
    <(printf '%s\n' "${actual_application_images}") >&2 || true
  exit 1
fi

echo "deployment Compose configuration is valid"
