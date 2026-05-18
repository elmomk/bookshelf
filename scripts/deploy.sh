#!/usr/bin/env bash
set -euo pipefail

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
