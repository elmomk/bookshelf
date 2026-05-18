# PWA Features & Deployment Architecture

A practical guide to understanding and maintaining Life Manager's progressive web app
capabilities, offline data layer, build pipeline, and Docker + Tailscale deployment.

---

## Table of Contents

1. [Progressive Web App Fundamentals](#1-progressive-web-app-fundamentals)
2. [Service Worker Strategy](#2-service-worker-strategy)
3. [Offline-First Data Layer](#3-offline-first-data-layer)
4. [Tailwind CSS v4](#4-tailwind-css-v4)
5. [Dioxus Fullstack Build](#5-dioxus-fullstack-build)
6. [Docker Deployment](#6-docker-deployment)
7. [Tailscale Sidecar Pattern](#7-tailscale-sidecar-pattern)
8. [CI/CD & Scripts](#8-cicd--scripts)

---

## 1. Progressive Web App Fundamentals

> *Reference: "Building Progressive Web Apps" by Tal Ater (O'Reilly, 2017)*

A Progressive Web App is a web application that meets three criteria: it is served
over HTTPS, it has a web app manifest, and it registers a service worker. Life Manager
satisfies all three.

### The Web App Manifest

The manifest at `assets/manifest.json` tells the browser how to present the app when
installed:

```json
{
  "name": "Life Manager",
  "short_name": "LifeMgr",
  "start_url": "/",
  "display": "standalone",
  "orientation": "portrait",
  "background_color": "#08080f",
  "theme_color": "#00f0ff",
  "icons": [
    { "src": "/icons/icon-192.png", "sizes": "192x192", "type": "image/png", "purpose": "any" },
    { "src": "/icons/icon-512.png", "sizes": "512x512", "type": "image/png", "purpose": "any" },
    { "src": "/icons/icon-192.png", "sizes": "192x192", "type": "image/png", "purpose": "maskable" },
    { "src": "/icons/icon-512.png", "sizes": "512x512", "type": "image/png", "purpose": "maskable" }
  ]
}
```

Key fields:

- **`display: "standalone"`** -- the app runs without browser chrome (address bar, tabs),
  looking and feeling like a native app.
- **`orientation: "portrait"`** -- locks to portrait on mobile, matching the mobile-first
  design.
- **`background_color: "#08080f"`** -- the splash screen background while the app loads,
  matching `cyber-black`.
- **`theme_color: "#00f0ff"`** -- the status bar and window color, matching `neon-cyan`.
- **Icons** are provided at 192px and 512px with both `any` (standard) and `maskable`
  (adaptive icon) purposes. Android uses maskable icons to fit the device's icon shape.

### Install Prompts and Home Screen Behavior

- **Android (Chrome):** Chrome shows an "Add to Home Screen" mini-infobar automatically
  once the PWA criteria are met. Users can also install via the browser menu. Once
  installed, the app launches in standalone mode with the manifest's theme color.
- **iOS (Safari):** Safari does not show automatic install prompts. Users must tap
  Share > Add to Home Screen. The `apple-mobile-web-app-capable` and
  `apple-mobile-web-app-status-bar-style` meta tags (injected by `sw-register.js`)
  ensure the app launches fullscreen with a translucent status bar.

### HTTPS via Tailscale

Browsers require HTTPS for service worker registration. Life Manager gets automatic
HTTPS through Tailscale Serve (see [Section 7](#7-tailscale-sidecar-pattern)), which
provisions TLS certificates for the `*.ts.net` domain. No manual certificate
management is needed.

---

## 2. Service Worker Strategy

> *Reference: "Service Workers in Action" (MDN Web Docs); "Offline Web Applications"
> by Ben Galbraith & Dion Almaer*

### Registration Flow

The service worker lifecycle starts in `assets/sw-register.js`, which Dioxus loads as
a script resource (configured in `Dioxus.toml`):

```toml
[web.resource]
script = ["/sw-register.js"]
```

The registration script does two things before Dioxus even hydrates:

1. **Injects PWA head tags** into the static HTML -- manifest link, theme-color meta,
   apple-touch-icon, and Apple mobile web app meta tags. This must happen early because
   browsers check the static HTML (not the post-hydration DOM) for installability.

2. **Registers the service worker** at `/sw.js` with root scope:

```js
if ('serviceWorker' in navigator) {
  window.addEventListener('load', function() {
    navigator.serviceWorker.register('/sw.js', { scope: '/' });
  });
  navigator.serviceWorker.addEventListener('controllerchange', function() {
    window.location.reload();
  });
}
```

The `controllerchange` listener is critical: when a new service worker takes over
(after a deploy), the page reloads automatically so users always get the latest code.

### Caching Strategy

The service worker (`assets/sw.js`) uses a **dual strategy** depending on request type:

**Navigation requests (HTML pages) -- Network-First:**

```js
if (event.request.mode === 'navigate') {
  event.respondWith(
    fetch(event.request)
      .then((response) => {
        const clone = response.clone();
        caches.open(CACHE_NAME).then((cache) => cache.put(event.request, clone));
        return response;
      })
      .catch(() => caches.match(event.request))
  );
  return;
}
```

The app always tries the network first for page loads. If successful, the response is
cached for offline fallback. This ensures new deploys take effect immediately -- users
do not get stale HTML.

**Static assets (WASM, JS, CSS, fonts, icons) -- Stale-While-Revalidate:**

```js
event.respondWith(
  caches.match(event.request).then((cached) => {
    const fetched = fetch(event.request).then((response) => {
      if (response.ok) {
        const clone = response.clone();
        caches.open(CACHE_NAME).then((cache) => cache.put(event.request, clone));
      }
      return response;
    }).catch(() => cached);
    return cached || fetched;
  })
);
```

If a cached copy exists, it is returned immediately (fast load). In the background,
the service worker fetches the latest version and updates the cache. On the next
page load, the updated asset is served. This gives the best of both worlds: instant
load times for returning users and eventual consistency with the server.

Non-GET requests (server function calls, form submissions) are not intercepted at all
-- they pass straight through to the network.

### Update Lifecycle

The cache is versioned with `CACHE_NAME = 'life-manager-v2'`. When this string is
changed in a deploy:

1. The browser detects a byte-changed `sw.js` and installs the new service worker.
2. `skipWaiting()` in the `install` handler makes the new worker activate immediately
   (no waiting for all tabs to close).
3. The `activate` handler deletes all caches that do not match the new `CACHE_NAME`.
4. `clients.claim()` makes the new worker take control of all open tabs.
5. The `controllerchange` listener in `sw-register.js` reloads the page.

---

## 3. Offline-First Data Layer

> *Reference: "Designing Data-Intensive Applications" by Martin Kleppmann (O'Reilly,
> 2017), Chapter 5: Replication*

### The Cache Module

`src/cache.rs` provides a thin abstraction over `localStorage` for client-side data
persistence. It is dual-compiled: on WASM it uses the Web Storage API; on the server
it is a no-op.

```rust
pub fn read<T: DeserializeOwned>(key: &str) -> Option<T> {
    #[cfg(target_arch = "wasm32")]
    {
        let storage = web_sys::window()?.local_storage().ok()??;
        let json = storage.get_item(&format!("lm_{key}")).ok()??;
        serde_json::from_str(&json).ok()
    }
    #[cfg(not(target_arch = "wasm32"))]
    { None }
}

pub fn write<T: Serialize>(key: &str, data: &T) {
    // Serializes to JSON and stores under "lm_{key}" in localStorage
}
```

All cache keys are prefixed with `lm_` to avoid collisions. A separate
`write_sync_time()` / `read_sync_time()` pair stores the timestamp of the last
successful server sync, used by the sync indicator.

### SyncStatus Enum

```rust
pub enum SyncStatus {
    Synced,     // Data matches server, green dot
    Syncing,    // Fetch in progress, pulsing cyan dot
    CachedOnly, // Server unreachable, showing stale data, orange dot
}
```

This enum is provided as a Dioxus context signal from `AppLayout` and consumed by
every page and the `SyncIndicator` component.

### Load-Then-Fetch Pattern

Every page follows the same data loading pattern, demonstrated here with the
Watchlist page:

```rust
use_effect(move || {
    // Step 1: Load cached data immediately (instant render, no loading spinner)
    if let Some(cached) = cache::read::<Vec<WatchItem>>("watchlist") {
        items.set(cached);
    }
    // Step 2: Fetch fresh data from server
    reload();
});
```

The `reload` closure:

```rust
let reload = move || {
    spawn(async move {
        sync_status.set(SyncStatus::Syncing);
        match watchlist_api::list_watchlist().await {
            Ok(loaded) => {
                cache::write("watchlist", &loaded);   // Update cache
                cache::write_sync_time();              // Record sync time
                items.set(loaded);                      // Update UI
                sync_status.set(SyncStatus::Synced);
            }
            Err(e) => {
                // If we already have cached items, just show them
                if items.read().is_empty() {
                    error_msg.set(Some(format!("Failed to load: {e}")));
                }
                sync_status.set(SyncStatus::CachedOnly);
            }
        }
    });
};
```

This pattern ensures:

- **Instant perceived load**: cached data renders before the network request completes.
- **Graceful offline**: if the server is unreachable, the UI shows cached data with an
  orange "OFFLINE" indicator instead of a blank screen or error.
- **Fresh on success**: when the server responds, the UI updates and the cache is
  overwritten.

### The Sync Indicator

`src/components/sync_indicator.rs` renders in the app header and shows:

| Status | Dot Color | Label |
|--------|-----------|-------|
| Synced | Green | Time since last sync ("JUST NOW", "5m AGO", etc.) |
| Syncing | Cyan (pulsing) | "SYNCING..." |
| CachedOnly | Orange | "OFFLINE" |

Tapping the indicator increments a `SyncTrigger` context signal, which pages watch
via `use_effect` to re-fetch data on demand.

### Conflict Resolution: Server Wins (Last-Write-Wins)

The app uses a simple conflict resolution strategy: the server's SQLite database is
the single source of truth. When a mutation (add, complete, delete) is performed, it
calls a server function directly. If the call fails, the UI shows an error. There is
no offline mutation queue or client-side conflict merging.

In Kleppmann's terminology, this is a **single-leader** topology with **last-write-wins**
semantics. The tradeoff is simplicity: no need for vector clocks, CRDTs, or merge
logic. The cost is that mutations require connectivity -- but for a household app on
a private Tailscale network, this is an acceptable constraint.

---

## 4. Tailwind CSS v4

> *Reference: [Tailwind CSS v4 Documentation](https://tailwindcss.com/docs)*

### v4 vs. v3

Tailwind CSS v4 introduced a fundamentally different configuration approach:

- **No `tailwind.config.js`** -- all configuration lives in CSS using `@theme` and
  `@import` directives.
- **CSS-native** -- v4 uses CSS custom properties (variables) and cascade layers
  internally.
- **Faster** -- the new Oxide engine is written in Rust and significantly faster.

### The Cyberpunk Theme

The theme is defined in `input.css` (project root) using `@theme`:

```css
@import "tailwindcss";

@theme {
  --color-cyber-black: #08080f;
  --color-cyber-dark: #0d0d18;
  --color-cyber-card: #12122a;
  --color-cyber-border: #1a1a3e;
  --color-neon-cyan: #00f0ff;
  --color-neon-green: #00ff88;
  --color-neon-magenta: #ff00ff;
  --color-neon-orange: #ff8800;
  --color-neon-purple: #a855f7;
  --color-neon-pink: #ec4899;
  --color-neon-yellow: #facc15;
  /* ... */
  --font-family-mono: "JetBrains Mono", ui-monospace, monospace;
}
```

These become usable as standard Tailwind utilities: `bg-cyber-black`, `text-neon-cyan`,
`font-mono`, etc.

Glow effects are achieved with custom utility classes like `text-glow-cyan` using
CSS `text-shadow` with the neon colors. A `scanlines` class adds a subtle CRT-style
overlay via a repeating gradient pseudo-element.

### Build Process

Tailwind CSS is compiled separately from Dioxus, using the standalone CLI:

```bash
npx @tailwindcss/cli -i ./input.css -o ./assets/main.css --minify
```

- **Input**: `./input.css` (theme + imports)
- **Output**: `./assets/main.css` (placed in Dioxus's `asset_dir`)
- The `--minify` flag is used for production builds; dev mode uses `--watch` for
  hot reloading.

Dioxus picks up `assets/main.css` automatically because `asset_dir = "assets"` in
`Dioxus.toml`.

---

## 5. Dioxus Fullstack Build

> *Reference: "Programming WebAssembly with Rust" by Kevin Hoffman (Pragmatic
> Programmers, 2019)*

### One Codebase, Two Targets

Dioxus fullstack compiles the same Rust source into two artifacts:

1. **WASM bundle** (client) -- runs in the browser, handles rendering, interactivity,
   and localStorage caching.
2. **Native binary** (server) -- an Axum HTTP server that serves the WASM bundle,
   handles API calls, and manages the SQLite database.

`Dioxus.toml` sets this up:

```toml
[application]
name = "life_manager"
default_platform = "fullstack"
```

### cfg Gating

Server-only code is gated behind `#[cfg(not(target_arch = "wasm32"))]`. This includes:

- The SQLite database module (`src/server/db.rs`)
- Authentication logic (`src/server/auth.rs`)
- Any dependency that cannot compile to WASM (e.g., `rusqlite`, `tesseract`)

The cache module (`src/cache.rs`) demonstrates the opposite: WASM-only code uses
`#[cfg(target_arch = "wasm32")]`, while server builds get a no-op stub.

### Server Functions

Functions annotated with `#[server]` are the bridge between client and server:

```rust
#[server(headers: axum::http::HeaderMap)]
pub async fn list_watchlist() -> Result<Vec<WatchItem>, ServerFnError> {
    let user = auth::user_from_headers(&headers)?;
    let db = db::connect()?;
    // ... query SQLite, return results
}
```

At compile time, Dioxus generates:

- **On the server**: the actual function implementation.
- **On the client**: a stub that serializes the arguments, sends an HTTP POST to a
  generated endpoint (e.g., `/api/list_watchlist`), and deserializes the response.

The `headers` parameter is injected by the macro -- it gives server functions access to
the incoming HTTP headers (used for Tailscale-based authentication).

### The base_path Bug

**Never set `base_path` in `Dioxus.toml`.** In Dioxus 0.7, setting `base_path` causes
server function endpoints to be generated with the wrong URL prefix, breaking all API
calls. The app must be served from the root path `/`.

---

## 6. Docker Deployment

> *Reference: "Docker in Action" by Jeff Nickoloff & Stephen Kuenzli (Manning, 2019)*

### Why No Rust Build in Docker

The `Dockerfile` does not compile Rust. Instead, it copies a pre-built binary:

```dockerfile
FROM debian:trixie-slim

# Copy locally-built binary and public assets
COPY target/dx/life_manager/release/web/life_manager ./life_manager
COPY target/dx/life_manager/release/web/public ./public
```

This is a deliberate choice:

- **Speed**: A Rust + WASM build requires `rustup`, `wasm-pack`, the `wasm32` target,
  and hundreds of crate compilations. This takes 5-10+ minutes. Copying a pre-built
  binary takes seconds.
- **Image size**: No Rust toolchain in the image means a much smaller final image.
- **Simplicity**: The build machine (developer laptop) already has the full toolchain
  configured correctly.

### The Base Image: debian:trixie-slim

`debian:trixie-slim` is chosen over `alpine` or `scratch` because the app needs:

- **Tesseract OCR** (`tesseract-ocr`) for parsing Shopee delivery screenshots
- **Chinese Traditional language pack** (`tesseract-ocr-chi-tra`) since Shopee Taiwan
  receipts contain Chinese text
- **CA certificates** (`ca-certificates`) for HTTPS connections

```dockerfile
RUN apt-get update && apt-get install -y \
    ca-certificates tesseract-ocr tesseract-ocr-chi-tra tesseract-ocr-eng \
    && rm -rf /var/lib/apt/lists/*
```

### What Gets Copied

The Dockerfile copies assets in two stages:

1. **Dioxus build output**: the server binary and the `public/` directory (which
   contains the WASM bundle, hashed JS/CSS assets, and `index.html`).

2. **PWA files that Dioxus omits**: Dioxus does not include the service worker,
   manifest, icons, or custom fonts in its build output. These are copied manually:

```dockerfile
COPY assets/sw.js ./public/sw.js
COPY assets/sw-register.js ./public/sw-register.js
COPY assets/manifest.json ./public/manifest.json
COPY assets/icons ./public/icons
COPY assets/fonts ./public/fonts
```

### Non-Root User

```dockerfile
RUN useradd -r -m -s /bin/false appuser
```

A dedicated `appuser` is created, and `/app` is owned by this user. The app runs as
a non-root user by default. This limits the blast radius of any potential security
vulnerability -- the process cannot modify system files or access other users' data.

### Data Persistence

```dockerfile
RUN mkdir -p /app/data && chown -R appuser:appuser /app
VOLUME /app/data
ENV DATABASE_PATH=/app/data/life_manager.db
```

The SQLite database is stored in a Docker volume mounted at `/app/data`. This ensures
data survives container recreation. The `DATABASE_PATH` environment variable tells the
application where to find (or create) the database file.

---

## 7. Tailscale Sidecar Pattern

> *Reference: "Kubernetes in Action" by Marko Luksa (Manning, 2018), Chapter 3:
> Sidecar Containers*

### The Sidecar Container Pattern

In `docker-compose.yml`, two containers share a network namespace:

```yaml
services:
  tailscale:
    image: tailscale/tailscale:latest
    hostname: lifemanager
    environment:
      - TS_AUTHKEY=${TS_AUTHKEY}
      - TS_STATE_DIR=/var/lib/tailscale
      - TS_SERVE_CONFIG=/config/serve.json
    volumes:
      - tailscale-state:/var/lib/tailscale
      - ./ts-serve.json:/config/serve.json:ro
    cap_add:
      - NET_ADMIN
      - SYS_MODULE

  app:
    build: .
    network_mode: service:tailscale
    depends_on:
      - tailscale
```

The key line is `network_mode: service:tailscale`. This makes the `app` container
share the Tailscale container's network stack. From the app's perspective, it binds
to `0.0.0.0:8080` as usual. But from the outside, traffic reaches it through the
Tailscale container's network interface.

This is the sidecar pattern: one container provides a cross-cutting concern (secure
networking) while the other focuses on application logic. Neither needs to know about
the other's internals.

### HTTPS Termination with TS_SERVE_CONFIG

The `ts-serve.json` file configures Tailscale Serve:

```json
{
  "TCP": {
    "443": { "HTTPS": true }
  },
  "Web": {
    "${TS_CERT_DOMAIN}:443": {
      "Handlers": {
        "/": { "Proxy": "http://127.0.0.1:8080" }
      }
    }
  }
}
```

This tells Tailscale to:

1. Listen on TCP port 443 with HTTPS enabled.
2. Automatically provision a TLS certificate for the `*.ts.net` domain.
3. Reverse-proxy all HTTPS requests to `http://127.0.0.1:8080` (the app).

Because the app and Tailscale share a network namespace, `127.0.0.1:8080` reaches the
app container directly. The result: zero-config HTTPS with automatic certificate
renewal.

### Why Tailscale for a Household App

Tailscale provides:

- **Private network**: the app is only accessible to devices on your tailnet. No
  public IP, no port forwarding, no firewall rules.
- **Automatic HTTPS**: TLS certificates are provisioned and renewed automatically.
- **User identity**: Tailscale injects headers identifying the authenticated user. The
  app reads these via `auth::user_from_headers(&headers)` for access control and
  `auth::display_name_from_headers(&headers)` to attribute actions (e.g., "Mo completed
  this item").
- **Zero configuration**: `TS_AUTHKEY` is the only secret needed. The rest is automatic.

---

## 8. CI/CD & Scripts

All scripts live in `scripts/` and follow a consistent pattern: `set -euo pipefail`
(fail fast on any error), echo progress messages, and exit cleanly.

### deploy.sh -- Full Pipeline

The deploy script orchestrates the entire build-to-deploy pipeline:

```bash
#!/usr/bin/env bash
set -euo pipefail

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
STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
  https://lifemanager.tail6c1af7.ts.net/ 2>/dev/null || echo "000")
if [ "$STATUS" = "200" ]; then
    echo "==> Deploy successful! (HTTP $STATUS)"
else
    echo "==> WARNING: HTTP $STATUS -- checking logs..."
    docker compose logs app --tail 10
fi
```

The steps in order:

1. **Tailwind build** -- compiles `input.css` to `assets/main.css` with minification.
2. **Dioxus release build** -- compiles both the WASM client bundle and the native
   server binary. Output lands in `target/dx/life_manager/release/web/`.
3. **Docker build** -- builds the image, copying the pre-built artifacts.
4. **Docker deploy** -- starts (or restarts) the containers in detached mode.
5. **Health check** -- waits 2 seconds, then curls the production URL. If the response
   is not HTTP 200, it dumps the last 10 lines of container logs for debugging.

### build.sh -- Build Only

Same as deploy but stops after the Dioxus build. Useful for verifying the build
succeeds without deploying:

```bash
npx @tailwindcss/cli -i ./input.css -o ./assets/main.css --minify
dx build --release --platform web
```

### dev.sh -- Development Server

Runs Tailwind in watch mode (background process) alongside the Dioxus dev server:

```bash
npx @tailwindcss/cli -i ./input.css -o ./assets/main.css --watch &
TAILWIND_PID=$!
trap "kill $TAILWIND_PID 2>/dev/null; exit" INT TERM
dx serve
```

The `trap` ensures the Tailwind watcher is killed when `dx serve` exits or the user
presses Ctrl+C. The dev server runs on `http://localhost:8080` with hot reloading
(configured via `Dioxus.toml`'s `[web.watcher]` section).

### check.sh -- Type Checking

A minimal script that runs `cargo check` -- verifies the code compiles without
producing build artifacts. Useful as a fast feedback loop during development.

### screenshot.sh -- Documentation Screenshots

Uses Playwright to capture mobile-viewport screenshots of all five pages:

```bash
PAGES=("todos" "groceries" "shopee" "watchlist" "period")
for page in "${PAGES[@]}"; do
    npx playwright screenshot --browser "$BROWSER" \
      --viewport-size="390,844" \
      "${BASE_URL}/${page}" "${OUT_DIR}/lm-${page}.png"
done
```

Defaults to the production URL with a 390x844 viewport (iPhone 14 dimensions) and
Firefox. The base URL, output directory, and browser can all be overridden via
positional arguments.

---

## Summary

The deployment pipeline flows as follows:

```
input.css ──> Tailwind CLI ──> assets/main.css
                                     │
src/**/*.rs ──> dx build ──> target/dx/.../life_manager (binary)
                              target/dx/.../public/ (WASM + assets)
                                     │
                              ┌──────┴──────┐
                              │  Dockerfile  │
                              │  - binary    │
                              │  - public/   │
                              │  - sw.js     │
                              │  - manifest  │
                              │  - icons     │
                              │  - fonts     │
                              └──────┬──────┘
                                     │
                         docker compose up -d
                                     │
                     ┌───────────────┴───────────────┐
                     │ tailscale container           │
                     │ (HTTPS termination, auth)     │
                     │         network_mode ─────────│──> app container
                     │                               │    (Axum server on :8080)
                     └───────────────────────────────┘
                                     │
                     https://lifemanager.tail6c1af7.ts.net/
```

The user's browser loads the WASM bundle, renders the UI, reads cached data from
localStorage for instant display, then fetches fresh data from the server. The service
worker caches assets for offline access and auto-reloads when a new deploy is detected.
