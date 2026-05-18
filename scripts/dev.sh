#!/usr/bin/env bash
set -euo pipefail

echo "==> Building Tailwind CSS (watch mode)..."
npx @tailwindcss/cli -i ./input.css -o ./assets/main.css --watch &
TAILWIND_PID=$!

trap "kill $TAILWIND_PID 2>/dev/null; exit" INT TERM

echo "==> Starting Dioxus dev server on http://localhost:8080..."
dx serve

kill $TAILWIND_PID 2>/dev/null
