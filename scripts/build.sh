#!/usr/bin/env bash
set -euo pipefail

echo "==> Building Tailwind CSS..."
npx @tailwindcss/cli -i ./input.css -o ./assets/main.css --minify

echo "==> Building Dioxus app (release)..."
dx build --release --platform web

echo "==> Build complete!"
echo "    Output: target/dx/bookclub/release/web/"
