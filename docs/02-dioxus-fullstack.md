# 2. The Dioxus Fullstack Model

> *"Ownership is Rust's most unique feature, and it enables Rust to make memory safety guarantees without needing a garbage collector."* — The Rust Programming Language
>
> In Dioxus, ownership isn't just about memory — it's about who owns the state, who can read it, and when re-renders happen.

## How Dioxus 0.7 Works

Dioxus is a React-like framework for Rust. Components are functions that return `Element`. State is managed through **signals** — reactive containers that trigger re-renders when their value changes.

```rust
#[component]
fn Counter() -> Element {
    let mut count = use_signal(|| 0);

    rsx! {
        button {
            onclick: move |_| count += 1,
            "Count: {count}"
        }
    }
}
```

When `count` changes, Dioxus diffs the virtual DOM and patches only the affected text node. The developer never manually manipulates the DOM.

## Server Functions: The RPC Bridge

The `#[server]` macro is Dioxus's killer feature. It lets you write a function that runs on the server but can be called from the client as if it were a local async function.

```rust
// This compiles to TWO things:
// 1. Server: the actual function body
// 2. Client: an async stub that does HTTP POST

#[server(headers: axum::http::HeaderMap)]
pub async fn add_checklist(
    text: String,
    category: ItemCategory,
    date: Option<String>,
) -> Result<(), ServerFnError> {
    // This code ONLY runs on the server
    use crate::server::{auth, db, validate};

    let user_id = auth::user_from_headers(&headers)?;
    validate::text(&text, "text")?;
    let conn = db::pool().get()?;
    // ... SQL INSERT ...
    Ok(())
}
```

The `headers: axum::http::HeaderMap` annotation tells Dioxus to inject the HTTP headers into a variable called `headers`. This is how we access the Tailscale authentication header without explicitly passing it from the client.

### What the Client Sees

On the WASM side, `add_checklist` becomes:

```rust
// Auto-generated client stub (conceptual)
pub async fn add_checklist(
    text: String,
    category: ItemCategory,
    date: Option<String>,
) -> Result<(), ServerFnError> {
    let args = serialize(text, category, date);
    let response = http_post("/_server_fn/add_checklist_HASH", args).await?;
    deserialize(response)
}
```

The function signature is identical. The call site doesn't know or care whether it's running on the client or server. This is why `models/` types must derive both `Serialize` and `Deserialize` — they cross the network boundary.

## Signals: Reactive State

Dioxus signals are the core state primitive. They are:

- **Copy**: `Signal<T>` implements `Copy`, so closures can capture them without lifetime issues
- **Reactive**: reading a signal inside a component subscribes that component to changes
- **Thread-local**: signals live on the UI thread (or WASM main thread)

### The Reload Pattern

Every page in Life Manager follows the same state management pattern:

```rust
let mut items = use_signal(Vec::<Item>::new);
let mut error_msg = use_signal(|| Option::<String>::None);

// Define a reload closure — all captured values are Copy
let reload = move || {
    spawn(async move {
        match api::list_items().await {
            Ok(data) => items.set(data),
            Err(e) => error_msg.set(Some(format!("{e}"))),
        }
    });
};

// Load on mount
use_effect(move || { reload(); });

// After mutations, call reload() directly
spawn(async move {
    match api::add_item(text).await {
        Ok(()) => reload(),  // Re-fetch from server
        Err(e) => error_msg.set(Some(format!("{e}"))),
    }
});
```

This pattern avoids optimistic updates. After every mutation, we re-fetch the full list from the server. This is simpler than maintaining client-side state that could diverge from the database, and the latency is acceptable for a personal app on a local network.

### Why Closures Are Tricky

Rust closures capture their environment by move or by reference. In Dioxus, event handlers must be `'static` — they can't borrow from the component's stack frame. This is why we use `move` closures everywhere.

The key insight: **`Signal<T>` is `Copy`**, so a `move` closure that only captures signals doesn't consume them — it copies them. This means the same signal can be used in multiple closures without `.clone()`.

But `Vec<String>` is NOT `Copy`. If a closure captures a `Vec`, it moves it, and no other closure can use it. This is why we store fallback chip lists in a `use_signal`:

```rust
// BAD: initial_chips is Vec<String>, moved into first closure
let reload_chips = move || {
    let fb = initial_chips.clone(); // ERROR: initial_chips was moved
    // ...
};

// GOOD: store in a signal (which is Copy)
let seed_chips = use_signal(move || initial_chips.clone());
let reload_chips = move || {
    let fb = seed_chips.read().clone(); // OK: seed_chips is Copy
    // ...
};
```

## The RSX Macro

Dioxus uses an `rsx!` macro that looks like a hybrid of HTML and Rust:

```rust
rsx! {
    div { class: "flex items-center gap-3",
        p { class: "text-sm font-medium", "{item.text}" }
        if item.done {
            span { class: "text-neon-green", "DONE" }
        }
        for tag in &item.tags {
            span { class: "text-xs", "{tag}" }
        }
    }
}
```

Key differences from React JSX:
- **No closing tags** — blocks are delimited by braces
- **Attributes use colons** — `class: "..."` not `className="..."`
- **Rust control flow** — `if`, `for`, `match` work directly in RSX
- **String interpolation** — `"{variable}"` in text nodes
- **Event handlers** — `onclick: move |e| { ... }` with Rust closures
- **Auto-escaping** — all interpolated values are escaped, preventing XSS

## JavaScript Interop

When Rust can't do it alone (file picker, clipboard, timers), Dioxus provides `document::eval()`:

```rust
let mut eval = document::eval(r#"
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = 'image/*';
    input.onchange = () => {
        const file = input.files[0];
        const reader = new FileReader();
        reader.onload = () => dioxus.send(reader.result);
        reader.readAsDataURL(file);
    };
    input.click();
"#);

// Receive the result in Rust
let base64_data = eval.recv::<String>().await?;
```

The critical pattern: **use `dioxus.send(value)` to communicate from JS to Rust**, not Promise return values. The `.recv::<T>()` method on the Rust side deserializes the sent value.
