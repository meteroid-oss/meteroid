#!/usr/bin/env bash
# preview-watch.sh — Runs in a tmux window on the Hetzner box.
# Polls the PR branch for new commits and triggers a live update when found.
#
# Required env vars (set by preview-setup.sh):
#   PR_NUMBER       — PR number
#   PR_BRANCH       — branch name
#   GH_TOKEN        — GitHub PAT (for git fetch authentication)
#   GITHUB_REPO     — org/repo
#   WORKSPACE       — absolute path to the git checkout
#   TMUX_SESSION    — tmux session name (e.g. meteroid-pr-127)
#   FRONTEND_PORT   — vite dev server port
#   LOG_DIR         — directory for log files

set -uo pipefail

POLL_INTERVAL=60   # seconds between git fetch checks

log() { echo "[watcher $(date '+%H:%M:%S')] PR #${PR_NUMBER}: $*"; }

log "Starting. Branch=${PR_BRANCH}"
cd "$WORKSPACE"

# Authenticate git fetches with the PAT
git remote set-url origin "https://x-access-token:${GH_TOKEN}@github.com/${GITHUB_REPO}.git"

LAST_SHA=$(git rev-parse HEAD)
log "Current SHA: ${LAST_SHA:0:8}"

while true; do
  sleep "$POLL_INTERVAL"

  # Fetch remote branch quietly; skip on transient errors
  if ! git fetch origin "${PR_BRANCH}" --quiet 2>/dev/null; then
    log "git fetch failed — will retry in ${POLL_INTERVAL}s"
    continue
  fi

  NEW_SHA=$(git rev-parse "origin/${PR_BRANCH}" 2>/dev/null || echo "")
  [[ -z "$NEW_SHA" ]] && { log "Could not resolve remote SHA — retrying"; continue; }
  [[ "$NEW_SHA" == "$LAST_SHA" ]] && continue

  log "Update detected: ${LAST_SHA:0:8} → ${NEW_SHA:0:8}"

  # Pull new commits
  git reset --hard "origin/${PR_BRANCH}"
  log "Workspace updated to ${NEW_SHA:0:8}"

  # ── Reinstall pnpm deps if lockfile/package.json changed ─────────────────
  CHANGED=$(git diff --name-only "$LAST_SHA" "$NEW_SHA" 2>/dev/null || echo "")
  if echo "$CHANGED" | grep -qE 'pnpm-lock\.yaml|package\.json'; then
    log "Package manifest changed — reinstalling pnpm deps…"
    # Stop the frontend window and restart after install
    tmux send-keys -t "${TMUX_SESSION}:3" C-c ""
    sleep 2
    tmux send-keys -t "${TMUX_SESSION}:3" \
      "cd ${WORKSPACE} && \
       set -a && source .env && set +a && \
       pnpm --prefix modules/web install && \
       pnpm --prefix modules/web/web-app run dev -- \
         --host 0.0.0.0 \
         --port ${FRONTEND_PORT} \
         2>&1 | tee ${LOG_DIR}/frontend.log" \
      Enter
    log "Frontend restarted with updated deps."
  fi

  # ── Backend: cargo-watch detects changed .rs files and auto-recompiles ────
  log "cargo-watch will detect .rs changes and recompile automatically."

  # ── Check for .env.example changes (secrets/vars added) ──────────────────
  if echo "$CHANGED" | grep -q '\.env\.example'; then
    log "⚠️  .env.example changed — you may need to update .env and restart services manually."
  fi

  LAST_SHA="$NEW_SHA"
  log "Done. Next check in ${POLL_INTERVAL}s."
done
