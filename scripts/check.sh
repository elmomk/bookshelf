#!/usr/bin/env bash
set -euo pipefail

echo "==> Type checking..."
cargo check

echo "==> All checks passed!"
