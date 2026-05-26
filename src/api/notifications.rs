use dioxus::prelude::*;

use crate::models::notification::NotificationStatus;

#[server(headers: axum::http::HeaderMap)]
pub async fn list_notifications() -> Result<NotificationStatus, ServerFnError> {
    use crate::models::notification::Notification;
    use crate::server::{auth, db};

    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    let display_name = auth::display_name_from_headers(&headers);
    let conn = db::pool().get().map_err(|e| ServerFnError::new(e.to_string()))?;

    // Check if user has opted in (default: disabled)
    let enabled: bool = conn
        .query_row(
            "SELECT enabled FROM notification_settings WHERE user_name = ?1",
            rusqlite::params![display_name],
            |row| row.get::<_, i32>(0),
        )
        .unwrap_or(0)
        != 0;

    if !enabled {
        return Ok(NotificationStatus {
            notifications: vec![],
            unread_count: 0,
        });
    }

    // Get last_read_at and cleared_at for filtering
    let (last_read_at, cleared_at): (f64, f64) = conn
        .query_row(
            "SELECT last_read_at, cleared_at FROM notification_reads WHERE user_name = ?1",
            rusqlite::params![display_name],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap_or((0.0, 0.0));

    // Fetch notifications from others created after cleared_at, newest first, limit 50
    let mut stmt = conn
        .prepare(
            "SELECT id, actor, action, module, item_text, created_at
             FROM notifications
             WHERE actor != ?1 AND created_at > ?2
             ORDER BY created_at DESC
             LIMIT 50",
        )
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let notifications = stmt
        .query_map(rusqlite::params![display_name, cleared_at], |row| {
            Ok(Notification {
                id: row.get(0)?,
                actor: row.get(1)?,
                action: row.get(2)?,
                module: row.get(3)?,
                item_text: row.get(4)?,
                created_at: row.get(5)?,
            })
        })
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let unread_count = notifications
        .iter()
        .filter(|n| n.created_at > last_read_at)
        .count() as u32;

    Ok(NotificationStatus {
        notifications,
        unread_count,
    })
}

#[server(headers: axum::http::HeaderMap)]
pub async fn mark_notifications_read() -> Result<(), ServerFnError> {
    use crate::server::{auth, db};

    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    let display_name = auth::display_name_from_headers(&headers);
    let conn = db::pool().get().map_err(|e| ServerFnError::new(e.to_string()))?;

    let now = chrono::Utc::now().timestamp_millis() as f64;
    conn.execute(
        "INSERT INTO notification_reads (user_name, last_read_at) VALUES (?1, ?2)
         ON CONFLICT(user_name) DO UPDATE SET last_read_at = ?2",
        rusqlite::params![display_name, now],
    )
    .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(())
}

#[server(headers: axum::http::HeaderMap)]
pub async fn set_notification_enabled(enabled: bool) -> Result<(), ServerFnError> {
    use crate::server::{auth, changelog, db};

    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    let display_name = auth::display_name_from_headers(&headers);
    let mut conn = db::pool()
        .get()
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let label = format!(
        "set_notification_enabled({})",
        if enabled { "on" } else { "off" }
    );
    let rec =
        changelog::ChangeRecorder::begin(&tx, Some(display_name.clone()), Some(label))
            .map_err(|e| ServerFnError::new(e.to_string()))?;

    // Branch the UPSERT so the recorder logs a clean INSERT or UPDATE.
    let pk: &[&dyn rusqlite::ToSql] = &[&display_name];
    rec.record_update_with("notification_settings", pk, |t| {
        // record_update_with falls back to INSERT logging if the row didn't
        // exist before; either way we just run the UPSERT here.
        t.execute(
            "INSERT INTO notification_settings (user_name, enabled) VALUES (?1, ?2)
             ON CONFLICT(user_name) DO UPDATE SET enabled = ?2",
            rusqlite::params![display_name, enabled as i32],
        )
        .map(|_| ())
    })
    .map_err(|e| ServerFnError::new(e.to_string()))?;

    tx.commit()
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(())
}

#[server(headers: axum::http::HeaderMap)]
pub async fn get_notification_enabled() -> Result<bool, ServerFnError> {
    use crate::server::{auth, db};

    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    let display_name = auth::display_name_from_headers(&headers);
    let conn = db::pool().get().map_err(|e| ServerFnError::new(e.to_string()))?;

    let enabled: bool = conn
        .query_row(
            "SELECT enabled FROM notification_settings WHERE user_name = ?1",
            rusqlite::params![display_name],
            |row| row.get::<_, i32>(0),
        )
        .unwrap_or(0)
        != 0;

    Ok(enabled)
}

/// Clear notifications for the current user by setting cleared_at to now.
/// Does not delete any rows — other users can still see them.
#[server(headers: axum::http::HeaderMap)]
pub async fn clear_notifications() -> Result<(), ServerFnError> {
    use crate::server::{auth, db};

    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    let display_name = auth::display_name_from_headers(&headers);
    let conn = db::pool().get().map_err(|e| ServerFnError::new(e.to_string()))?;

    let now = chrono::Utc::now().timestamp_millis() as f64;
    conn.execute(
        "INSERT INTO notification_reads (user_name, last_read_at, cleared_at) VALUES (?1, ?2, ?2)
         ON CONFLICT(user_name) DO UPDATE SET last_read_at = ?2, cleared_at = ?2",
        rusqlite::params![display_name, now],
    )
    .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(())
}

/// Returns the VAPID public key so the client can subscribe to push.
#[server(headers: axum::http::HeaderMap)]
pub async fn get_vapid_public_key() -> Result<String, ServerFnError> {
    use crate::server::auth;

    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    let key = std::env::var("VAPID_PUBLIC_KEY").unwrap_or_default();
    // Validate base64url format to prevent injection when interpolated into JS
    if !key.is_empty() && !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err(ServerFnError::new("Invalid VAPID public key format"));
    }
    Ok(key)
}

/// Save a push subscription for the current user.
#[server(headers: axum::http::HeaderMap)]
pub async fn save_push_subscription(
    endpoint: String,
    p256dh: String,
    auth: String,
) -> Result<(), ServerFnError> {
    use crate::server::{auth as auth_mod, db};

    auth_mod::user_from_headers(&headers).map_err(ServerFnError::new)?;
    let display_name = auth_mod::display_name_from_headers(&headers);
    let conn = db::pool().get().map_err(|e| ServerFnError::new(e.to_string()))?;

    // Remove any existing subscription with same endpoint for this user
    conn.execute(
        "DELETE FROM push_subscriptions WHERE user_name = ?1 AND endpoint = ?2",
        rusqlite::params![display_name, endpoint],
    )
    .map_err(|e| ServerFnError::new(e.to_string()))?;

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis() as f64;

    conn.execute(
        "INSERT INTO push_subscriptions (id, user_name, endpoint, p256dh, auth, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![id, display_name, endpoint, p256dh, auth, now],
    )
    .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(())
}

/// Remove push subscriptions for the current user.
#[server(headers: axum::http::HeaderMap)]
pub async fn remove_push_subscription() -> Result<(), ServerFnError> {
    use crate::server::{auth as auth_mod, db};

    auth_mod::user_from_headers(&headers).map_err(ServerFnError::new)?;
    let display_name = auth_mod::display_name_from_headers(&headers);
    let conn = db::pool().get().map_err(|e| ServerFnError::new(e.to_string()))?;

    conn.execute(
        "DELETE FROM push_subscriptions WHERE user_name = ?1",
        rusqlite::params![display_name],
    )
    .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(())
}
