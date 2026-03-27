#!/bin/sh

set -eu

# Config
ENV_FILE="/usr/share/nginx/html/env-override.js"

# Validate required env vars
if [ -z "${VITE_METEROID_API_EXTERNAL_URL:-}" ]; then
    echo "Error: VITE_METEROID_API_EXTERNAL_URL is not set"
    exit 1
fi
if [ -z "${VITE_METEROID_REST_API_EXTERNAL_URL:-}" ]; then
    echo "Error: VITE_METEROID_REST_API_EXTERNAL_URL is not set"
    exit 1
fi

# Helper to add optional env var to config
add_optional() {
    var_name="$1"
    eval "var_value=\"\${${var_name}:-}\""
    if [ -n "$var_value" ]; then
        echo "    ${var_name}: \"${var_value}\","
    fi
}

# Generate config
{
    echo "window._env = {"
    echo "    IS_DOCKER: true,"
    echo "    VITE_METEROID_API_EXTERNAL_URL: \"${VITE_METEROID_API_EXTERNAL_URL}\","
    echo "    VITE_METEROID_REST_API_EXTERNAL_URL: \"${VITE_METEROID_REST_API_EXTERNAL_URL}\","
    add_optional VITE_PUBLIC_POSTHOG_KEY
    add_optional VITE_PUBLIC_POSTHOG_HOST
    add_optional VITE_PUBLIC_POSTHOG_HOST_FALLBACK
    echo "};"
} > "$ENV_FILE"
