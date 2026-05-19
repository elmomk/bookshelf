#!/usr/bin/env bash
set -euo pipefail

# --- Mandatory: back up the SQLite DB before any deploy ---------------------
# Copies the live DB (+ WAL/SHM so a restore is consistent) out of the running
# container. If the app IS running the backup MUST succeed or we abort — never
# deploy over un-backed-up data. First deploy (no container yet) safely skips.
echo "==> Backing up database..."
mkdir -p backups
BK_TS="$(date -u +%Y%m%d-%H%M%S)"
APP_CID="$(docker compose ps -q app 2>/dev/null || true)"
if [ -n "$APP_CID" ] && [ -n "$(docker ps -q --no-trunc | grep -F "$APP_CID" || true)" ]; then
    if docker compose cp "app:/app/data/bookclub.db" "backups/bookclub-${BK_TS}.db"; then
        docker compose cp "app:/app/data/bookclub.db-wal" "backups/bookclub-${BK_TS}.db-wal" 2>/dev/null || true
        docker compose cp "app:/app/data/bookclub.db-shm" "backups/bookclub-${BK_TS}.db-shm" 2>/dev/null || true
        echo "    saved backups/bookclub-${BK_TS}.db ($(du -h "backups/bookclub-${BK_TS}.db" | cut -f1))"
        # Keep the 20 most recent backup sets.
        ls -1t backups/bookclub-*.db 2>/dev/null | tail -n +21 | while read -r old; do
            rm -f "$old" "${old}-wal" "${old}-shm"
        done
    else
        echo "    ERROR: backup failed while the app container is running — aborting deploy." >&2
        exit 1
    fi
else
    echo "    No running app container (first deploy?) — nothing to back up, skipping."
fi

echo "==> Stamping service-worker cache version (auto cache-bust)..."
SW_BUILD_ID="$(date -u +%Y%m%d-%H%M%S)-$(git rev-parse --short HEAD 2>/dev/null || echo nogit)"
restore_sw() { git checkout -- assets/sw.js 2>/dev/null || true; }
trap restore_sw EXIT
sed -i "s/^const CACHE_NAME = '.*';/const CACHE_NAME = 'bookclub-${SW_BUILD_ID}';/" assets/sw.js
echo "    CACHE_NAME = bookclub-${SW_BUILD_ID}"

echo "==> Building Tailwind CSS..."
npx @tailwindcss/cli -i ./input.css -o ./assets/main.css --minify

echo "==> Building Dioxus app (release)..."
dx build --release --platform web

echo "==> Building Docker image..."
docker compose build app

echo "==> Deploying..."
docker compose up -d

echo "==> Waiting for startup..."
sleep 2

echo "==> Checking health..."
STATUS=$(curl -s -o /dev/null -w "%{http_code}" https://bookclub.tail6c1af7.ts.net/ 2>/dev/null || echo "000")
if [ "$STATUS" = "200" ]; then
    echo "==> Deploy successful! (HTTP $STATUS)"
else
    echo "==> WARNING: HTTP $STATUS — checking logs..."
    docker compose logs app --tail 10
fi
