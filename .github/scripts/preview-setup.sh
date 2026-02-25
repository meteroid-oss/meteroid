#!/usr/bin/env bash
# preview-setup.sh — Runs on the Hetzner box to start a PR preview environment.
#
# Environment variables (all required unless noted):
#   PR_NUMBER         — PR number (e.g. 127)
#   PR_BRANCH         — PR branch name
#   PR_SHA            — PR HEAD commit SHA
#   GITHUB_REPO       — org/repo (e.g. meteroid-oss/meteroid)
#   GH_TOKEN          — GitHub PAT with repo scope (used for git clone/fetch)
#   BASE_DOMAIN       — Base domain, e.g. preview.meteroid.dev
#   FRP_SERVER        — frp server address
#   FRP_PORT          — frp server port (default: 7000)
#   FRP_TOKEN         — frp auth token (optional)
#   COMMENT_ID        — GitHub comment ID to update with final URLs (optional)
#   GH_COMMENT_TOKEN  — GitHub token to update the comment (optional)
#
# Port allocation (unique per PR number, works for PRs up to ~999):
#   Postgres  : 54000 + PR_NUMBER  (local docker port)
#   gRPC      : 60000 + PR_NUMBER  (local listen + frp remote port)
#   REST API  : 62000 + PR_NUMBER  (local listen + frp remote port)
#   Frontend  : 20000 + PR_NUMBER  (local vite port, tunnelled via frp HTTP vhost)
#
# Outputs (printed to stdout for the workflow to parse):
#   PREVIEW_FRONTEND_URL=http://pr-N.BASE_DOMAIN
#   PREVIEW_GRPC_URL=http://BASE_DOMAIN:PORT
#   PREVIEW_REST_URL=http://BASE_DOMAIN:PORT

set -euo pipefail

# ── Constants ────────────────────────────────────────────────────────────────
DB_PORT=$(( 54000 + PR_NUMBER ))
GRPC_PORT=$(( 60000 + PR_NUMBER ))
REST_PORT=$(( 62000 + PR_NUMBER ))
FRONTEND_PORT=$(( 20000 + PR_NUMBER ))

WORKSPACE="/opt/meteroid-previews/pr-${PR_NUMBER}/repo"
TMUX_SESSION="meteroid-pr-${PR_NUMBER}"
COMPOSE_PROJECT="meteroid-pr-${PR_NUMBER}"
FRPC_CONFIG="/opt/meteroid-previews/pr-${PR_NUMBER}/frpc.toml"
LOG_DIR="/opt/meteroid-previews/pr-${PR_NUMBER}/logs"

FRONTEND_URL="http://pr-${PR_NUMBER}.${BASE_DOMAIN}"
GRPC_URL="http://${BASE_DOMAIN}:${GRPC_PORT}"
REST_URL="http://${BASE_DOMAIN}:${REST_PORT}"

# ── Helpers ───────────────────────────────────────────────────────────────────
log()  { echo "[preview-setup] $*"; }
die()  { echo "[preview-setup] ERROR: $*" >&2; exit 1; }

# ── Prerequisites check ───────────────────────────────────────────────────────
log "Checking prerequisites…"
command -v tmux    >/dev/null || die "tmux not found (apt install tmux)"
command -v docker  >/dev/null || die "docker not found"
command -v git     >/dev/null || die "git not found"
command -v cargo   >/dev/null || die "cargo/rustup not found — install via https://rustup.rs"
command -v pnpm    >/dev/null || die "pnpm not found (npm install -g pnpm)"
command -v frpc    >/dev/null || die "frpc not found — install frp client (https://github.com/fatedier/frp/releases)"
# cargo-watch is optional but strongly recommended for auto-reload
if ! command -v cargo-watch &>/dev/null; then
  log "cargo-watch not found — installing (one-time, ~30s)…"
  cargo install cargo-watch --quiet
fi

# ── Stop any existing preview for this PR ────────────────────────────────────
log "Stopping any existing preview for PR #${PR_NUMBER}…"
PR_NUMBER="$PR_NUMBER" bash /tmp/preview-scripts/preview-stop.sh --quiet 2>/dev/null || true

# ── Create directories ────────────────────────────────────────────────────────
mkdir -p "$WORKSPACE" "$LOG_DIR" "$(dirname "$FRPC_CONFIG")"

# ── Clone / update repository ────────────────────────────────────────────────
REPO_URL="https://x-access-token:${GH_TOKEN}@github.com/${GITHUB_REPO}.git"

if [[ -d "$WORKSPACE/.git" ]]; then
  log "Updating existing checkout…"
  cd "$WORKSPACE"
  git remote set-url origin "$REPO_URL"
  git fetch origin "$PR_BRANCH" --quiet
  git checkout "$PR_BRANCH"
  git reset --hard "origin/${PR_BRANCH}"
else
  log "Cloning repository (branch: ${PR_BRANCH})…"
  git clone --branch "$PR_BRANCH" "$REPO_URL" "$WORKSPACE"
  cd "$WORKSPACE"
fi

log "Checked out: $(git log --oneline -1)"

# ── Write .env ────────────────────────────────────────────────────────────────
log "Writing .env…"
cp .env.example .env

# Append preview-specific overrides (these come last so they win)
cat >> .env <<ENV
## ── Preview overrides for PR #${PR_NUMBER} ──────────────────────────────────
DATABASE_USER=meteroid
DATABASE_PASSWORD=secret
DATABASE_NAME=meteroid
DATABASE_URL=postgres://meteroid:secret@localhost:${DB_PORT}/meteroid?sslmode=disable

METEROID_API_LISTEN_ADDRESS=0.0.0.0:${GRPC_PORT}
METEROID_API_EXTERNAL_URL=${GRPC_URL}

METEROID_REST_API_LISTEN_ADDRESS=0.0.0.0:${REST_PORT}
METEROID_REST_API_EXTERNAL_URL=${REST_URL}

METEROID_PUBLIC_URL=${FRONTEND_URL}

VITE_METEROID_API_EXTERNAL_URL=${GRPC_URL}
VITE_METEROID_REST_API_EXTERNAL_URL=${REST_URL}
ENV

# ── Docker compose override (unique postgres port + container name) ────────────
log "Writing docker compose port override…"
cat > "/opt/meteroid-previews/pr-${PR_NUMBER}/compose-override.yml" <<COMPOSE
services:
  meteroid-db:
    container_name: meteroid-db-pr-${PR_NUMBER}
    ports:
      - "${DB_PORT}:5432"
    volumes:
      - /opt/meteroid-previews/pr-${PR_NUMBER}/pg_data:/var/lib/postgresql/data
COMPOSE

# ── frpc configuration ────────────────────────────────────────────────────────
log "Writing frpc config…"
{
  cat <<FRPC
serverAddr = "${FRP_SERVER}"
serverPort = ${FRP_PORT:-7000}
FRPC

  # Auth section (only if token is provided)
  if [[ -n "${FRP_TOKEN:-}" ]]; then
    cat <<FRPC_AUTH

[auth]
method = "token"
token = "${FRP_TOKEN}"
FRPC_AUTH
  fi

  cat <<FRPC_PROXIES

# Frontend — HTTP vhost (frp routes *.${BASE_DOMAIN} → local vite)
# hostHeaderRewrite prevents Vite's host-check from rejecting the custom domain
[[proxies]]
name = "pr-${PR_NUMBER}-frontend"
type = "http"
localPort = ${FRONTEND_PORT}
customDomains = ["pr-${PR_NUMBER}.${BASE_DOMAIN}"]
hostHeaderRewrite = "127.0.0.1"

# gRPC API — TCP passthrough
[[proxies]]
name = "pr-${PR_NUMBER}-grpc"
type = "tcp"
localPort = ${GRPC_PORT}
remotePort = ${GRPC_PORT}

# REST API — TCP passthrough
[[proxies]]
name = "pr-${PR_NUMBER}-rest"
type = "tcp"
localPort = ${REST_PORT}
remotePort = ${REST_PORT}
FRPC_PROXIES
} > "$FRPC_CONFIG"

# ── Print URLs early (workflow can pick these up even if something fails later)
echo "PREVIEW_FRONTEND_URL=${FRONTEND_URL}"
echo "PREVIEW_GRPC_URL=${GRPC_URL}"
echo "PREVIEW_REST_URL=${REST_URL}"

# ── Start tmux session ────────────────────────────────────────────────────────
log "Starting tmux session: ${TMUX_SESSION}"
tmux new-session -d -s "$TMUX_SESSION" -x 220 -y 50

# ── Window 0 — Postgres ───────────────────────────────────────────────────────
log "Starting postgres (docker compose project: ${COMPOSE_PROJECT})…"
tmux rename-window -t "${TMUX_SESSION}:0" "postgres"
tmux send-keys -t "${TMUX_SESSION}:0" \
  "cd $WORKSPACE && docker compose \
    -f docker/develop/docker-compose-lite.yml \
    -f /opt/meteroid-previews/pr-${PR_NUMBER}/compose-override.yml \
    --env-file .env \
    -p $COMPOSE_PROJECT \
    up 2>&1 | tee $LOG_DIR/postgres.log" \
  Enter

# Wait for postgres to be healthy (up to 90 s)
log "Waiting for postgres to be healthy…"
WAITED=0
until docker exec "meteroid-db-pr-${PR_NUMBER}" pg_isready -U meteroid -d meteroid >/dev/null 2>&1; do
  sleep 5
  WAITED=$(( WAITED + 5 ))
  (( WAITED >= 90 )) && die "Postgres did not become healthy within 90 s"
done
log "Postgres is ready."

# ── Window 1 — frp tunnel ─────────────────────────────────────────────────────
log "Starting frp tunnel…"
tmux new-window -t "$TMUX_SESSION" -n "tunnel"
tmux send-keys -t "${TMUX_SESSION}:1" \
  "frpc -c $FRPC_CONFIG 2>&1 | tee $LOG_DIR/frpc.log" \
  Enter
sleep 3   # give frp a moment to register proxies

# ── Window 2 — Backend (standalone, auto-reload via cargo-watch) ──────────────
log "Starting backend (standalone mode, cargo-watch for auto-reload)…"
tmux new-window -t "$TMUX_SESSION" -n "backend"
tmux send-keys -t "${TMUX_SESSION}:2" \
  "cd $WORKSPACE && set -a && source .env && set +a && \
   cargo watch --why \
     -x 'run --bin standalone' \
     2>&1 | tee $LOG_DIR/backend.log" \
  Enter

# ── Window 3 — Frontend (Vite dev, watch mode) ────────────────────────────────
log "Starting frontend dev server on port ${FRONTEND_PORT}…"
tmux new-window -t "$TMUX_SESSION" -n "frontend"
tmux send-keys -t "${TMUX_SESSION}:3" \
  "cd $WORKSPACE && \
   set -a && source .env && set +a && \
   ([ -d modules/web/node_modules ] || pnpm --prefix modules/web install) && \
   pnpm --prefix modules/web/web-app run dev -- \
     --host 0.0.0.0 \
     --port ${FRONTEND_PORT} \
     2>&1 | tee $LOG_DIR/frontend.log" \
  Enter

# ── Window 4 — PR watcher ─────────────────────────────────────────────────────
log "Starting PR change watcher…"
tmux new-window -t "$TMUX_SESSION" -n "watcher"
tmux send-keys -t "${TMUX_SESSION}:4" \
  "export PR_NUMBER='${PR_NUMBER}' PR_BRANCH='${PR_BRANCH}' \
          GH_TOKEN='${GH_TOKEN}' GITHUB_REPO='${GITHUB_REPO}' \
          WORKSPACE='${WORKSPACE}' TMUX_SESSION='${TMUX_SESSION}' \
          FRONTEND_PORT='${FRONTEND_PORT}' LOG_DIR='${LOG_DIR}'; \
   bash /tmp/preview-scripts/preview-watch.sh 2>&1 | tee $LOG_DIR/watcher.log" \
  Enter

log ""
log "═══════════════════════════════════════════════════════════════"
log "  Preview environment started for PR #${PR_NUMBER}"
log "  Frontend : ${FRONTEND_URL}"
log "  gRPC API : ${GRPC_URL}"
log "  REST API : ${REST_URL}"
log ""
log "  tmux session : ${TMUX_SESSION}"
log "  Attach with  : tmux attach -t ${TMUX_SESSION}"
log ""
log "  ⚠️  Backend is still compiling — check window 2 for progress."
log "═══════════════════════════════════════════════════════════════"

# ── Optionally update the GitHub comment with live URLs ───────────────────────
if [[ -n "${COMMENT_ID:-}" && -n "${GH_COMMENT_TOKEN:-}" ]]; then
  log "Updating GitHub comment #${COMMENT_ID} with live URLs…"
  SHA_SHORT="${PR_SHA:0:8}"
  BODY="## ✅ Preview Environment Ready

| Service | URL |
|---------|-----|
| 🌐 Frontend | [${FRONTEND_URL}](${FRONTEND_URL}) |
| ⚡ gRPC API | \`${GRPC_URL}\` |
| 🔌 REST API | \`${REST_URL}\` |

**Branch:** \`${PR_BRANCH}\` @ \`${SHA_SHORT}\`

💡 Auto-updates when you push new commits to this PR.
🛑 Comment \`/stop-preview\` to stop it.

> ⏳ Backend is still compiling on first run — the frontend will load once it's ready."

  curl -s -X PATCH \
    -H "Authorization: token ${GH_COMMENT_TOKEN}" \
    -H "Content-Type: application/json" \
    "https://api.github.com/repos/${GITHUB_REPO}/issues/comments/${COMMENT_ID}" \
    -d "$(jq -n --arg body "$BODY" '{body: $body}')" \
    > /dev/null
fi
