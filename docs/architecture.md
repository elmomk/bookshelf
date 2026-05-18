# Life Manager Architecture Tutorial

This document is a deep-dive into the architecture and design patterns behind Life Manager, a mobile-first PWA built with Rust, Dioxus 0.7, and SQLite. Each section maps a real pattern in the codebase to a foundational concept from software engineering literature, with code excerpts drawn directly from the source.

---

## Table of Contents

1. [Fullstack Architecture](#1-fullstack-architecture)
2. [State Management](#2-state-management)
3. [Offline-First with Cache](#3-offline-first-with-cache)
4. [Database Design](#4-database-design)
5. [Authentication & Authorization](#5-authentication--authorization)
6. [Fire-and-Forget Side Effects](#6-fire-and-forget-side-effects)
7. [Component Architecture](#7-component-architecture)

---

## 1. Fullstack Architecture

**Reference:** *Clean Architecture* by Robert C. Martin (2017)

Martin's central thesis is the **Dependency Rule**: source code dependencies must point inward, from infrastructure toward domain entities. Life Manager implements this through four layers, each corresponding to a directory in the codebase.

### The Layers

```
src/
  models/          -- Entities (innermost ring)
  api/             -- Use Cases / Server Functions
  pages/           -- UI / Presenters
  components/      -- UI building blocks
  server/          -- Infrastructure (outermost ring)
    db.rs          -- SQLite connection pool
    auth.rs        -- Tailscale identity extraction
    google.rs      -- Google Calendar integration
  cache.rs         -- Client-side localStorage abstraction
```

**Entities** (`src/models/`) are plain data structs with no framework dependencies. They derive `Serialize` and `Deserialize` so they can cross the WASM-server boundary, but they know nothing about databases, HTTP, or UI:

```rust
// src/models/checklist_item.rs
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChecklistItem {
    pub id: String,
    pub text: String,
    pub date: Option<NaiveDate>,
    pub done: bool,
    pub category: ItemCategory,
    pub created_at: f64,
    pub completed_by: Option<String>,
}
```

**Use Cases** (`src/api/`) are Dioxus server functions. They orchestrate business logic -- accepting input, calling infrastructure, returning domain types. The `#[server]` macro compiles these into HTTP endpoints on the server and RPC stubs on the client:

```rust
// src/api/checklist.rs
#[server(headers: axum::http::HeaderMap)]
pub async fn add_checklist(
    text: String,
    category: ItemCategory,
    date: Option<String>,
) -> Result<(), ServerFnError> {
    use crate::server::{auth, db, validate};

    let user_id = auth::user_from_headers(&headers).map_err(|e| ServerFnError::new(e))?;
    validate::text(&text, "text")?;
    // ... insert into DB, fire-and-forget Calendar sync
    Ok(())
}
```

**Presenters / Pages** (`src/pages/`) call into the API layer but never touch infrastructure directly. A page like `Shopee` calls `shopee_api::list_shopee()` without knowing whether that function runs locally or across the network.

**Infrastructure** (`src/server/`) is gated behind `#[cfg(not(target_arch = "wasm32"))]` so it never compiles into the browser bundle. This is enforced at the module level in `src/main.rs`:

```rust
// src/main.rs
#[cfg(not(target_arch = "wasm32"))]
mod server;
```

### How Dioxus 0.7 Fullstack Compilation Works

A single `cargo` invocation produces two artifacts:

1. A **WASM binary** for the browser, containing models, pages, components, cache, and server-function *stubs* (which serialize arguments and POST them to the server).
2. A **native binary** for the server, containing everything the WASM build has *plus* the `server/` module and the actual server-function implementations.

The router (`src/route.rs`) drives client-side navigation via the Dioxus `Router`, while server functions are transparently dispatched as HTTP calls:

```rust
// src/route.rs
#[derive(Routable, Clone, Debug, PartialEq)]
pub enum Route {
    #[layout(AppLayout)]
        #[route("/todos")]
        Todos {},
        #[route("/groceries")]
        Groceries {},
        #[route("/shopee")]
        Shopee {},
        #[route("/watchlist")]
        Watchlist {},
        #[route("/period")]
        Period {},
    #[end_layout]
    #[redirect("/", || Route::Todos {})]
    #[route("/:..segments")]
    NotFound { segments: Vec<String> },
}
```

The key insight from Clean Architecture: the `models/` layer has zero `use crate::server::` imports, the `api/` layer imports both `models` and `server` (but only under `#[server]` blocks), and the `pages/` layer imports `api` and `models` but never `server`. Dependencies always point inward.

---

## 2. State Management

**Reference:** *Programming Rust* by Jim Blandy, Jason Orendorff & Leonora Tindall (2021)

Blandy and Orendorff emphasize Rust's ownership model as the foundation for safe concurrency. Dioxus builds on this with **signals** -- reactive smart pointers that track reads and writes at runtime, borrowing Rust's borrow-checker philosophy for UI state.

### Signals and Effects

Every page uses `use_signal` for local state and `use_effect` for side effects. Here is the canonical pattern from `ChecklistPage`:

```rust
// src/components/checklist_page.rs
let mut items = use_signal(Vec::<ChecklistItem>::new);
let mut error_msg = use_signal(|| Option::<String>::None);
let mut sync_status: Signal<SyncStatus> = use_context();

let reload = move || {
    spawn(async move {
        sync_status.set(SyncStatus::Syncing);
        match checklist::list_checklist(category).await {
            Ok(loaded) => {
                cache::write(cache_key, &loaded);
                cache::write_sync_time();
                items.set(loaded);
                sync_status.set(SyncStatus::Synced);
            }
            Err(e) => {
                if items.read().is_empty() {
                    error_msg.set(Some(format!("Failed to load: {e}")));
                }
                sync_status.set(SyncStatus::CachedOnly);
            }
        }
    });
};
```

### The Reload Pattern (Direct Re-fetch After Mutations)

Life Manager deliberately avoids reactive subscriptions or client-side caches that attempt to mirror server state. Instead, after every mutation (add, toggle, delete), the page calls `reload()` to re-fetch the full list from the server:

```rust
// After adding an item:
match checklist::add_checklist(text, category, date).await {
    Ok(()) => {
        input_text.set(String::new());
        input_date.set(None);
        reload();  // <-- re-fetch from server
    }
    Err(e) => error_msg.set(Some(format!("Failed to add: {e}"))),
}
```

Why not optimistic updates with local patching? Three reasons:

1. **Correctness over cleverness.** The server is the source of truth. Re-fetching guarantees the client never diverges from server state, even when multiple users (via Tailscale sharing) mutate simultaneously.
2. **Simplicity.** There is no reconciliation logic, no conflict resolution, no partial update code paths. The reload closure is 10 lines.
3. **Latency is acceptable.** The app runs on a private Tailscale network where round-trip times are typically under 50ms. The re-fetch is imperceptible.

### Global State via Context

The sync status is shared across the entire app using `use_context_provider` (in the layout) and `use_context` (in pages):

```rust
// src/components/layout.rs -- provider
let sync_status = use_context_provider(|| Signal::new(SyncStatus::Syncing));
let mut sync_trigger = use_context_provider(|| Signal::new(SyncTrigger(0)));

// src/components/checklist_page.rs -- consumer
let mut sync_status: Signal<SyncStatus> = use_context();
let sync_trigger: Signal<SyncTrigger> = use_context();
```

The `SyncTrigger` is an incrementing counter. When the header's sync button is pressed, it increments; a `use_effect` in each page watches for changes:

```rust
use_effect(move || {
    let _trigger = sync_trigger.read().0;
    reload();
});
```

This is a manual "pub/sub" mechanism: the trigger is the event, and reading it inside `use_effect` subscribes to future changes. It is intentionally primitive -- no event bus, no global store, just a signal and a closure.

---

## 3. Offline-First with Cache

**Reference:** *Designing Data-Intensive Applications* by Martin Kleppmann (2017)

Kleppmann describes the tension between consistency and availability in distributed systems. Life Manager is a two-node system (browser + server) that chooses **availability with eventual consistency**: the app always renders immediately from cache, then reconciles with the server.

### The Cache Module

`src/cache.rs` provides four functions -- `read`, `write`, `read_sync_time`, `write_sync_time` -- all backed by the browser's `localStorage` on WASM and no-ops on the server:

```rust
// src/cache.rs
pub fn read<T: DeserializeOwned>(key: &str) -> Option<T> {
    #[cfg(target_arch = "wasm32")]
    {
        let storage = web_sys::window()?.local_storage().ok()??;
        let json = storage.get_item(&format!("lm_{key}")).ok()??;
        serde_json::from_str(&json).ok()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = key;
        None
    }
}
```

Note the conditional compilation: the same module compiles to real persistence on WASM and to a harmless no-op on the server. This mirrors Kleppmann's principle that storage layers should be pluggable.

### Sync Status State Machine

The `SyncStatus` enum models three states:

```rust
// src/cache.rs
pub enum SyncStatus {
    Synced,      // Server fetch succeeded, cache is fresh
    Syncing,     // Fetch in progress
    CachedOnly,  // Server unreachable, showing stale data
}
```

The lifecycle on every page load:

1. **Instant render from cache:** `use_effect` fires, calls `cache::read()`, populates `items` immediately. The user sees data in under 16ms.
2. **Background sync:** `reload()` fires a server function call. The status moves to `Syncing`.
3. **Reconcile:** On success, `cache::write()` updates localStorage and `items.set()` updates the UI. Status moves to `Synced`. On failure, status moves to `CachedOnly`, and the stale cached data remains visible.

```rust
// src/components/checklist_page.rs
use_effect(move || {
    if let Some(cached) = cache::read::<Vec<ChecklistItem>>(cache_key) {
        items.set(cached);
    }
    reload();
});
```

The `SyncIndicator` component in the header visualizes this state machine with a colored dot (green/pulsing cyan/orange) and a human-readable timestamp:

```rust
// src/components/sync_indicator.rs
let (dot_color, dot_anim) = match *status.read() {
    SyncStatus::Synced => ("bg-neon-green", ""),
    SyncStatus::Syncing => ("bg-neon-cyan", "animate-pulse"),
    SyncStatus::CachedOnly => ("bg-neon-orange", ""),
};
```

This is not full offline-first (writes require connectivity). But for reads, it follows Kleppmann's pattern of "read your writes from cache, reconcile in background" -- a pragmatic middle ground for a personal-use app.

---

## 4. Database Design

**Reference:** *SQL Antipatterns* by Bill Karwin (2010)

Karwin warns against over-engineering database schemas and advocates for simplicity appropriate to the problem domain. Life Manager follows this philosophy with a single-file SQLite database.

### Why SQLite

The app serves a household (1-3 users) via a private Tailscale network. There is no horizontal scaling requirement, no multi-region deployment, no need for a client-server database protocol. SQLite offers:

- **Zero administration.** No daemon, no configuration files, no backups to coordinate (the Docker volume handles persistence).
- **Atomic transactions.** ACID-compliant with WAL mode.
- **Single-file portability.** The entire database can be copied or backed up with `cp`.

### WAL Mode and Pragmas

The initialization in `src/server/db.rs` sets three critical pragmas:

```rust
// src/server/db.rs
conn.pragma_update(None, "journal_mode", "WAL")
    .expect("Failed to set WAL mode");
conn.pragma_update(None, "synchronous", "NORMAL")
    .expect("Failed to set synchronous mode");
conn.pragma_update(None, "busy_timeout", 5000)
    .expect("Failed to set busy_timeout");
```

- **WAL (Write-Ahead Logging):** Allows concurrent readers while a writer is active. Without WAL, any write locks the entire database, blocking reads. WAL is the standard recommendation for web-serving SQLite workloads.
- **NORMAL synchronous:** With WAL, `NORMAL` is safe against application crashes (only an OS crash + power loss at the exact wrong moment could cause data loss). `FULL` would add an fsync per transaction -- unnecessary for a Docker container that restarts on failure.
- **busy_timeout:** Instead of returning `SQLITE_BUSY` immediately when another connection holds a lock, wait up to 5 seconds. This prevents spurious failures under concurrent access from the r2d2 pool.

### Connection Pooling with r2d2

```rust
// src/server/db.rs
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;

static POOL: OnceLock<DbPool> = OnceLock::new();

pub fn init() {
    let db_path = std::env::var("DATABASE_PATH")
        .unwrap_or_else(|_| "life_manager.db".to_string());
    let manager = SqliteConnectionManager::file(&db_path)
        .with_init(|conn| {
            conn.pragma_update(None, "busy_timeout", 5000)?;
            Ok(())
        });
    let pool = Pool::new(manager).expect("Failed to create DB pool");
    // ...
    POOL.set(pool).expect("DB pool already initialized");
}

pub fn pool() -> &'static DbPool {
    POOL.get().expect("DB pool not initialized")
}
```

The pool is stored in a `OnceLock<DbPool>` -- a static that is initialized exactly once and then provides `&'static` references for the lifetime of the process. Every server function acquires a connection from the pool with `db::pool().get()`, uses it synchronously, and returns it when the connection goes out of scope.

### Idempotent Migrations

Karwin's chapter on "Metadata Tribbles" warns against fragile migration systems. Life Manager takes the simplest possible approach:

1. **`CREATE TABLE IF NOT EXISTS`** for initial schema -- these are always safe to re-run.
2. **`ALTER TABLE ADD COLUMN`** wrapped in error handlers that ignore "duplicate column" errors:

```rust
// src/server/db.rs
for sql in [
    "ALTER TABLE checklist_items ADD COLUMN completed_by TEXT",
    "ALTER TABLE shopee_packages ADD COLUMN completed_by TEXT",
    "ALTER TABLE watch_items ADD COLUMN completed_by TEXT",
] {
    if let Err(e) = conn.execute_batch(sql) {
        let msg = e.to_string();
        if !msg.contains("duplicate column") {
            eprintln!("WARNING: migration failed: {msg}");
        }
    }
}
```

3. **Named one-time migrations** tracked in a `migrations` table for data transformations:

```rust
// src/server/db.rs
fn run_once(conn: &rusqlite::Connection, name: &str, sql: &str) {
    let already_run: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM migrations WHERE name = ?1",
            rusqlite::params![name],
            |row| row.get(0),
        )
        .unwrap_or(false);

    if !already_run {
        let _ = conn.execute_batch(sql);
        let _ = conn.execute(
            "INSERT INTO migrations (name) VALUES (?1)",
            rusqlite::params![name],
        );
    }
}
```

Every migration strategy here is idempotent: the app can restart at any time, re-run all migrations, and arrive at the correct schema. There is no ordered migration numbering, no rollback mechanism -- appropriate simplicity for a single-database, single-developer project.

---

## 5. Authentication & Authorization

**Reference:** *The Web Application Hacker's Handbook* by Dafydd Stuttard & Marcus Pinto (2011)

Stuttard and Pinto emphasize that authentication mechanisms must match the threat model. Life Manager's threat model is narrow: the app is only accessible via a Tailscale private network, so the network layer itself provides identity.

### Tailscale-Based Identity

Tailscale injects identity headers into every request that passes through its `ts-serve` reverse proxy. The auth module extracts this:

```rust
// src/server/auth.rs
pub fn user_from_headers(headers: &axum::http::HeaderMap) -> Result<String, String> {
    let require_auth = std::env::var("REQUIRE_AUTH").unwrap_or_default() == "true";
    if require_auth {
        if headers.get("Tailscale-User-Login").is_none() {
            return Err("Unauthorized: missing Tailscale-User-Login header".to_string());
        }
    }
    Ok("default".to_string())
}
```

Two modes of operation:

- **Production (`REQUIRE_AUTH=true`):** The `Tailscale-User-Login` header must be present. If it is missing, the request is rejected. Note that this header cannot be forged -- it is injected by the Tailscale daemon, not by the client.
- **Development (`REQUIRE_AUTH` unset or `false`):** All requests are accepted as the "default" user. This allows local development without a Tailscale connection.

All data is scoped to a single shared `"default"` user ID. The app is a household tool, not a multi-tenant SaaS. But it still tracks *who* completed an item via `display_name_from_headers`:

```rust
// src/server/auth.rs
pub fn display_name_from_headers(headers: &axum::http::HeaderMap) -> String {
    let name = headers
        .get("Tailscale-User-Login")
        .and_then(|v| v.to_str().ok())
        .map(|login| login.split('@').next().unwrap_or(login).to_string())
        .unwrap_or_else(|| "local".to_string());
    name.chars().take(50).collect()
}
```

This is stored in the `completed_by` column so the UI can show "mo" or "partner" next to completed items. The 50-character truncation prevents storage abuse.

### Why This Is Secure

The Hacker's Handbook warns against trusting client-supplied headers. But Tailscale headers are not client-supplied -- they are injected by the Tailscale sidecar container *after* authenticating the client's WireGuard identity. The `docker-compose.yml` shows the topology:

```yaml
# docker-compose.yml
services:
  tailscale:
    image: tailscale/tailscale:latest
    # ... handles TLS termination and identity injection

  app:
    build: .
    network_mode: service:tailscale  # shares Tailscale's network namespace
```

The `network_mode: service:tailscale` means the app container shares the Tailscale container's network namespace. It listens on `0.0.0.0:8080`, but that port is only reachable through Tailscale's encrypted tunnel. There is no path from the public internet to the app.

---

## 6. Fire-and-Forget Side Effects

**Reference:** *Concurrency in Go* by Katherine Cox-Buday (2017)

Cox-Buday's patterns for goroutine-based concurrency translate directly to Tokio's `spawn` in async Rust. The key principle: when a side effect is non-critical and should not block the user, launch it concurrently and handle errors via logging, not propagation.

### The Pattern

Every mutation that affects Google Calendar uses `tokio::spawn` to fire off the sync without blocking the HTTP response:

```rust
// src/api/checklist.rs -- inside add_checklist
if let Some(ref d) = date {
    let id2 = id.clone();
    let text2 = text.clone();
    let d2 = d.clone();
    tokio::spawn(async move {
        crate::server::google::sync_item(&id2, &text2, Some(&d2), false, None).await;
    });
}

Ok(())  // Returns immediately to the client
```

The `Ok(())` returns to the client *before* the Calendar API call completes. The spawned task runs in the background on the Tokio runtime.

### Error Handling via Tracing

The `sync_item` function wraps the entire Calendar operation in an `async` block and logs failures:

```rust
// src/server/google.rs
pub async fn sync_item(
    item_id: &str, title: &str, date: Option<&str>,
    done: bool, google_event_id: Option<&str>,
) {
    if !is_configured() {
        return;  // No-op if Google Calendar is not set up
    }

    let result: Result<(), String> = async {
        if done {
            // Delete Calendar event...
        } else if let Some(date) = date {
            // Create or update Calendar event...
        }
        Ok(())
    }.await;

    if let Err(e) = result {
        tracing::warn!("Google Calendar sync failed for item {item_id}: {e}");
    }
}
```

Why `tracing::warn!` instead of returning an error? Because the user's action (adding a to-do) already succeeded. The Calendar sync is a bonus -- if Google's API is down, or the token expired, or the network blipped, the user should not see an error. The warning appears in server logs for debugging, but the UI is unaffected.

### Data Cloning for `'static` Lifetimes

Note the `.clone()` calls before every `tokio::spawn`. This is a direct consequence of Rust's ownership model (Blandy & Orendorff again): the spawned future must be `'static`, meaning it cannot borrow from the enclosing scope. Every piece of data the future needs must be moved into it:

```rust
let id2 = id.clone();
let text2 = text.clone();
let d2 = d.clone();
tokio::spawn(async move {
    crate::server::google::sync_item(&id2, &text2, Some(&d2), false, None).await;
});
```

This is the Rust equivalent of Go's "capture by value in a goroutine closure." It is verbose but prevents data races at compile time.

### Graceful Degradation

The `is_configured()` guard ensures the entire Calendar integration is a no-op when the environment variable is not set:

```rust
pub fn is_configured() -> bool {
    std::env::var("GOOGLE_SA_KEY_FILE").is_ok()
}
```

This means the app works perfectly without Google Calendar. The integration is additive, never a dependency.

---

## 7. Component Architecture

**Reference:** *Atomic Design* by Brad Frost (2016)

Frost's methodology organizes UI into five levels: atoms, molecules, organisms, templates, and pages. Life Manager's `src/components/` and `src/pages/` directories map cleanly to this hierarchy.

### Atoms: Single-Responsibility Building Blocks

**ErrorBanner** -- A dismissible error message. Takes a `Signal<Option<String>>` and renders when populated:

```rust
// src/components/error_banner.rs
#[component]
pub fn ErrorBanner(message: Signal<Option<String>>) -> Element {
    let msg = message.read().clone();
    if let Some(text) = msg {
        let mut message = message;
        rsx! {
            div { class: "bg-neon-magenta/10 border border-neon-magenta/40 ...",
                span { class: "flex-1", "{text}" }
                button {
                    onclick: move |_| message.set(None),
                    "\u{00d7}"
                }
            }
        }
    } else {
        rsx! {}
    }
}
```

**SyncIndicator** -- Displays sync state as a colored dot and label. Purely presentational:

```rust
// src/components/sync_indicator.rs
#[component]
pub fn SyncIndicator(
    status: Signal<SyncStatus>,
    on_sync: EventHandler<()>,
) -> Element { /* ... */ }
```

**SwipeItem** -- A touch-gesture wrapper. Tracks touch coordinates, determines direction and threshold, triggers callbacks. It knows nothing about what it contains:

```rust
// src/components/swipe_item.rs
#[component]
pub fn SwipeItem(
    children: Element,
    on_swipe_right: Option<EventHandler<()>>,
    on_swipe_left: EventHandler<()>,
    completed: bool,
) -> Element { /* ... */ }
```

The swipe logic uses a 100px threshold (`THRESHOLD: f64 = 100.0`) and a direction-lock mechanism: the first 10px of movement determines whether the gesture is horizontal or vertical. This prevents accidental swipes during vertical scrolling.

### Molecules: Composed Atoms with Behavior

**QuickAdd** -- Combines chip buttons with an edit mode toggle. Each chip is an atom; the editing state and event routing make it a molecule:

```rust
// src/components/quick_add.rs
#[component]
pub fn QuickAdd(
    chips: Vec<String>,
    on_select: EventHandler<String>,
    on_delete: Option<EventHandler<String>>,
    #[props(default = "cyan")] accent: &'static str,
) -> Element { /* ... */ }
```

**ShopeeOcr** -- Camera capture + Tesseract OCR invocation + result parsing. A molecule because it composes a file input (atom) with server-side processing and result transformation.

### Organisms: Self-Contained Feature Blocks

**ChecklistPage** -- The most complex reusable component. It composes ErrorBanner, QuickAdd, SwipeItem, and manages its own state (items, chips, input fields, loading state, sync status). It is parameterized by category, making it reusable for both To-Dos and Groceries:

```rust
// src/components/checklist_page.rs
#[component]
pub fn ChecklistPage(
    category: ItemCategory,
    placeholder: &'static str,
    initial_chips: Vec<String>,
    empty_text: &'static str,
    done_label: &'static str,
    accent_color: &'static str,
) -> Element { /* ... */ }
```

The Todos page and Groceries page are thin wrappers that call `ChecklistPage` with different props -- different accent colors, different placeholder text, different default chips, but identical behavior.

### Templates: Page Shells

**AppLayout** -- The template that wraps every page. It provides the header (with title and sync indicator), the content outlet, and the tab bar:

```rust
// src/components/layout.rs
#[component]
pub fn AppLayout() -> Element {
    let route: Route = use_route();
    let sync_status = use_context_provider(|| Signal::new(SyncStatus::Syncing));
    let mut sync_trigger = use_context_provider(|| Signal::new(SyncTrigger(0)));

    rsx! {
        div { class: "scanlines min-h-screen bg-cyber-black text-cyber-text font-mono",
            header { /* ... title, sync indicator ... */ }
            main { class: "pt-14 pb-16 max-w-lg mx-auto",
                Outlet::<Route> {}
            }
            TabBar {}
        }
    }
}
```

`AppLayout` is declared as a layout in the router (`#[layout(AppLayout)]`), so Dioxus wraps every route's content inside it. The `Outlet::<Route> {}` is where the current page's content renders.

### Pages: Route-Specific Compositions

Pages are the outermost UI layer. They compose organisms and molecules into complete screens. The `Shopee` page, for example, wires together `ErrorBanner`, `ShopeeOcr`, `SwipeItem`, and store-selection chips into a cohesive pickup tracker:

```rust
// src/pages/shopee.rs
#[component]
pub fn Shopee() -> Element {
    let mut items = use_signal(Vec::<ShopeePackage>::new);
    // ... state setup, reload closure, cache loading ...

    rsx! {
        div { class: "p-4 space-y-4",
            ErrorBanner { message: error_msg }
            div { /* add form with ShopeeOcr */ }
            div { /* item list with SwipeItem */ }
        }
    }
}
```

### The Complete Hierarchy

```
Pages (route-specific)
  Todos, Groceries        --> thin wrappers around ChecklistPage
  Shopee                  --> composes ShopeeOcr + SwipeItem + ErrorBanner
  Watchlist               --> composes SwipeItem + ErrorBanner
  Period                  --> standalone cycle tracker

Templates (page shells)
  AppLayout               --> header + outlet + tab bar

Organisms (self-contained feature blocks)
  ChecklistPage           --> add form + quick-add + item list + state management

Molecules (composed atoms with behavior)
  QuickAdd                --> chip list + edit mode
  ShopeeOcr               --> camera input + OCR processing

Atoms (single-responsibility)
  ErrorBanner             --> dismissible error message
  SyncIndicator           --> sync state dot + label
  SwipeItem               --> touch gesture wrapper
  TabBar                  --> bottom navigation
  Icons                   --> SVG icon components
```

---

## Cross-Cutting Concerns

### Input Validation

Server functions validate all user input before database operations. The `validate` module (invoked in API functions like `add_checklist` and `add_shopee`) checks text length, format, and presence -- preventing SQL injection at the application level even though parameterized queries already prevent it at the database level. This is defense in depth.

### Error Propagation

Errors flow through two channels:

1. **Server function errors** (`ServerFnError`) propagate back to the client as structured error responses. Pages display them via `ErrorBanner`.
2. **Background task errors** (fire-and-forget) are logged via `tracing::warn!` and never reach the client.

This separation ensures that user-facing operations always report failures, while background syncs degrade silently.

### The Service Worker

The PWA's service worker (`assets/sw.js`) is not compiled from Rust -- it is a plain JavaScript file copied into the Docker image. It handles offline caching of static assets. The registration script (`assets/sw-register.js`) is injected into the page via the Dioxus build output. This is a pragmatic choice: service workers must be JavaScript, so they live outside the Rust build pipeline.

---

## Summary

| Concept | Pattern | Reference |
|---------|---------|-----------|
| Layered separation | Models -> API -> Pages -> Server | *Clean Architecture* (Martin) |
| Reactive state | Signals + direct re-fetch after mutations | *Programming Rust* (Blandy & Orendorff) |
| Offline reads | localStorage cache + sync status state machine | *Designing Data-Intensive Applications* (Kleppmann) |
| Database | SQLite WAL + idempotent migrations | *SQL Antipatterns* (Karwin) |
| Auth | Tailscale header-based identity on private network | *Web Application Hacker's Handbook* (Stuttard & Pinto) |
| Background sync | `tokio::spawn` + `tracing::warn!` error handling | *Concurrency in Go* (Cox-Buday) |
| Component hierarchy | Atoms -> Molecules -> Organisms -> Templates -> Pages | *Atomic Design* (Frost) |
