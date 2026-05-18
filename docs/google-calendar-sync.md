# Google Calendar Sync

This document explains how Life Manager synchronizes checklist items and Shopee packages with Google Calendar. It covers the authentication mechanism, the Calendar Events API usage, the sync architecture, every integration hook, setup instructions, the UI component, and failure modes.

---

## Table of Contents

1. [Service Account Authentication](#1-service-account-authentication)
2. [Calendar Events API](#2-calendar-events-api)
3. [Sync Architecture](#3-sync-architecture)
4. [Integration Points](#4-integration-points)
5. [Setup Guide](#5-setup-guide)
6. [UI Component](#6-ui-component)
7. [Failure Modes and Recovery](#7-failure-modes-and-recovery)

---

## 1. Service Account Authentication

**Reference:** Justin Richer, *OAuth 2.0 in Action* (Manning, 2017), Chapter 9 -- Client Credentials and Assertion Grants.

### Why a Service Account?

A Google Service Account (SA) is a machine identity that acts on its own behalf rather than on behalf of a human user. Life Manager uses a Service Account instead of the standard three-legged OAuth 2.0 flow for several reasons:

- **No user consent screen.** The app runs as a self-hosted PWA behind Tailscale. There is no browser-based OAuth redirect to handle on the client, and no refresh tokens to store per user.
- **Server-to-server communication.** Calendar sync happens inside `tokio::spawn` tasks on the server. There is no browser involved at the point of sync.
- **Simpler credential management.** A single JSON key file is mounted into the Docker container. No token database, no per-user credential storage.

The tradeoff is that the SA operates on calendars that have been explicitly shared with it (via the SA's email address), rather than accessing an arbitrary user's calendar through consent.

### The JWT Bearer Flow

The authentication mechanism is the **JWT Bearer assertion grant** defined in [RFC 7523](https://datatracker.ietf.org/doc/html/rfc7523). The flow works as follows:

1. **Load the SA key file.** The JSON key file (downloaded from Google Cloud Console) contains `client_email` and `private_key` fields. The file path comes from the `GOOGLE_SA_KEY_FILE` environment variable:

```rust
fn load_sa_key() -> Result<SaKey, String> {
    let path = std::env::var("GOOGLE_SA_KEY_FILE")
        .map_err(|_| "GOOGLE_SA_KEY_FILE not set".to_string())?;
    let data = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read SA key file: {e}"))?;
    serde_json::from_str(&data)
        .map_err(|e| format!("Failed to parse SA key file: {e}"))
}
```

2. **Construct JWT claims.** The claims object contains five fields:

| Claim   | Value                                                       | Purpose                              |
|---------|-------------------------------------------------------------|--------------------------------------|
| `iss`   | `client_email` from the key file                            | Identifies the service account       |
| `scope` | `https://www.googleapis.com/auth/calendar.events`           | Requests calendar event access       |
| `aud`   | `https://oauth2.googleapis.com/token`                       | The token endpoint that will consume the JWT |
| `iat`   | Current Unix timestamp                                      | Issued-at time                       |
| `exp`   | `iat + 3600`                                                | Token expires in 60 minutes          |

```rust
let claims = serde_json::json!({
    "iss": sa.client_email,
    "scope": "https://www.googleapis.com/auth/calendar.events",
    "aud": "https://oauth2.googleapis.com/token",
    "iat": now,
    "exp": now + 3600,
});
```

3. **Sign with RS256.** The JWT is signed using the RSA private key from the key file with the RS256 algorithm (RSASSA-PKCS1-v1_5 using SHA-256). The `jsonwebtoken` crate handles encoding:

```rust
let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
let key = jsonwebtoken::EncodingKey::from_rsa_pem(sa.private_key.as_bytes())?;
let jwt = jsonwebtoken::encode(&header, &claims, &key)?;
```

4. **Exchange for an access token.** The signed JWT is sent as a `POST` form body to Google's token endpoint:

```rust
let resp = client
    .post("https://oauth2.googleapis.com/token")
    .form(&[
        ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
        ("assertion", &jwt),
    ])
    .send()
    .await?;
```

Google validates the JWT signature against the SA's registered public key, checks the claims, and returns an `access_token` valid for 60 minutes.

### Token Caching Strategy

Tokens are cached in a `static Mutex<Option<(String, u64)>>` -- a tuple of the token string and its expiry timestamp:

```rust
static TOKEN_CACHE: Mutex<Option<(String, u64)>> = Mutex::new(None);
```

The cache expiry is set to **50 minutes** (`now + 3000` seconds) even though Google issues tokens valid for 60 minutes (`now + 3600`). This 10-minute buffer ensures that a cached token will never be used in the final minutes before expiry, when it might expire mid-request. This is a standard defensive pattern -- if a sync operation takes a few seconds, and the token was about to expire, the request would fail with a 401 partway through.

```rust
// Cache for 50 minutes (token valid for 60)
let mut cache = TOKEN_CACHE.lock().unwrap();
*cache = Some((token.clone(), now + 3000));
```

On cache hit (token exists and `now < exp`), the cached token is returned immediately without any network call.

---

## 2. Calendar Events API

**Reference:** [Google Calendar API v3 -- Events](https://developers.google.com/calendar/api/v3/reference/events).

### All-Day Events

Life Manager creates **all-day events** rather than timed events. The Calendar API distinguishes these by using the `date` field (a `YYYY-MM-DD` string) instead of `dateTime` (an RFC 3339 timestamp).

For all-day events, the `end.date` must be the day *after* the event. A single-day event on March 22 has `start.date = "2026-03-22"` and `end.date = "2026-03-23"`. The code computes this with `chrono::NaiveDate::succ_opt()`:

```rust
let end_date = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")?
    .succ_opt()
    .ok_or("Date overflow")?
    .format("%Y-%m-%d")
    .to_string();

let body = serde_json::json!({
    "summary": title,
    "start": { "date": date },
    "end": { "date": end_date },
});
```

### Extended Properties for Idempotent Sync

Each calendar event stores a `life_manager_id` in its private extended properties. This is the UUID of the checklist item or Shopee package that created it:

```rust
"extendedProperties": {
    "private": {
        "life_manager_id": item_id
    }
}
```

This serves two purposes:

1. **Idempotent creation.** Before creating a duplicate, `find_event_by_item_id` can search for an existing event with that ID.
2. **Orphan cleanup.** During full sync, the system can find and delete events whose corresponding items have been completed or deleted.

### CRUD Operations

| Operation | HTTP Method | Endpoint                                          | When Used                        |
|-----------|-------------|---------------------------------------------------|----------------------------------|
| Create    | `POST`      | `/calendars/{calendarId}/events`                  | New item with a date             |
| Update    | `PATCH`     | `/calendars/{calendarId}/events/{eventId}`         | Item title or date changed       |
| Delete    | `DELETE`    | `/calendars/{calendarId}/events/{eventId}`         | Item completed or deleted        |
| Find      | `GET`       | `/calendars/{calendarId}/events?privateExtendedProperty=life_manager_id%3D{id}` | Locating event by item ID |

The `create_event` function returns the Google-assigned `event_id`, which is stored in the `google_event_id` column in SQLite for future updates and deletes. The `find_event_by_item_id` function uses the `privateExtendedProperty` query parameter to locate events by their Life Manager ID -- this is the fallback when the local `google_event_id` column is NULL (e.g., after a database restore or if the initial store failed).

### URL Encoding for Calendar IDs

Calendar IDs can contain characters that are not URL-safe (e.g., `@` and `.` in email-style IDs like `abc123@group.calendar.google.com`). The code URL-encodes them:

```rust
fn encode_calendar_id(id: &str) -> String {
    url::form_urlencoded::byte_serialize(id.as_bytes()).collect()
}
```

---

## 3. Sync Architecture

**Reference:** Sam Newman, *Building Microservices* (O'Reilly, 2nd edition, 2021), Chapter 6 -- on eventual consistency and the tradeoff between synchronous coordination and availability.

### Fire-and-Forget Pattern

Every mutation hook (add, toggle, delete) triggers calendar sync via `tokio::spawn`. The spawned task runs independently of the HTTP response:

```rust
tokio::spawn(async move {
    crate::server::google::sync_item(&id2, &text2, Some(&d2), false, None).await;
});
```

This is a deliberate architectural choice:

- **User operations never block on Calendar API latency.** A Google API call takes 200-500ms. The user's add/toggle/delete completes in ~5ms (SQLite only).
- **Calendar sync failures do not fail user operations.** If Google is down, the todo still gets added. The calendar just lags.
- **Eventual consistency is acceptable.** Calendar events are a convenience view of Life Manager data, not the source of truth. If sync falls behind, the manual re-sync button catches everything up.

This aligns with Newman's guidance: when the downstream system (Google Calendar) is not the system of record, prefer asynchronous fire-and-forget over synchronous coordination. The cost is temporary inconsistency; the benefit is that the primary system never degrades due to a secondary integration.

### Logging Without Propagation

The `sync_item` function wraps its entire body in an async block that returns `Result<(), String>`. If anything fails, it logs a warning and moves on:

```rust
let result: Result<(), String> = async {
    // ... all sync logic ...
    Ok(())
}.await;

if let Err(e) = result {
    tracing::warn!("Google Calendar sync failed for item {item_id}: {e}");
}
```

There is no retry, no queue, no dead letter. The rationale is simplicity: the full re-sync mechanism (Section 4) serves as the catch-all recovery. Adding retry logic would increase complexity without proportional benefit, since the user can trigger a re-sync at any time.

### Table-Agnostic Design

The `sync_item` function does not know whether it is syncing a checklist item or a Shopee package. It tries both tables:

```rust
// Try checklist_items first
let affected = conn.execute(
    "UPDATE checklist_items SET google_event_id = ?2 WHERE id = ?1",
    rusqlite::params![item_id, event_id],
)?;
// If no rows matched, try shopee_packages
if affected == 0 {
    conn.execute(
        "UPDATE shopee_packages SET google_event_id = ?2 WHERE id = ?1",
        rusqlite::params![item_id, event_id],
    )?;
}
```

This works because UUIDs are globally unique across tables. The same pattern applies when clearing `google_event_id` on completion -- both tables are attempted, and whichever has a matching row gets updated.

### The `google_event_id` Column

Both `checklist_items` and `shopee_packages` have a nullable `google_event_id TEXT` column, added via migration:

```sql
ALTER TABLE checklist_items ADD COLUMN google_event_id TEXT;
ALTER TABLE shopee_packages ADD COLUMN google_event_id TEXT;
```

This column stores the Google Calendar event ID (a string like `"abc123def456"`). Its lifecycle:

- **NULL** when the item has no associated calendar event (no date, or sync hasn't run yet).
- **Set** after `create_event` returns a new event ID.
- **Cleared to NULL** after `delete_event` succeeds (item completed or deleted).

---

## 4. Integration Points

### `add_checklist` / `add_shopee` -- Create Event on Add

When a new item is added with a date, a calendar event is created:

```rust
// In add_checklist:
if let Some(ref d) = date {
    let id2 = id.clone();
    let text2 = text.clone();
    let d2 = d.clone();
    tokio::spawn(async move {
        crate::server::google::sync_item(&id2, &text2, Some(&d2), false, None).await;
    });
}
```

The `sync_item` call receives `done: false` and `google_event_id: None`, so it takes the "create new event" branch. The same pattern applies in `add_shopee`, using `due_date` and `title` instead.

Items without dates are not synced -- there is no calendar event to create for an undated task.

### `toggle_checklist` / `toggle_shopee` -- Create or Delete on Toggle

Toggle is the most complex hook because the item's state has already changed in the database by the time the spawned task runs. The task re-reads the current state:

```rust
tokio::spawn(async move {
    let conn = crate::server::db::pool().get().ok();
    if let Some(conn) = conn {
        let item: Option<(String, Option<String>, bool, Option<String>)> = conn
            .query_row(
                "SELECT text, date, done, google_event_id FROM checklist_items WHERE id = ?1",
                rusqlite::params![id2],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .ok();
        if let Some((text, date, done, event_id)) = item {
            crate::server::google::sync_item(
                &id2, &text, date.as_deref(), done, event_id.as_deref(),
            ).await;
        }
    }
});
```

The re-read is necessary because `toggle_checklist` uses `SET done = 1 - done` -- the function does not know the new value without querying. Inside `sync_item`:

- If `done == true`: delete the calendar event (if one exists).
- If `done == false` and the item has a date: create or update the calendar event.

For Shopee packages, `toggle_shopee` follows the identical pattern, reading `title, due_date, picked_up, google_event_id` from `shopee_packages`.

### `delete_checklist` / `delete_shopee` -- Read Before Delete

Deletion has a specific ordering constraint: the `google_event_id` must be read *before* the row is deleted from SQLite, because after deletion there is nothing to read:

```rust
// Read google_event_id before deleting
let event_id: Option<String> = conn
    .query_row(
        "SELECT google_event_id FROM checklist_items WHERE id = ?1 AND user_id = ?2",
        rusqlite::params![id, user_id],
        |row| row.get(0),
    )
    .ok()
    .flatten();

// Delete the row
conn.execute(
    "DELETE FROM checklist_items WHERE id = ?1 AND user_id = ?2",
    rusqlite::params![id, user_id],
)?;

// Fire-and-forget: delete Calendar event
if let Some(eid) = event_id {
    let eid2 = eid.clone();
    tokio::spawn(async move {
        crate::server::google::sync_item("", "", None, true, Some(&eid2)).await;
    });
}
```

Note the `sync_item` call passes empty strings for `item_id` and `title` and `None` for `date` -- only `done: true` and the event ID matter for the delete path.

If `google_event_id` was NULL (item never synced or had no date), no spawn happens.

### Full Sync -- Catch-All Recovery

The `google_full_sync` server function iterates all relevant items and reconciles calendar state. It handles four categories:

1. **Undone checklist items with dates** -- ensures each has a calendar event.
2. **Done checklist items with stale `google_event_id`** -- deletes the orphaned event and clears the column.
3. **Undone Shopee packages with `due_date`** -- ensures each has a calendar event.
4. **Picked-up Shopee packages with stale `google_event_id`** -- deletes the orphaned event and clears the column.

For each item, it calls `sync_item` with the current state, then verifies the `google_event_id` column was populated (for undone items) or cleared (for done items). The function returns a summary string:

```
"Synced 12/15 tasks, 3/3 packages (3 errors)"
```

This is exposed via the `GoogleSyncPanel` UI component (Section 6).

---

## 5. Setup Guide

### Step 1: Create or Reuse a Google Service Account

1. Go to [Google Cloud Console > IAM & Admin > Service Accounts](https://console.cloud.google.com/iam-admin/serviceaccounts).
2. Select your project (or create one).
3. Click **Create Service Account**. Give it a name like `life-manager-calendar`.
4. Skip the optional role grants (it does not need project-level roles).
5. On the service account page, go to **Keys > Add Key > Create new key > JSON**.
6. Download the JSON key file. It looks like:

```json
{
  "type": "service_account",
  "project_id": "your-project",
  "private_key_id": "abc123",
  "private_key": "-----BEGIN RSA PRIVATE KEY-----\n...\n-----END RSA PRIVATE KEY-----\n",
  "client_email": "life-manager-calendar@your-project.iam.gserviceaccount.com",
  "client_id": "123456789",
  "auth_uri": "https://accounts.google.com/o/oauth2/auth",
  "token_uri": "https://oauth2.googleapis.com/token",
  ...
}
```

Only the `client_email` and `private_key` fields are used by Life Manager.

### Step 2: Share a Calendar with the Service Account

1. Open [Google Calendar](https://calendar.google.com) in a browser.
2. Under **Other calendars**, click **+** > **Create new calendar** (or use an existing one).
3. Open the calendar's **Settings and sharing**.
4. Under **Share with specific people or groups**, add the SA email (e.g., `life-manager-calendar@your-project.iam.gserviceaccount.com`).
5. Set permission to **Make changes to events** (editor access).
6. Copy the **Calendar ID** from the **Integrate calendar** section. It looks like `abc123@group.calendar.google.com` or `primary` for your default calendar.

### Step 3: Configure Environment Variables

Two environment variables control the integration:

| Variable               | Required | Default     | Description                          |
|------------------------|----------|-------------|--------------------------------------|
| `GOOGLE_SA_KEY_FILE`   | Yes      | *(none)*    | Absolute path to the SA JSON key file |
| `GOOGLE_CALENDAR_ID`   | No       | `primary`   | Calendar ID to sync events to        |

If `GOOGLE_SA_KEY_FILE` is not set, the entire sync feature is a no-op -- `is_configured()` returns `false` and all sync calls return immediately.

### Step 4: Docker Volume Mount

In `docker-compose.yml`, the key file is mounted read-only into the container:

```yaml
app:
  environment:
    - GOOGLE_SA_KEY_FILE=/app/gorilla-coach-sheets-key.json
    - GOOGLE_CALENDAR_ID=${GOOGLE_CALENDAR_ID:-primary}
  volumes:
    - ./gorilla-coach-sheets-key.json:/app/gorilla-coach-sheets-key.json:ro
```

Place the downloaded JSON key file next to `docker-compose.yml` with the expected filename. The `:ro` flag ensures the container cannot modify the key file.

The `GOOGLE_CALENDAR_ID` defaults to `primary` if not set in the `.env` file. To target a specific calendar, add to your `.env`:

```
GOOGLE_CALENDAR_ID=abc123def456@group.calendar.google.com
```

### Step 5: Verify Sync Works

1. Deploy or restart the app.
2. Navigate to the **Todos** tab.
3. Look at the **Google Calendar** panel at the bottom of the page:
   - A **green dot** means `GOOGLE_SA_KEY_FILE` is set and the file loaded.
   - A **dim dot** means the env var is missing.
4. Add a todo with a date. Check your Google Calendar -- an all-day event should appear within a few seconds.
5. Click the **RE-SYNC** button to run a full sync. The result message shows counts like `"Synced 5/5 tasks, 2/2 packages (0 errors)"`.

---

## 6. UI Component

The `GoogleSyncPanel` component (`src/components/google_sync.rs`) provides status visibility and manual re-sync. It is embedded at the bottom of the Todos page.

### Status Detection

On mount, the component calls the `google_calendar_status` server function, which simply returns `google::is_configured()` -- a check for whether the `GOOGLE_SA_KEY_FILE` env var is set:

```rust
let mut status = use_signal(|| Option::<bool>::None);

use_effect(move || {
    spawn(async move {
        if let Ok(configured) = google_api::google_calendar_status().await {
            status.set(Some(configured));
        }
    });
});
```

The result drives the indicator dot:

- **Green dot** (`bg-neon-green`): configured and ready.
- **Dim dot** (`bg-cyber-dim/40`): not configured.

When not configured, a hint message is shown: *"Set GOOGLE_SA_KEY_FILE + GOOGLE_CALENDAR_ID to enable"*.

### Manual Re-Sync Button

When configured, a **RE-SYNC** button appears. It triggers `google_full_sync()` and shows a loading state:

```rust
button {
    disabled: *syncing.read(),
    onclick: move |_| {
        syncing.set(true);
        sync_result.set(None);
        spawn(async move {
            match google_api::google_full_sync().await {
                Ok(msg) => sync_result.set(Some(msg)),
                Err(e) => sync_result.set(Some(format!("Error: {e}"))),
            }
            syncing.set(false);
        });
    },
    if *syncing.read() { "SYNCING..." } else { "RE-SYNC" }
}
```

The button is disabled during sync to prevent double-clicks. Results are displayed below the button in neon green monospace text.

---

## 7. Failure Modes and Recovery

**Reference:** Michael T. Nygard, *Release It!* (Pragmatic Bookshelf, 2nd edition, 2018), Chapters 4-5 on Stability Patterns and Antipatterns.

### Token Expiry and Automatic Refresh

Tokens are cached for 50 minutes but valid for 60. If a token expires between the cache check and the API call (an unlikely but possible race), Google returns a `401`. The current implementation does not retry on `401` -- the sync fails and is logged. The next sync call will fetch a fresh token because the cached one will have passed the 50-minute mark.

This is an acceptable failure mode because:
- The 10-minute buffer makes it extremely rare.
- The next mutation or manual re-sync will succeed.

### Network Errors

All `reqwest` calls use `.map_err()` to convert network failures into `String` errors. These propagate up to the `sync_item` wrapper, which logs them:

```
WARN Google Calendar sync failed for item abc-123: Token request failed: connection refused
```

Following Nygard's "bulkhead" pattern, the Calendar integration is isolated from the main request path. A network partition to `googleapis.com` has zero impact on Life Manager's core functionality.

### 404/410 on Delete

When deleting a calendar event, the response status is checked with tolerance for "already gone" states:

```rust
if !resp.status().is_success()
    && resp.status().as_u16() != 404
    && resp.status().as_u16() != 410
{
    return Err(format!("Delete event error: {text}"));
}
```

- **404 Not Found**: The event was already deleted (perhaps manually by the user in Google Calendar).
- **410 Gone**: The event was deleted and the deletion has been fully propagated.

Both are treated as success. This is essential for idempotent deletes -- if two rapid toggles fire two delete requests, the second one encountering a 404 is not an error.

### Full Re-Sync as Recovery Mechanism

The `google_full_sync` function is the catch-all for any state that drifted:

- Items that were added while Google was unreachable get their events created.
- Items that were completed but whose delete-event call failed get their orphaned events cleaned up.
- Items whose `google_event_id` was lost (e.g., database restore from backup) get re-linked via `find_event_by_item_id` (which searches by the `life_manager_id` extended property).

This follows Nygard's recommendation of having a "steady state" reconciliation mechanism rather than relying solely on transactional consistency with external systems.

### What Happens When `GOOGLE_SA_KEY_FILE` Is Not Set

The entire feature degrades gracefully to a no-op:

```rust
pub fn is_configured() -> bool {
    std::env::var("GOOGLE_SA_KEY_FILE").is_ok()
}

pub async fn sync_item(...) {
    if !is_configured() {
        return; // Silent no-op
    }
    // ...
}
```

- `sync_item` returns immediately without logging.
- `google_calendar_status` returns `false`, so the UI shows the dim dot and hint text.
- `google_full_sync` returns an error message ("Google Calendar not configured").
- No panics, no error logs, no performance overhead.

This means the app can be deployed and run indefinitely without Google Calendar configured. The feature activates the moment the env var and key file are provided and the app is restarted.

---

## File Reference

| File | Purpose |
|------|---------|
| `src/server/google.rs` | Token management, Calendar API calls, `sync_item` orchestrator |
| `src/api/google.rs` | Server functions: `google_calendar_status`, `google_full_sync` |
| `src/api/checklist.rs` | Sync hooks in `add_checklist`, `toggle_checklist`, `delete_checklist` |
| `src/api/shopee.rs` | Sync hooks in `add_shopee`, `toggle_shopee`, `delete_shopee` |
| `src/components/google_sync.rs` | `GoogleSyncPanel` UI component |
| `src/pages/todos.rs` | Embeds `GoogleSyncPanel` below the checklist |
| `src/server/db.rs` | Schema migrations adding `google_event_id` columns |
| `docker-compose.yml` | Environment variables and key file volume mount |
