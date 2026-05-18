# 1. Architecture Overview

> *"The limits of my language mean the limits of my world."* — Ludwig Wittgenstein
>
> Life Manager is built in Rust not because it's fashionable, but because when a single binary serves your frontend, backend, and database layer, the language's guarantees become your architecture's guarantees.

## The Big Picture

Life Manager is a **fullstack Progressive Web App** compiled from a single Rust codebase. The same source tree produces two artifacts: a WebAssembly binary that runs in the browser and a native server binary that handles API calls and database access. Dioxus 0.7 makes this possible through its fullstack feature, which generates client stubs for server functions at compile time.

```
┌─────────────────────────────────────────────────────────┐
│                    User's Phone                         │
│                                                         │
│  ┌─────────────────────────────────────────────────┐   │
│  │              Browser (PWA Mode)                  │   │
│  │                                                   │   │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────────┐  │   │
│  │  │  Dioxus   │  │  Service  │  │   Tailwind   │  │   │
│  │  │  WASM     │  │  Worker   │  │   CSS v4     │  │   │
│  │  │  (RSX UI) │  │  (Cache)  │  │   (Theme)    │  │   │
│  │  └─────┬─────┘  └──────────┘  └──────────────┘  │   │
│  │        │ HTTP POST (server functions)             │   │
│  └────────┼──────────────────────────────────────────┘   │
│           │                                              │
└───────────┼──────────────────────────────────────────────┘
            │ Tailscale Encrypted Tunnel
            ▼
┌─────────────────────────────────────────────────────────┐
│                  Docker Host                             │
│                                                          │
│  ┌──────────────┐     ┌──────────────────────────────┐  │
│  │  Tailscale    │────▶│       Life Manager Server    │  │
│  │  Sidecar      │     │                              │  │
│  │  (HTTPS:443)  │     │  ┌────────┐  ┌───────────┐  │  │
│  │               │     │  │ Axum   │  │ Tesseract │  │  │
│  │  Injects:     │     │  │ Router │  │ OCR       │  │  │
│  │  Tailscale-   │     │  │        │  │ (chi_tra) │  │  │
│  │  User-Login   │     │  └───┬────┘  └───────────┘  │  │
│  └──────────────┘     │      │                        │  │
│                        │  ┌───▼────────────────────┐  │  │
│                        │  │     SQLite (r2d2)      │  │  │
│                        │  │  /app/data/life_mgr.db │  │  │
│                        │  └────────────────────────┘  │  │
│                        └──────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

## Why This Stack?

### Rust + Dioxus: One Language, Both Sides

Traditional web applications maintain two codebases — a JavaScript frontend and a backend in whatever language the team prefers. This creates a boundary where data models must be defined twice, serialization bugs hide, and type mismatches only surface at runtime.

Dioxus eliminates this boundary. Models defined in `src/models/` are shared verbatim between client and server. When a `ChecklistItem` struct changes, the compiler catches every callsite in both the WASM frontend and the Axum backend.

The `#[server]` macro is the bridge. A function annotated with `#[server]` compiles into:
- **On the server**: the actual function body with database access
- **On the client**: an async stub that serializes arguments, sends an HTTP POST, and deserializes the response

This means `checklist::add_checklist("Buy milk".into(), ItemCategory::Grocery, None).await` looks identical whether called from a button handler (client) or a test (server).

### SQLite: The Embedded Database

For a personal app serving 1–3 users on a private Tailscale network, SQLite is the right choice. There is no separate database server to manage, no connection string to configure, and no network latency on queries. The database is a single file in a Docker volume, trivially backed up with `cp`.

Connection pooling via `r2d2` prevents contention when multiple requests arrive simultaneously. The pool is initialized once at startup and stored in a `OnceLock` singleton — Rust's equivalent of a lazily-initialized global.

### Tailscale: Zero-Config Networking

Instead of configuring TLS certificates, DNS records, and firewall rules, the app uses Tailscale as a sidecar container. Tailscale provides:
- HTTPS with automatic certificate management
- User identity via the `Tailscale-User-Login` header
- Network-level access control (only tailnet members can reach the app)

The app never touches the internet directly. It listens on `0.0.0.0:8080` inside the Docker network, and Tailscale's `serve` feature proxies HTTPS traffic to it.

## The Five Modules

Life Manager is organized around five independent modules, each following the same architectural pattern:

| Module | Model | Purpose |
|--------|-------|---------|
| To-Dos | `ChecklistItem` (category=Todo) | General task tracking |
| Groceries | `ChecklistItem` (category=Grocery) | Shopping list |
| Shopee Pick-ups | `ShopeePackage` | Convenience store package tracking |
| Watchlist | `WatchItem` | Movie/series/anime tracker |
| Cycle Tracker | `Cycle` | Menstrual cycle logging with predictions |

Todos and Groceries share the same model (`ChecklistItem`) and component (`ChecklistPage`), differentiated only by an enum variant and visual accent color. This is the app's primary deduplication — two pages, one component, zero duplicated logic.

## Request Lifecycle

Every user interaction follows the same path:

1. **User taps** a button or swipes an item
2. **Dioxus event handler** captures the gesture
3. **`spawn(async move { ... })`** launches an async task
4. **Server function stub** serializes arguments to JSON
5. **HTTP POST** to `/_server_fn/FunctionNameHash`
6. **Axum handler** deserializes, extracts Tailscale headers
7. **Auth check** via `user_from_headers(&headers)`
8. **Input validation** via `validate::text()`, `validate::date()`
9. **Database operation** via `rusqlite` with parameterized queries
10. **Response** serialized back to the client
11. **Signal update** (`items.set(new_data)`) triggers UI re-render
12. **Virtual DOM diff** updates only the changed elements

This lifecycle is consistent across all modules. The only variation is step 9 — the specific SQL query.

## File Organization

The codebase follows a layered architecture inspired by Domain-Driven Design, but without the ceremony:

```
src/
├── models/     ← Domain layer: what the data looks like
├── api/        ← Application layer: what you can do with it
├── server/     ← Infrastructure layer: how it's stored and secured
├── components/ ← Presentation layer: reusable UI building blocks
├── pages/      ← Composition layer: page-level component assembly
├── route.rs    ← Navigation: URL ↔ page mapping
└── main.rs     ← Bootstrap: entry point, global config
```

Each layer depends only on the layers above it. Pages depend on components and API. API depends on models and server. Models depend on nothing. This makes the codebase navigable: if you want to understand how groceries work, start at `pages/groceries.rs`, follow the imports to `components/checklist_page.rs`, then to `api/checklist.rs`, then to `server/db.rs`.
