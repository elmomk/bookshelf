# 8. Developer Guide

> *"Don't Repeat Yourself."* — The Pragmatic Programmer
>
> This guide covers the practical patterns you need to extend Life Manager: adding modules, modifying the theme, and using the development tools.

## Quick Start

```bash
# Prerequisites: Rust, Node.js, dx CLI
cargo install dioxus-cli@0.7.3
rustup target add wasm32-unknown-unknown
npm install

# Development
dx serve                    # Start dev server (http://localhost:8080)
npm run tailwind            # Watch CSS changes (separate terminal)

# Or use the script
bash scripts/dev.sh
```

## Adding a New Module

Every module follows the same four-file pattern. Let's walk through adding a hypothetical "Notes" module.

### Step 1: Define the Model

Create `src/models/note.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub id: String,
    pub text: String,
    pub done: bool,
    pub created_at: f64,
    pub completed_by: Option<String>,
}
```

Register in `src/models/mod.rs`:

```rust
pub mod note;
pub use note::*;
```

### Step 2: Create the API

Create `src/api/notes.rs`:

```rust
use dioxus::prelude::*;
use crate::models::Note;

#[server(headers: axum::http::HeaderMap)]
pub async fn list_notes() -> Result<Vec<Note>, ServerFnError> {
    use crate::server::{auth, db};

    let user_id = auth::user_from_headers(&headers)
        .map_err(|e| ServerFnError::new(e))?;
    let conn = db::pool().get()
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    // Query and return
    // ...
}

#[server(headers: axum::http::HeaderMap)]
pub async fn add_note(text: String) -> Result<(), ServerFnError> {
    use crate::server::{auth, db, validate};

    let user_id = auth::user_from_headers(&headers)
        .map_err(|e| ServerFnError::new(e))?;
    validate::text(&text, "text")?;
    // Insert and return
    // ...
}
```

Register in `src/api/mod.rs`:

```rust
pub mod notes;
```

### Step 3: Add the Database Table

In `src/server/db.rs`, add to the `execute_batch` call:

```sql
CREATE TABLE IF NOT EXISTS notes (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    text TEXT NOT NULL,
    done INTEGER NOT NULL DEFAULT 0,
    created_at REAL NOT NULL,
    completed_by TEXT
);
CREATE INDEX IF NOT EXISTS idx_notes_user ON notes(user_id, done);
```

### Step 4: Create the Page

Create `src/pages/notes.rs`:

```rust
use dioxus::prelude::*;
use crate::api::notes as notes_api;
use crate::components::error_banner::ErrorBanner;
use crate::components::swipe_item::SwipeItem;
use crate::models::Note;

#[component]
pub fn Notes() -> Element {
    let mut items = use_signal(Vec::<Note>::new);
    let mut error_msg = use_signal(|| Option::<String>::None);

    let reload = move || {
        spawn(async move {
            match notes_api::list_notes().await {
                Ok(loaded) => items.set(loaded),
                Err(e) => error_msg.set(Some(format!("{e}"))),
            }
        });
    };

    use_effect(move || { reload(); });

    rsx! {
        div { class: "p-4 space-y-4",
            ErrorBanner { message: error_msg }
            // ... form and list ...
        }
    }
}
```

### Step 5: Wire Everything Up

**Route** (`src/route.rs`):
```rust
#[route("/notes")]
Notes {},
```

**Tab bar** (`src/components/tab_bar.rs`): Add an icon and link.

**Pages mod** (`src/pages/mod.rs`): Export the component.

### Step 6: Verify

```bash
cargo check          # Type check
bash scripts/deploy.sh  # Build and deploy
```

## Common Patterns

### The Reload Pattern

Never use a `refresh` counter. Call `reload()` directly after mutations:

```rust
let reload = move || {
    spawn(async move {
        match api::list().await {
            Ok(data) => items.set(data),
            Err(e) => error_msg.set(Some(format!("{e}"))),
        }
    });
};

// After add/toggle/delete:
match api::add(text).await {
    Ok(()) => reload(),  // Direct re-fetch
    Err(e) => error_msg.set(Some(format!("{e}"))),
}
```

### The Copy Closure Trick

If a closure needs to be used in multiple places (e.g., both a form submit and a chip click), all captured values must be `Copy`:

```rust
// Signals are Copy ✓
// ItemCategory is Copy (derives Copy) ✓
// &'static str is Copy ✓
// Vec<String> is NOT Copy ✗ — wrap in a Signal

let seed = use_signal(move || my_vec.clone());  // Now it's Copy
```

### Error Handling in Server Functions

Always use the `?` operator with `map_err`:

```rust
let user_id = auth::user_from_headers(&headers)
    .map_err(|e| ServerFnError::new(e))?;
let conn = db::pool().get()
    .map_err(|e| ServerFnError::new(e.to_string()))?;
```

### Cyberpunk Styling Cheat Sheet

```
Card:       bg-cyber-card/80 border border-cyber-border rounded-xl p-3
Input:      bg-cyber-dark border border-cyber-border rounded-lg px-4 py-2.5
            text-sm text-cyber-text font-mono
            focus:border-neon-{color}/60
Button:     bg-neon-{color}/20 text-neon-{color} border border-neon-{color}/40
            rounded-lg px-4 py-2.5 text-xs font-bold tracking-wider uppercase
            hover:bg-neon-{color}/30 glow-{color}
Chip:       bg-neon-cyan/10 text-neon-cyan border border-neon-cyan/30
            rounded-md px-4 py-2.5 text-xs tracking-wider uppercase
Badge:      text-[10px] bg-neon-{color}/10 text-neon-{color}
            border border-neon-{color}/30 px-2 py-0.5 rounded
Label:      text-[10px] text-cyber-dim tracking-widest uppercase
Empty:      text-xs tracking-[0.3em] uppercase text-cyber-dim
Dim text:   text-cyber-dim
Glow:       glow-cyan / glow-green / glow-orange / glow-purple / glow-pink
Text glow:  text-glow-cyan / text-glow-pink
```

## CLI Commands

| Command | Purpose |
|---------|---------|
| `/deploy` | Full build + Docker deploy + health check |
| `/build` | Tailwind + Dioxus release build |
| `/dev` | Start development server |
| `/check` | Run `cargo check` |
| `/tailwind` | Compile Tailwind CSS |
| `/db-migrate` | Add table/column to schema |
| `/add-module` | Scaffold a new module |

## Debugging Tips

### Server Logs
```bash
docker compose logs app --tail 50
docker compose logs app -f  # Follow
```

### Mobile Screenshots
```bash
bash scripts/screenshot.sh
# Output: /tmp/lm-todos.png, /tmp/lm-groceries.png, etc.
```

### Database Inspection
```bash
docker compose exec app sqlite3 /app/data/life_manager.db
sqlite> .tables
sqlite> SELECT * FROM checklist_items LIMIT 5;
sqlite> .quit
```

### Common Build Issues

| Error | Fix |
|-------|-----|
| `GLIBC_2.39 not found` | Use `debian:trixie-slim` in Dockerfile |
| `sw.js returns HTML` | Copy PWA files into `public/` in Dockerfile |
| `dioxus.send is not a function` | Your JS in `document::eval` must use `dioxus.send()`, not `return` |
| Chips not rendering | Check `reload_chips()` is called in `use_effect` |
| `FnOnce not FnMut` | Your closure captures a non-Copy value — wrap it in a `use_signal` |
