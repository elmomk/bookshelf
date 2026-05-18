pub fn user_from_headers(headers: &axum::http::HeaderMap) -> Result<String, String> {
    let require_auth = std::env::var("REQUIRE_AUTH").unwrap_or_default() == "true";
    if require_auth
        && headers.get("Tailscale-User-Login").is_none() {
            return Err("Unauthorized: missing Tailscale-User-Login header".to_string());
        }
    Ok("default".to_string())
}

/// Stable per-reader identity key (the raw Tailscale login, or "local").
/// Used to key the alias table — never changes when the alias changes.
pub fn reader_login(headers: &axum::http::HeaderMap) -> String {
    headers
        .get("Tailscale-User-Login")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "local".to_string())
}

/// The auto-derived default name (login local-part), before any alias.
/// Truncates to 50 chars to prevent abuse.
pub fn base_name_from_headers(headers: &axum::http::HeaderMap) -> String {
    let name = headers
        .get("Tailscale-User-Login")
        .and_then(|v| v.to_str().ok())
        .map(|login| login.split('@').next().unwrap_or(login).to_string())
        .unwrap_or_else(|| "local".to_string());
    name.chars().take(50).collect()
}

/// Per-reader identity used for attribution (who did what): the user's chosen
/// alias if one is set, otherwise the auto-derived name.
pub fn display_name_from_headers(headers: &axum::http::HeaderMap) -> String {
    if let Some(alias) = lookup_alias(&reader_login(headers)) {
        return alias;
    }
    base_name_from_headers(headers)
}

/// Best-effort alias lookup; any failure falls back to the derived name.
fn lookup_alias(login: &str) -> Option<String> {
    let conn = crate::server::db::pool().get().ok()?;
    let alias: String = conn
        .query_row(
            "SELECT alias FROM reader_aliases WHERE login = ?1",
            rusqlite::params![login],
            |r| r.get(0),
        )
        .ok()?;
    let alias: String = alias.trim().chars().take(50).collect();
    (!alias.is_empty()).then_some(alias)
}
