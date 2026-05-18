# 7. Deployment & Operations

> *"Hope is not a strategy."* — Google SRE Book
>
> Life Manager's deployment is designed for reliability through simplicity: one binary, one database file, one Docker Compose command.

## The Build Pipeline

```
Source Code
    │
    ├─ input.css ──────────────────▶ Tailwind CLI ──▶ assets/main.css
    │
    ├─ src/ ───────────────────────▶ dx build ──────▶ target/dx/.../life_manager (server binary)
    │                                                  target/dx/.../public/ (WASM + assets)
    │
    └─ assets/ (sw.js, manifest,   ▶ Docker COPY ──▶ Container image
       icons, fonts)
```

### Why Local Build + Docker Copy?

The original Dockerfile built from source inside Docker. This required:
- Installing the Rust toolchain (~1GB)
- Installing `dioxus-cli`
- Adding the `wasm32-unknown-unknown` target
- Installing Node.js for Tailwind
- Full compilation from scratch (~5 minutes)

The current approach builds locally and copies the binary:
- Docker image is just `debian:trixie-slim` + Tesseract + the binary
- Build time: ~4 seconds (vs. ~5 minutes)
- Image size: ~200MB (vs. ~2GB with the build stage)

The trade-off: you must have the Rust toolchain locally. For a personal project, this is fine.

### The GLIBC Constraint

The binary is dynamically linked against glibc. The Docker base image must have a compatible glibc version. Arch Linux (the development host) ships glibc 2.39+, so the Docker image uses `debian:trixie-slim` (glibc 2.40). Using `debian:bookworm-slim` (glibc 2.36) would cause a `GLIBC_2.39 not found` error.

## Docker Compose Architecture

```yaml
services:
  tailscale:
    image: tailscale/tailscale:latest
    # Provides HTTPS tunnel + user identity headers
    volumes:
      - tailscale-state:/var/lib/tailscale  # Persistent auth state
      - ./ts-serve.json:/config/serve.json  # Serve config

  app:
    build: .
    network_mode: service:tailscale  # Shares Tailscale's network namespace
    volumes:
      - app-data:/app/data  # Persistent SQLite database
    depends_on:
      - tailscale
```

### Network Architecture

The `network_mode: service:tailscale` directive is critical. It means the app container shares the Tailscale container's network stack. The app binds to `0.0.0.0:8080`, and Tailscale's serve proxy routes HTTPS:443 to localhost:8080 — all within the same network namespace.

This means the app is never directly reachable from outside the Tailscale network. Even if Docker port mapping were misconfigured, the app only listens on the Tailscale container's interface.

## PWA Configuration

### The Static Asset Problem

Dioxus's build output includes only the compiled WASM, JavaScript, and CSS (with content hashes in filenames). It does NOT include:
- `sw.js` (service worker)
- `sw-register.js` (service worker registration)
- `manifest.json` (PWA manifest)
- `icons/` (app icons)
- `fonts/` (self-hosted JetBrains Mono)

These must be manually copied into the `public/` directory in the Dockerfile:

```dockerfile
COPY assets/sw.js ./public/sw.js
COPY assets/sw-register.js ./public/sw-register.js
COPY assets/manifest.json ./public/manifest.json
COPY assets/icons ./public/icons
COPY assets/fonts ./public/fonts
```

### Service Worker Strategy

The service worker uses a **stale-while-revalidate** strategy:

1. Check the cache for the request
2. If cached, return immediately AND fetch from network in background
3. Update the cache with the fresh response
4. If not cached, fetch from network, cache, and return

This provides instant loading for repeat visits while keeping content fresh. Since all API calls use POST (Dioxus server functions), they bypass the service worker cache — only static assets (HTML, CSS, JS, WASM, fonts, icons) are cached.

### Manifest Requirements

For a PWA to be installable, browsers require:
- A `manifest.json` with `name`, `icons`, `start_url`, `display`
- A registered service worker
- HTTPS (provided by Tailscale)
- Icons in at least 192x192 and 512x512 sizes

## Operational Scripts

### `scripts/deploy.sh`

The full deployment pipeline:

```bash
#!/usr/bin/env bash
set -euo pipefail

npx @tailwindcss/cli -i ./input.css -o ./assets/main.css --minify
dx build --release --platform web
docker compose build app
docker compose up -d

# Health check
STATUS=$(curl -s -o /dev/null -w "%{http_code}" https://lifemanager.tail6c1af7.ts.net/)
if [ "$STATUS" = "200" ]; then
    echo "Deploy successful!"
else
    echo "WARNING: HTTP $STATUS"
    docker compose logs app --tail 10
fi
```

### `scripts/screenshot.sh`

Uses Playwright to capture mobile-viewport screenshots of all pages:

```bash
npx playwright screenshot --browser firefox \
    --viewport-size="390,844" \
    "${BASE_URL}/${page}" \
    "${OUT_DIR}/lm-${page}.png"
```

This enables visual regression testing without a phone.

## Database Backup

The SQLite database lives in a Docker volume at `/app/data/life_manager.db`. To backup:

```bash
# Copy the DB file from the container
docker compose cp app:/app/data/life_manager.db ./backup.db

# Or from the Docker volume directly
docker run --rm -v life_manager_app-data:/data alpine \
    cp /data/life_manager.db /backup/
```

SQLite databases are single files — no dump/restore ceremony needed.

## Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `DATABASE_PATH` | `life_manager.db` | SQLite database file location |
| `REQUIRE_AUTH` | `false` | Require Tailscale header in production |
| `IP` | `0.0.0.0` | Server bind address |
| `PORT` | `8080` | Server bind port |
| `TS_AUTHKEY` | (none) | Tailscale ephemeral auth key (in `.env`) |
