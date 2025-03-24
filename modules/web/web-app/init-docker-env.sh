#!/bin/sh

set -eu

# Config
ENV_FILE="/usr/share/nginx/html/env-override.js"

# Validate required env vars
if [ -z "${VITE_METEROID_API_EXTERNAL_URL:-}" ]; then
    echo "Error: VITE_METEROID_API_EXTERNAL_URL is not set"
    exit 1
fi
# Validate required env vars
if [ -z "${VITE_METEROID_REST_API_EXTERNAL_URL:-}" ]; then
    echo "Error: VITE_METEROID_API_EXTERNAL_URL is not set"
    exit 1
fi
# Generate config
cat > "$ENV_FILE" << EOF
window._env = {
    IS_DOCKER: true,
    VITE_METEROID_API_EXTERNAL_URL: "${VITE_METEROID_API_EXTERNAL_URL}",
    VITE_METEROID_REST_API_EXTERNAL_URL: "${VITE_METEROID_REST_API_EXTERNAL_URL}"
};
EOF
