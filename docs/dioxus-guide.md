# Dioxus 0.7 Fullstack Development Guide

A practical tutorial for Rust developers new to Dioxus, based on the patterns used in
this Life Manager PWA. Every code snippet references a real file in the codebase.

> **References:**
> - [Dioxus 0.7 official docs](https://dioxuslabs.com/learn/0.7/)
> - *Programming Rust*, 2nd ed. -- Blandy, Orendorff & Tindall (O'Reilly, 2021)
> - *Patterns of Enterprise Application Architecture* -- Martin Fowler (Addison-Wesley, 2002)
> - *Functional Programming in Scala* -- Chiusano & Bjarnason (Manning, 2014) -- for reactive/effect patterns

---

## 1. Dioxus 0.7 RSX Syntax

Dioxus uses the `rsx!{}` macro instead of a template language. It looks like JSX but
follows Rust syntax rules.

### Basic structure

```rust
// src/main.rs:23-36
#[component]
fn App() -> Element {
    rsx! {
        document::Stylesheet { href: CSS }
        document::Meta { name: "viewport", content: "width=device-width, initial-scale=1" }
        Router::<Route> {}
    }
}
```

Key differences from JSX:
- **Curly braces, not angle brackets** for the macro body: `rsx! { ... }`
- **Comma-free attributes:** `name: "value"` not `name="value"` -- attributes are
  separated by commas only when on the same line
- **Children follow attributes** inside the same braces, separated by a comma after the
  last attribute
- **No `cx` parameter** -- Dioxus 0.7 dropped the `Scope` / `cx` argument entirely.
  Components are plain functions returning `Element`.

### Attribute syntax

```rust
// src/components/swipe_item.rs:56-57
div {
    class: "relative bg-cyber-card border border-cyber-border rounded-lg p-4 {opacity}",
    style: "transform: translateX({tx}px)",
    // children...
}
```

- Attributes use `key: value` syntax
- String interpolation works with `{variable}` inside double-quoted strings -- just like
  Rust's `format!()` macro
- Reserved Rust keywords need the `r#` prefix: `r#type: "text"`, `r#for: "input-id"`

### Event handlers

```rust
// src/components/checklist_page.rs:128-132
form {
    onsubmit: move |e| {
        e.prevent_default();
        let text = input_text.read().clone();
        do_add(text);
    },
    // ...
}
```

- Events use `oneventname: move |e| { ... }` syntax
- The event object provides methods like `prevent_default()`, `stop_propagation()`,
  `value()` (for input events), and `data()` (for touch events)

### Conditional rendering

```rust
// src/components/checklist_page.rs:156-174
if loading() {
    svg { /* spinner */ }
} else {
    "ADD"
}
```

Standard Rust `if`/`else` expressions work directly inside `rsx!`. No ternary operator
needed -- just use `if`/`else` blocks.

### Optional rendering

```rust
// src/components/checklist_page.rs:266-268
if let Some(date) = &item.date {
    span { class: "text-xs text-cyber-dim font-mono", "{date}" }
}
```

`if let` works inside RSX for rendering optional values. This is idiomatic for
`Option<T>` fields.

### List rendering

```rust
// src/components/checklist_page.rs:200-202
for item in items.read().iter() {
    { render_checklist_item(item.clone(), done_label, category, reload, reload_chips, error_msg) }
}
```

`for` loops work directly in RSX. When calling a helper function that returns `Element`,
wrap the call in `{ }` braces.

### Text nodes

```rust
// Literal strings are text nodes:
"ADD PACKAGE"

// Interpolated strings:
"{pkg.title}"
```

Bare string literals become text nodes. Use `{expr}` for interpolation.

---

## 2. Component Patterns

### The `#[component]` attribute and props

Every component is an annotated function. Props are function parameters:

```rust
// src/components/checklist_page.rs:12-20
#[component]
pub fn ChecklistPage(
    category: ItemCategory,
    placeholder: &'static str,
    initial_chips: Vec<String>,
    empty_text: &'static str,
    done_label: &'static str,
    accent_color: &'static str,
) -> Element {
    // ...
}
```

The `#[component]` macro generates a props struct behind the scenes. Callers pass props
as named attributes:

```rust
// src/pages/groceries.rs:11-18
ChecklistPage {
    category: ItemCategory::Grocery,
    placeholder: "Add grocery item...",
    initial_chips: CHIPS.iter().map(|s| s.to_string()).collect(),
    empty_text: "No grocery items yet",
    done_label: "GOT IT",
    accent_color: "green",
}
```

**Optional props** use `Option<T>`:

```rust
// src/components/swipe_item.rs:9
on_swipe_right: Option<EventHandler<()>>,
```

**Default values** use the `#[props]` attribute:

```rust
// src/components/quick_add.rs:8
#[props(default = "cyan")] accent: &'static str,
```

**Children** are passed as an `Element` prop named `children`:

```rust
// src/components/swipe_item.rs:8-9
#[component]
pub fn SwipeItem(
    children: Element,
    on_swipe_right: Option<EventHandler<()>>,
    on_swipe_left: EventHandler<()>,
    completed: bool,
) -> Element { ... }
```

Callers pass children as nested content:

```rust
SwipeItem {
    completed: picked_up,
    on_swipe_right: move |_| { ... },
    on_swipe_left: move |_| { ... },
    // Everything below is the `children` prop:
    div { class: "space-y-1",
        p { "Hello" }
    }
}
```

### EventHandler for callbacks

Use `EventHandler<T>` for callback props. Call them with `.call(value)`:

```rust
// src/components/shopee_ocr.rs:7
pub fn ShopeeOcr(on_results: EventHandler<Vec<OcrResult>>) -> Element { ... }

// Calling it:
on_results.call(results);
```

### The render function pattern

Extract complex list items into standalone functions that return `Element`. This is
analogous to Fowler's *Template View* pattern -- the component owns the data and
lifecycle, while render functions handle presentation:

```rust
// src/pages/shopee.rs:232-292
fn render_package(
    pkg: ShopeePackage,
    reload: impl Fn() + Copy + 'static,
    mut error_msg: Signal<Option<String>>,
) -> Element {
    let id = pkg.id.clone();
    rsx! {
        SwipeItem {
            completed: picked_up,
            on_swipe_right: move |_| { ... },
            on_swipe_left: move |_| { ... },
            div { /* package content */ }
        }
    }
}
```

These are plain functions, not components -- they don't use `#[component]` and don't
have their own hook state. This makes them lightweight and avoids prop struct generation.

Call them from RSX with braces:

```rust
for pkg in items.read().iter() {
    { render_package(pkg.clone(), reload, error_msg) }
}
```

### Component composition: template components

`ChecklistPage` demonstrates the *Template Method* pattern (Fowler) -- a reusable
component that encapsulates the full CRUD lifecycle for a checklist, parameterised
by category-specific values:

```rust
// src/pages/todos.rs -- just 26 lines for a complete CRUD page
#[component]
pub fn Todos() -> Element {
    rsx! {
        div {
            ChecklistPage {
                category: ItemCategory::Todo,
                placeholder: "Add a task...",
                initial_chips: CHIPS.iter().map(|s| s.to_string()).collect(),
                empty_text: "No tasks yet",
                done_label: "DONE",
                accent_color: "cyan",
            }
            // Extra module-specific content
            GoogleSyncPanel {}
        }
    }
}
```

`ChecklistPage` owns all state (signals), data loading (effects), and CRUD operations
internally. The page component only configures it.

---

## 3. Server Functions

Server functions are the core of Dioxus fullstack. A single function definition compiles
to two things:
- On the **server** (native): the actual implementation runs directly
- On the **client** (WASM): a generated stub that makes an HTTP POST to the server

This is explained well in Chapter 21 of *Programming Rust* (async) and the Dioxus server
function docs. The key insight: the same function signature exists on both targets, but
the body only runs on the server.

### Declaring a server function

```rust
// src/api/checklist.rs:5-6
#[server(headers: axum::http::HeaderMap)]
pub async fn list_checklist(category: ItemCategory) -> Result<Vec<ChecklistItem>, ServerFnError> {
    use crate::server::{auth, db};

    let user_id = auth::user_from_headers(&headers).map_err(|e| ServerFnError::new(e))?;
    let conn = db::pool().get().map_err(|e| ServerFnError::new(e.to_string()))?;
    // ... SQL query ...
    Ok(items)
}
```

Key details:
- **`#[server]`** marks the function. The `headers: axum::http::HeaderMap` argument tells
  Dioxus to inject the HTTP headers as a local variable named `headers`.
- The function must be **`pub async fn`** returning `Result<T, ServerFnError>`.
- All parameters (`category`) and the return type (`Vec<ChecklistItem>`) must implement
  `Serialize + Deserialize` -- they are sent over the wire as JSON.
- **Server-only imports go inside the function body** with `use`, not at the top of the
  file. This prevents WASM compilation errors for server-only crates.

### The `headers` injection pattern

The `headers` variable appears to come from nowhere:

```rust
#[server(headers: axum::http::HeaderMap)]
pub async fn toggle_checklist(id: String) -> Result<(), ServerFnError> {
    let user_id = auth::user_from_headers(&headers)?;  // `headers` is injected by the macro
    let display_name = auth::display_name_from_headers(&headers);
    // ...
}
```

The `#[server(headers: axum::http::HeaderMap)]` macro injects a local binding named
`headers` of type `axum::http::HeaderMap`. This gives you access to HTTP headers (cookies,
auth tokens, Tailscale identity headers) without manually extracting them.

### Error handling

All server function errors must be `ServerFnError`. Convert other error types with
`.map_err()`:

```rust
let conn = db::pool().get().map_err(|e| ServerFnError::new(e.to_string()))?;
```

On the client side, these errors surface as the `Err` variant when you `.await` the
server function call.

### How it works under the hood

When compiled for WASM, the `#[server]` macro replaces the function body with an HTTP
POST call to a generated endpoint (e.g., `/api/list_checklist`). The parameters are
serialized, sent as the request body, and the response is deserialized back into the
return type. This means:

1. Server functions are always async (network call on WASM)
2. Parameters and return types must be serializable
3. Server-only types (DB connections, file handles) can only be used inside the body
4. The function signature is identical on both targets -- only the implementation differs

### Calling server functions

From client code, call them like any async function:

```rust
// src/components/checklist_page.rs:39
match checklist::list_checklist(category).await {
    Ok(loaded) => items.set(loaded),
    Err(e) => error_msg.set(Some(format!("Failed to load: {e}"))),
}
```

---

## 4. Routing

### Route definition

Routes are defined as an enum with derive macros:

```rust
// src/route.rs:6-24
#[derive(Routable, Clone, Debug, PartialEq)]
#[rustfmt::skip]
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

Key concepts:
- **`#[derive(Routable)]`** generates the router logic
- **`#[route("/path")]`** maps a URL path to an enum variant
- **`#[layout(AppLayout)]`** wraps all indented routes in a shared layout component
- **`#[end_layout]`** closes the layout scope
- **`#[redirect("/", || Route::Todos {})]`** redirects `/` to the Todos page
- **`#[route("/:..segments")]`** is a catch-all for 404 pages

Each enum variant name must match a component function name. `Todos {}` expects a
`pub fn Todos() -> Element` component to exist.

### Layout component

The layout renders shared chrome (header, tab bar) and uses `Outlet` to render the
matched child route:

```rust
// src/components/layout.rs:28-52
rsx! {
    div { class: "min-h-screen bg-cyber-black",
        header { /* ... */ }
        main { class: "pt-14 pb-16 max-w-lg mx-auto",
            Outlet::<Route> {}
        }
        TabBar {}
    }
}
```

### Navigation

The `TabBar` component uses `Link` for navigation and `use_route()` to detect the
active tab:

```rust
// src/components/tab_bar.rs:8,29-30
let route: Route = use_route();
// ...
fn render_tab(target: Route, label: &str, icon: Element, current: &Route) -> Element {
    let is_active = std::mem::discriminant(&target) == std::mem::discriminant(current);
    rsx! {
        Link {
            to: target,
            class: "flex flex-col items-center gap-0.5 {color}",
            {icon}
            span { "{label}" }
        }
    }
}
```

`Link` performs client-side navigation. `use_route()` returns the current `Route` enum
variant. Comparing discriminants (ignoring field values) checks if we are on the right
tab.

---

## 5. State and Effects

Dioxus's reactivity model is similar to SolidJS signals, and the effect/signal split
maps well to the concepts in *Functional Programming in Scala* (chapter 13 on external
effects and referential transparency).

### `use_signal` -- local reactive state

```rust
// src/components/checklist_page.rs:21-27
let mut items = use_signal(Vec::<ChecklistItem>::new);
let mut input_text = use_signal(String::new);
let mut error_msg = use_signal(|| Option::<String>::None);
let mut loading = use_signal(|| false);
```

- `use_signal(initial)` creates reactive state. The argument is either a value or a
  closure returning the initial value.
- **Read** with `.read()` (returns a `Ref`) or `signal()` (returns a clone for `Copy`
  types, e.g., `loading()`).
- **Write** with `.set(value)` or `.write()` (returns a `RefMut`).
- When a signal changes, any RSX that reads it re-renders automatically.

### `use_effect` -- side effects on mount / dependency change

```rust
// src/components/checklist_page.rs:76-82
use_effect(move || {
    if let Some(cached) = cache::read::<Vec<ChecklistItem>>(cache_key) {
        items.set(cached);
    }
    reload();
    reload_chips();
});
```

`use_effect` runs its closure:
1. Once on component mount
2. Again whenever any signal it reads changes

The effect above reads no external signals, so it runs exactly once (on mount). This is
the standard pattern for initial data loading.

For re-running on signal changes:

```rust
// src/components/checklist_page.rs:85-88
use_effect(move || {
    let _trigger = sync_trigger.read().0;  // subscribes to sync_trigger
    reload();
});
```

Reading `sync_trigger` inside the effect subscribes to it. When `sync_trigger` changes,
the effect re-runs, calling `reload()`.

### `use_context` / `use_context_provider` -- global state

The provider creates the signal and makes it available to all descendants:

```rust
// src/components/layout.rs:16-17  (in AppLayout)
let sync_status = use_context_provider(|| Signal::new(SyncStatus::Syncing));
let mut sync_trigger = use_context_provider(|| Signal::new(SyncTrigger(0)));
```

Child components retrieve it with `use_context`:

```rust
// src/components/checklist_page.rs:28-29
let mut sync_status: Signal<SyncStatus> = use_context();
let sync_trigger: Signal<SyncTrigger> = use_context();
```

The type annotation is required so Dioxus knows which context to retrieve.

### The reload closure pattern

A recurring pattern in this codebase: define a `move` closure that spawns an async task
to fetch data and update signals, then call it from multiple places:

```rust
// src/components/checklist_page.rs:36-54
let reload = move || {
    spawn(async move {
        sync_status.set(SyncStatus::Syncing);
        match checklist::list_checklist(category).await {
            Ok(loaded) => {
                items.set(loaded);
                sync_status.set(SyncStatus::Synced);
            }
            Err(e) => {
                error_msg.set(Some(format!("Failed to load: {e}")));
                sync_status.set(SyncStatus::CachedOnly);
            }
        }
    });
};
```

This closure is called:
- In `use_effect` for initial load
- After successful add/toggle/delete operations
- When the sync trigger fires

The closure must be `move` and `Copy` (closures that only capture `Copy` types like
`Signal` are automatically `Copy`). This lets you pass it to render functions:

```rust
fn render_package(
    pkg: ShopeePackage,
    reload: impl Fn() + Copy + 'static,  // accepts the reload closure
    mut error_msg: Signal<Option<String>>,
) -> Element { ... }
```

### Spawning async tasks

`spawn` launches a future on the Dioxus async runtime. It does not block the current
function:

```rust
// src/components/checklist_page.rs:96-107
spawn(async move {
    match checklist::add_checklist(text, category, date).await {
        Ok(()) => {
            input_text.set(String::new());
            reload();
        }
        Err(e) => error_msg.set(Some(format!("Failed to add: {e}"))),
    }
    loading.set(false);
});
```

Use `spawn` whenever you need to call a server function from an event handler (event
handlers are synchronous, but server functions are async).

---

## 6. Touch Interactions

### The SwipeItem component

`SwipeItem` (`src/components/swipe_item.rs`) implements swipe-to-action using raw touch
events. It is a good example of stateful UI logic in Dioxus.

**State setup** (lines 14-20):

```rust
let mut translate_x = use_signal(|| 0.0_f64);
let mut start_x = use_signal(|| 0.0_f64);
let mut start_y = use_signal(|| 0.0_f64);
let mut swiping = use_signal(|| false);
let mut direction_locked = use_signal(|| false);
let mut is_horizontal = use_signal(|| false);
let mut animating = use_signal(|| false);
```

**Touch event flow:**

1. `ontouchstart` records the starting coordinates and resets state
2. `ontouchmove` calculates the delta. A 10px dead zone determines whether the gesture
   is horizontal or vertical. Vertical gestures are ignored (allows page scrolling).
   Horizontal gestures update `translate_x`.
3. `ontouchend` checks if `translate_x` exceeds the threshold (100px). If so, it fires
   the appropriate callback. Then it resets `translate_x` to 0.

**Threshold-based detection** (lines 101-107):

```rust
if tx > THRESHOLD {
    if let Some(ref handler) = on_swipe_right {
        handler.call(());
    }
} else if tx < -THRESHOLD {
    on_swipe_left.call(());
}
translate_x.set(0.0);
```

**CSS transition for snap-back** (lines 34-38):

```rust
let transition = if *animating.read() {
    "transition-transform duration-200 ease-out"
} else {
    ""
};
```

The `animating` flag is only set to `true` on `touchend`, so the snap-back animates but
the drag itself follows the finger without delay.

**Semantic swipe directions:**
- Swipe right on a pending item = complete it
- Swipe right on a completed item = add to quick-add defaults (second action)
- Swipe left = delete

---

## 7. JS Interop

Dioxus provides `document::eval()` for running JavaScript from Rust. This is essential for
browser APIs that Dioxus does not wrap natively.

### The `eval` + `recv` pattern

```rust
// src/components/shopee_ocr.rs:13-41
let js = r#"
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = 'image/*';
    input.onchange = () => {
        const file = input.files[0];
        if (!file) { dioxus.send(''); return; }
        const reader = new FileReader();
        reader.onload = () => dioxus.send(reader.result);
        reader.onerror = () => dioxus.send('');
        reader.readAsDataURL(file);
    };
    input.oncancel = () => dioxus.send('');
    input.click();
"#;

let mut eval = document::eval(js);
let base64_data = match eval.recv::<String>().await {
    Ok(s) => s,
    Err(e) => { /* handle error */ return; }
};
```

**How it works:**

1. Call `document::eval(js_code)` to execute JavaScript in the browser.
2. In JS, call `dioxus.send(value)` to send data back to Rust.
3. In Rust, call `eval.recv::<T>().await` to receive the value. `T` must implement
   `DeserializeOwned` -- the value is serialized as JSON.

**Why not Promises?** In Dioxus 0.7, you cannot return a value from `eval()` via a JS
Promise. The `eval()` call does not resolve to the Promise's result. You **must** use the
`dioxus.send()` / `eval.recv()` message-passing channel instead. This is a common
stumbling block for developers coming from web frameworks where `eval` returns a value.

### Practical example: file picker + OCR

The `ShopeeOcr` component uses this pattern to:
1. Open the browser's file picker (no Dioxus equivalent for `<input type="file">`)
2. Read the selected image as a base64 data URL via `FileReader`
3. Send the base64 string back to Rust
4. Pass it to a server function for OCR processing

This demonstrates a complete JS-to-Rust-to-Server pipeline.

---

## 8. Common Pitfalls

### Never use `base_path` in Dioxus.toml

Dioxus 0.7 has a bug where setting `base_path` in `Dioxus.toml` breaks server function
routing. Server functions are registered at fixed paths (e.g., `/api/list_checklist`),
but `base_path` prepends a prefix to all routes, causing 404s on server function calls.

**Fix:** Do not set `base_path`. Handle path prefixing at the reverse proxy level if
needed.

### Server-only dependencies must be cfg-gated

Server-only crates (SQLite, Tesseract, tokio file I/O) cannot compile to WASM. Gate
them in `Cargo.toml`:

```toml
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
rusqlite = { version = "0.31", features = ["bundled"] }
tokio = { version = "1", features = ["full"] }
```

And gate server-only modules:

```rust
// src/main.rs:7-8
#[cfg(not(target_arch = "wasm32"))]
mod server;
```

Inside server functions, import server-only crates **inside the function body**, not at
file scope:

```rust
#[server(headers: axum::http::HeaderMap)]
pub async fn list_checklist(category: ItemCategory) -> Result<Vec<ChecklistItem>, ServerFnError> {
    use crate::server::{auth, db};  // inside the function, not at the top
    // ...
}
```

### PWA files need manual copying in Dockerfile

Dioxus's build output does not include files from `assets/` that are not referenced by
`asset!()`. PWA files (`sw.js`, `sw-register.js`, `manifest.json`, icon PNGs) must be
explicitly copied into the `public/` directory in the Dockerfile.

### Signal borrowing rules

Signals use interior mutability, but you still need to respect Rust's borrowing rules:

```rust
// BAD: holding a read guard across an await point
let text = input_text.read();  // borrows the signal
some_server_fn(text.clone()).await;  // signal is still borrowed

// GOOD: clone first, then await
let text = input_text.read().clone();  // clone releases the borrow
some_server_fn(text).await;
```

This is why you see `.read().clone()` throughout the codebase. Reading a signal returns
a `Ref<T>` guard -- if you hold it across an `.await`, you get a compile error (or worse,
a runtime panic if borrowing rules are violated dynamically).

### Async closures and `spawn`

Event handlers in Dioxus are synchronous. You cannot write:

```rust
// DOES NOT WORK
onclick: async move |_| {
    let result = some_server_fn().await;
}
```

Instead, use `spawn` inside the handler:

```rust
// CORRECT
onclick: move |_| {
    spawn(async move {
        let result = some_server_fn().await;
        // update signals here
    });
},
```

### Signal `Copy` semantics

`Signal<T>` implements `Copy` regardless of `T`. This is by design -- a `Signal` is just
a handle (like an index into a signal store), not the data itself. This is why closures
capturing only signals are `Copy`, enabling patterns like:

```rust
let reload = move || {       // captures Signal values, so this closure is Copy
    spawn(async move { ... });
};

// Can pass to multiple places without cloning:
render_item(item, reload, error_msg);
render_item(item2, reload, error_msg);
```

### Model serialization requirements

All types passed to/from server functions, used in signals that cross the
client/server boundary, or stored in route parameters must derive the standard set:

```rust
// src/models/checklist_item.rs:33-34
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChecklistItem { ... }
```

`Serialize` and `Deserialize` (from serde) are non-negotiable for server function
parameters and return types. `Clone` and `PartialEq` are needed by Dioxus's reactivity
system to detect changes.

---

## Quick Reference

| Pattern | File | Line |
|---|---|---|
| App entry point | `src/main.rs` | 16-21 |
| Route definitions | `src/route.rs` | 6-24 |
| Layout with Outlet | `src/components/layout.rs` | 28-52 |
| Server function with headers | `src/api/checklist.rs` | 5-43 |
| Signal + effect data loading | `src/components/checklist_page.rs` | 21-82 |
| Reload closure pattern | `src/components/checklist_page.rs` | 36-54 |
| Context provider / consumer | `src/components/layout.rs:16-17`, `checklist_page.rs:28-29` | -- |
| Render function (not component) | `src/pages/shopee.rs` | 219-292 |
| Template component (reuse) | `src/components/checklist_page.rs` | 12-217 |
| Touch swipe handling | `src/components/swipe_item.rs` | 8-115 |
| JS interop (eval + recv) | `src/components/shopee_ocr.rs` | 13-41 |
| EventHandler callback | `src/components/shopee_ocr.rs` | 7, 49 |
| Error banner pattern | `src/components/error_banner.rs` | 1-22 |
| Optional prop with default | `src/components/quick_add.rs` | 8 |
| Tab navigation | `src/components/tab_bar.rs` | 7-45 |
