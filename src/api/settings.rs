use dioxus::prelude::*;

/// Returns `(current_display_name, auto_derived_default_name)`.
/// When no alias is set the two are equal.
#[server(headers: axum::http::HeaderMap)]
pub async fn get_identity() -> Result<(String, String), ServerFnError> {
    use crate::server::auth;
    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    Ok((
        auth::display_name_from_headers(&headers),
        auth::base_name_from_headers(&headers),
    ))
}

/// Set (or, with an empty/blank string, clear) this reader's alias.
///
/// Rewrites the reader's existing rows (comments, reading progress, activity,
/// notification keys) from the old name to the new one so their identity stays
/// coherent. Returns the resulting display name.
#[server(headers: axum::http::HeaderMap)]
pub async fn set_alias(alias: String) -> Result<String, ServerFnError> {
    use crate::server::{auth, db};
    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;

    let alias: String = alias
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(50)
        .collect();
    let login = auth::reader_login(&headers);
    let old = auth::display_name_from_headers(&headers);
    let now = chrono::Utc::now().timestamp_millis() as f64;

    let mut conn = db::pool()
        .get()
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    // One IMMEDIATE transaction takes a write lock up front and holds it
    // across the whole check-then-write, so a concurrent set_alias can't slip
    // a colliding claim between the pre-check and the rewrite — it waits on
    // the connection's busy_timeout instead. Any early return drops `tx`,
    // rolling everything back.
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    // Reject a non-empty alias that already belongs to a *different* identity.
    // Invariant: all of this reader's rows are under `old`, so any occurrence
    // of the chosen name that isn't `old` (and any other login's matching
    // alias) means adopting it would merge/clobber someone else. Comparison is
    // case-insensitive to avoid near-duplicate readers. Clearing the alias
    // (empty) reverts to your own derived name and is always allowed.
    if !alias.is_empty() {
        let taken: bool = tx
            .query_row(
                "SELECT EXISTS (
                    SELECT 1 FROM reader_aliases
                      WHERE login <> ?1 AND alias = ?2 COLLATE NOCASE
                    UNION ALL
                    SELECT 1 FROM reading_progress
                      WHERE reader = ?2 COLLATE NOCASE AND reader <> ?3 COLLATE NOCASE
                    UNION ALL
                    SELECT 1 FROM book_comments
                      WHERE author = ?2 COLLATE NOCASE AND author <> ?3 COLLATE NOCASE
                    UNION ALL
                    SELECT 1 FROM notifications
                      WHERE actor = ?2 COLLATE NOCASE AND actor <> ?3 COLLATE NOCASE
                )",
                rusqlite::params![login, alias, old],
                |r| r.get(0),
            )
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        if taken {
            return Err(ServerFnError::new(format!(
                "\"{alias}\" is already taken by another reader — pick a different alias."
            )));
        }
    }

    let new_name = if alias.is_empty() {
        tx.execute(
            "DELETE FROM reader_aliases WHERE login = ?1",
            rusqlite::params![login],
        )
        .map_err(|e| ServerFnError::new(e.to_string()))?;
        auth::base_name_from_headers(&headers)
    } else {
        tx.execute(
            "INSERT INTO reader_aliases (login, alias, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(login) DO UPDATE SET alias = ?2, updated_at = ?3",
            rusqlite::params![login, alias, now],
        )
        .map_err(|e| ServerFnError::new(e.to_string()))?;
        alias.clone()
    };

    if new_name != old {
        // `OR IGNORE` on the uniquely-keyed tables: if the new name already has
        // a row (alias collides with another identity) keep the existing one
        // rather than aborting.
        for sql in [
            "UPDATE OR IGNORE reading_progress SET reader = ?1 WHERE reader = ?2",
            "UPDATE book_comments SET author = ?1 WHERE author = ?2",
            "UPDATE notifications SET actor = ?1 WHERE actor = ?2",
            "UPDATE OR IGNORE notification_reads SET user_name = ?1 WHERE user_name = ?2",
            "UPDATE OR IGNORE notification_settings SET user_name = ?1 WHERE user_name = ?2",
            "UPDATE push_subscriptions SET user_name = ?1 WHERE user_name = ?2",
        ] {
            tx.execute(sql, rusqlite::params![new_name, old])
                .map_err(|e| ServerFnError::new(e.to_string()))?;
        }
    }

    tx.commit()
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(new_name)
}
