---
name: add-module
description: Scaffold a new Life Manager module (model, API, page, route)
allowed-tools: Read, Write, Edit, Bash, Grep, Glob
---

Scaffold a new module for the Life Manager app. Pass the module name as an argument: `/add-module <name>`

## Architecture

Each module consists of 4 files:
1. **Model** (`src/models/<name>.rs`) — Data struct with `Clone, Debug, PartialEq, Serialize, Deserialize`
2. **API** (`src/api/<name>.rs`) — Server functions: `list_<name>`, `add_<name>`, `toggle_<name>` (if applicable), `delete_<name>`
3. **Page** (`src/pages/<name>.rs`) — Dioxus component using `SwipeItem` for list items
4. **DB table** in `src/server/db.rs` — `CREATE TABLE IF NOT EXISTS`

## Steps

1. Read existing modules for patterns:
   - `src/models/checklist_item.rs` for model pattern
   - `src/api/checklist.rs` for server function pattern (uses `#[server(headers: axum::http::HeaderMap)]`)
   - `src/components/checklist_page.rs` for reusable checklist pattern
   - `src/server/db.rs` for schema pattern

2. Create all 4 files following the exact same patterns

3. Register the module:
   - Add `pub mod <name>;` to `src/models/mod.rs` and `pub use <name>::*;`
   - Add `pub mod <name>;` to `src/api/mod.rs`
   - Add `pub mod <name>;` to `src/pages/mod.rs`
   - Add route variant to `src/route.rs`
   - Add tab icon to `src/components/tab_bar.rs`

4. Add table to `src/server/db.rs` `execute_batch`

5. Run `./scripts/check.sh` to verify

## Key patterns
- Server functions: `#[server(headers: axum::http::HeaderMap)]` with `auth::user_from_headers(&headers).map_err(|e| ServerFnError::new(e))?`
- Auth also provides `auth::display_name_from_headers(&headers)` for tracking who completed items
- Page data loading: `use_signal` + `use_effect` with explicit `reload()` closure (no refresh counter)
- Swipe right = toggle done, swipe left = delete
- Error feedback via `ErrorBanner` component with `error_msg` signal
- UUIDs and timestamps generated server-side
- Cyberpunk theme: `bg-cyber-card`, `border-cyber-border`, `text-neon-*`, `glow-*` classes
