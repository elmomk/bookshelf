# BookClub PWA

## Project Overview
Mobile-first PWA for a shared reading club: a single shared shelf of books where each
member tracks their own reading progress on the same book and discusses it with
spoiler-aware comments. Converted from the Life Manager codebase.

**Stack:** Rust + Dioxus 0.7 (fullstack, Wasm), Tailwind CSS v4, SQLite (server).

## Architecture
- `src/models/` — Data structs shared between client and server (`book.rs`, `notification.rs`)
- `src/pages/` — Dioxus page components: `books.rs` (shelf), `book_detail.rs`, `activity.rs`
- `src/components/` — Reusable UI (swipe, tab bar, icons, error banner, progress bar, sync indicator, notification bell)
- `src/api/` — Server functions (`#[server]`): `books.rs` (shelf/progress/comments/search/activity), `notifications.rs`
- `src/server/` — Server-only code (SQLite DB, auth, validate, notify/web-push) — gated behind `#[cfg(not(target_arch = "wasm32"))]`
- `src/route.rs` — Dioxus Router config (`/books`, `/book/:id`, `/activity`)
- `assets/` — Static assets (CSS, manifest, icons, SW)
- `scripts/` — Shell scripts for common tasks

## Build & Dev Commands
- **Dev server:** `./scripts/dev.sh` (Tailwind watch + Dioxus dev server on port 8080)
- **Production build:** `./scripts/build.sh` (Tailwind + Dioxus release build → `target/dx/bookclub/release/web/`)
- **Type check:** `./scripts/check.sh` (cargo check)
- **Deploy:** `./scripts/deploy.sh` (build + Docker + deploy + health check). Do NOT use the `/deploy` skill — it targets the unrelated `garmin_api` project.
- **Screenshots:** `./scripts/screenshot.sh` (Playwright mobile screenshots)

## Deploy Safety (MANDATORY)
- **Always back up the SQLite DB before any deploy.** `./scripts/deploy.sh` does this automatically (copies `bookclub.db` + WAL/SHM from the running container into `backups/`, keeps last 20, aborts the deploy if the backup fails while the app is running). Never deploy in a way that bypasses this.

## Git Workflow (MANDATORY)
- After **every** change set, always `git commit` AND create an annotated tag — without being asked.
- Commit directly on `main` (this repo uses a direct-to-main release flow; do **not** branch).
- Tags are `vX.Y.Z`; bump the **patch** from the latest tag (e.g. `v0.1.15` → `v0.1.16`), annotated (`git tag -a`).
- Commit message: concise summary line + optional `-` bullet body, matching existing history; end with the `Co-Authored-By` trailer.
- Do **not** `git push` unless explicitly asked.

## Deployment
- Docker Compose with Tailscale sidecar container (`hostname: bookclub`)
- Dockerfile copies the locally-built `bookclub` binary (no Rust build in Docker — `debian:trixie-slim`)
- App at `https://bookclub.tail6c1af7.ts.net/`
- SQLite DB in Docker volume at `/app/data/bookclub.db`
- `DATABASE_PATH` env var configures DB location (defaults to `bookclub.db`)
- `GOOGLE_BOOKS_API_KEY` is **effectively required** as of 2026-05 — Google now caps the keyless anonymous Books API quota at 0/day (HTTP 429), so without a key book search and the Google-Books ToC fallback both silently return no results. Get a key from a Google Cloud project with the Books API enabled, then put it in the container env. Open Library calls (ToC by ISBN) are unaffected and stay keyless.
- `VAPID_PUBLIC_KEY` / `VAPID_PRIVATE_KEY` enable web-push notifications

## Data Model
- `books` — shared shelf (one row per book; `google_books_id` de-duped)
- `reading_progress` — one row per (book, reader); `UNIQUE(book_id, reader)`; status `to_read|reading|finished`
- `book_comments` — discussion; optional `page`/`chapter` anchor for spoiler gating
- `notifications` / `notification_settings` / `notification_reads` / `push_subscriptions` — kept from base

## Code Conventions
- Dioxus 0.7 RSX syntax (no `cx` parameter, `rsx!{}` macro with `Element` return)
- Server functions use `#[server(headers: axum::http::HeaderMap)]` — `headers` injected by macro
- Auth: `auth::user_from_headers(&headers)` → shared `"default"` user (requires Tailscale header when `REQUIRE_AUTH=true`)
- Attribution: `auth::display_name_from_headers(&headers)` is the per-reader identity (progress, comments, activity)
- Spoiler gating is **server-side**: `list_comments` blanks comment bodies whose page/chapter anchor is past the requesting reader's progress (`hidden = true`)
- Models derive `Clone, Debug, PartialEq, Serialize, Deserialize`
- Tailwind CSS v4 (no tailwind.config.js — `@import`/`@theme` in `input.css`)
- Cyberpunk design language: neon accents, dark backgrounds, JetBrains Mono, glow, scanline overlay
- Theme colors: `cyber-black/dark/card/border`, `neon-cyan/green/magenta/orange/purple/pink/yellow`
- Data loading: `use_signal` + `use_effect` with explicit `reload()` closure; re-syncs on the header `SyncTrigger`
- Error feedback: `ErrorBanner` with dismissible `error_msg` signal
- Shelf swipe: right = advance my status (To Read → Reading → Finished), left = remove book

## Snapshots
- `src/server/snapshots.rs` manages files under `/app/data/snapshots/` (in the Docker volume) via SQLite's `VACUUM INTO` — atomic, online, WAL-safe.
- **Daily auto-snapshot:** a tokio task spawned from `main.rs` takes one snapshot per 24h (catches up immediately on startup if the newest is older than 24h). Auto and manual snapshots share the `snap-<ms>.db` naming and appear together in Settings → History.
- Manual snapshots come from Settings → History "📸 Take snapshot now" or are auto-created as a safety net before every restore (`restore_full_from_snapshot`, `restore_book_from_snapshot`, `undo_change`, `restore_to_before_tx`).
- No automatic pruning — the user deletes snapshots from the UI. If the volume fills up, that's the manual lever.

## Change log invariants
- Every write to `books`, `reading_progress`, `book_comments`, `comment_reactions`, `reader_aliases`, `notification_settings` MUST go through `crate::server::changelog::ChangeRecorder` inside a `transaction_with_behavior(Immediate)` — this is what powers Settings → Change log (undo + restore-to-before).
- **Any new column added to a logged table must be NULLable or have a DEFAULT.** Old `db_changes` rows captured pre-column won't have the field; inverse-replay binds NULL for missing keys, which must satisfy the schema. Don't add NOT NULL without a default to a logged table.
- The column lists in `src/server/changelog.rs::data_cols_of` are the source of truth for which columns the recorder captures. **If you add a column to a logged table, add it to that list too** — otherwise it's silently invisible to undo/restore.
- `db_changes` is capped at 50,000 rows by `changelog::prune_oldest`, called from `db::init`. Snapshots remain the long-term archive; the change log is fine-grained undo on a healthy DB.
- Notifications, notification_reads, push_subscriptions are intentionally NOT logged (ephemeral runtime state).

## Important Notes
- **Never use `base_path`** in Dioxus.toml — breaks fullstack server function calls (Dioxus 0.7 bug)
- PWA files (`sw.js`, `sw-register.js`, `manifest.json`, `icons/`, `fonts/`) are copied into `public/` via Dockerfile (Dioxus doesn't emit them)
- `target/`, `node_modules/`, `.env` are not committed
- Server-only deps gated with `cfg(not(target_arch = "wasm32"))`
- Dioxus fullstack means both `web` and `server` features are default-enabled
- JS-to-Rust in Dioxus 0.7: `dioxus.send()` in JS + `eval.recv::<T>().await` in Rust (not Promise return)
- Crate/binary name is `bookclub` (Cargo.toml + Dioxus.toml); build output path is `target/dx/bookclub/...`

## Skills & Commands
- `/deploy` — Build and deploy via Docker Compose
- `/build` — Build for production
- `/dev` — Start dev environment
- `/check` — Run type checking
- `/tailwind` — Compile Tailwind CSS
- `/db-migrate` — Add tables/columns to SQLite schema
