# 5. Authentication & Security

> *"Security is not a product, but a process."* — Bruce Schneier
>
> Life Manager's security model is simple by design: Tailscale provides the perimeter, and the app validates everything that crosses it.

## Threat Model

Life Manager is a personal app on a private Tailscale network. The threat model is:

- **Network access**: Only Tailscale members can reach the app. There is no public internet exposure.
- **Authenticated users**: The Tailscale sidecar injects the `Tailscale-User-Login` header for every request.
- **Trust boundary**: We trust Tailscale's header injection but validate its presence. We don't trust the content of user inputs.

### What We Defend Against

1. **Absent authentication** — requests without the Tailscale header are rejected in production
2. **Malformed input** — all text fields are length-limited, dates are format-validated
3. **SQL injection** — parameterized queries throughout
4. **XSS** — Dioxus RSX auto-escapes all interpolated values
5. **Resource exhaustion** — OCR has size limits (10MB) and timeouts (30s)
6. **Data leakage** — server errors are logged but not returned to the client verbatim

### What We Don't Defend Against

1. **Tailscale compromise** — if someone is on the tailnet, they're in
2. **Physical device access** — no per-user encryption at rest
3. **Side-channel attacks** — not relevant for a personal todo app

## Authentication Flow

```rust
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

Key decisions:

1. **Explicit auth flag** — `REQUIRE_AUTH=true` env var, not piggybacking on `DATABASE_PATH`
2. **Shared user ID** — all authenticated users map to `"default"` because this is a household app where everyone shares the same lists
3. **Attribution without isolation** — `display_name_from_headers()` captures the actual Tailscale login for tracking who completed items, but all data is shared

## Input Validation

Validation happens server-side in `src/server/validate.rs`:

```rust
const MAX_TEXT: usize = 500;    // descriptions, titles
const MAX_SHORT: usize = 100;   // codes, store names

pub fn text(s: &str, field: &str) -> Result<(), ServerFnError> { ... }
pub fn short(s: &str, field: &str) -> Result<(), ServerFnError> { ... }
pub fn date(s: &str) -> Result<(), ServerFnError> { ... }
```

Every write API validates before touching the database:

```rust
pub async fn add_checklist(text, category, date) -> Result<(), ServerFnError> {
    let user_id = auth::user_from_headers(&headers)?;  // Auth first
    validate::text(&text, "text")?;                     // Validate second
    if let Some(ref d) = date { validate::date(d)?; }   // Optional field
    // ... database operation ...                        // Act last
}
```

## SQL Injection Prevention

Every query uses `rusqlite::params![]` for parameter binding:

```rust
conn.execute(
    "DELETE FROM checklist_items WHERE id = ?1 AND user_id = ?2",
    params![id, user_id],
)?;
```

The `?1`, `?2` placeholders are bound by the database engine, not by string interpolation. There is no code path in the application that constructs SQL by concatenating user input.

## OCR Security

The OCR endpoint is the highest-risk surface because it:
- Accepts arbitrary binary data (base64-encoded images)
- Spawns an external process (Tesseract)
- Processes untrusted content (user-uploaded screenshots)

Mitigations:

```rust
// 1. Authentication required
auth::user_from_headers(&headers)?;

// 2. Size limit (10MB decoded)
const MAX_BASE64_SIZE: usize = 10 * 1024 * 1024 * 4 / 3;
if raw_b64.len() > MAX_BASE64_SIZE { return Err(...); }

// 3. Timeout (30 seconds)
tokio::time::timeout(Duration::from_secs(30), process.output()).await?;

// 4. Error sanitization
if !output.status.success() {
    tracing::error!("Tesseract failed: {}", stderr);  // Log details
    return Err(ServerFnError::new("OCR processing failed"));  // Generic to client
}
```

## Error Information Leakage

Server errors are sanitized before reaching the client. Internal details (file paths, SQL errors, Tesseract stderr) are logged server-side but never sent to the browser:

```rust
// BAD: leaks server internals
Err(ServerFnError::new(format!("Tesseract failed: {stderr}")))

// GOOD: generic message to client, details in server logs
tracing::error!("Tesseract failed: {stderr}");
Err(ServerFnError::new("OCR processing failed"))
```

## Migration Safety

The user consolidation migration (`UPDATE user_id = 'default'`) runs exactly once, tracked by the `migrations` table:

```rust
run_once(&conn, "consolidate_users", "UPDATE checklist_items SET ...");
```

This prevents the migration from re-running on every server restart, which would be a data integrity risk if multi-user support were ever added.
