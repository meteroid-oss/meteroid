#!/usr/bin/env bash
# preview-stop.sh — Tears down the preview environment for a given PR.
#
# Required env:
#   PR_NUMBER  — PR number
#
# Optional flags:
#   --quiet    — Suppress informational output (used when called from setup.sh)
#   --clean    — Also delete the workspace directory and postgres volume

set -uo pipefail

QUIET=false
CLEAN=false
for arg in "${@:-}"; do
  case "$arg" in
    --quiet) QUIET=true ;;
    --clean) CLEAN=true ;;
  esac
done

log() { $QUIET || echo "[preview-stop] PR #${PR_NUMBER}: $*"; }

WORKSPACE="/opt/meteroid-previews/pr-${PR_NUMBER}/repo"
PREVIEW_DIR="/opt/meteroid-previews/pr-${PR_NUMBER}"
TMUX_SESSION="meteroid-pr-${PR_NUMBER}"
COMPOSE_PROJECT="meteroid-pr-${PR_NUMBER}"
COMPOSE_OVERRIDE="${PREVIEW_DIR}/compose-override.yml"
FRPC_CONFIG="${PREVIEW_DIR}/frpc.toml"

log "Stopping preview environment…"

# ── Kill tmux session (stops all processes: frpc, backend, frontend, watcher) ─
if tmux has-session -t "$TMUX_SESSION" 2>/dev/null; then
  log "Killing tmux session ${TMUX_SESSION}…"
  tmux kill-session -t "$TMUX_SESSION"
else
  log "No tmux session found (already stopped?)"
fi

# ── Stop docker compose (postgres) ────────────────────────────────────────────
if [[ -f "${COMPOSE_OVERRIDE}" && -d "${WORKSPACE}" ]]; then
  log "Stopping docker compose project ${COMPOSE_PROJECT}…"
  docker compose \
    -f "${WORKSPACE}/docker/develop/docker-compose-lite.yml" \
    -f "${COMPOSE_OVERRIDE}" \
    -p "$COMPOSE_PROJECT" \
    down 2>/dev/null || true
elif docker ps --format '{{.Names}}' 2>/dev/null | grep -q "meteroid-db-pr-${PR_NUMBER}"; then
  log "Forcing container stop (meteroid-db-pr-${PR_NUMBER})…"
  docker stop "meteroid-db-pr-${PR_NUMBER}" 2>/dev/null || true
  docker rm   "meteroid-db-pr-${PR_NUMBER}" 2>/dev/null || true
fi

# ── Clean up temp/log files ───────────────────────────────────────────────────
log "Removing temp files…"
rm -f "$FRPC_CONFIG"
rm -f "${PREVIEW_DIR}/compose-override.yml"
rm -f "/tmp/preview-scripts/.env-pr-${PR_NUMBER}"

if $CLEAN; then
  log "--clean: removing workspace and postgres volume…"
  docker volume rm "${COMPOSE_PROJECT}_pg_data" 2>/dev/null || true
  rm -rf "$PREVIEW_DIR"
fi

log "Preview environment stopped."
