#!/usr/bin/env bash
set -euo pipefail

# Take Playwright screenshots of all pages at mobile viewport
BASE_URL="${1:-https://bookclub.tail6c1af7.ts.net}"
OUT_DIR="${2:-/tmp}"
BROWSER="${3:-firefox}"
SIZE="390,844"

PAGES=("books" "activity")

echo "==> Taking screenshots at ${SIZE} (${BROWSER})..."
for page in "${PAGES[@]}"; do
    echo "    ${page}..."
    npx playwright screenshot --browser "$BROWSER" --viewport-size="$SIZE" "${BASE_URL}/${page}" "${OUT_DIR}/bc-${page}.png" 2>/dev/null
done

echo "==> Screenshots saved to ${OUT_DIR}/bc-*.png"
