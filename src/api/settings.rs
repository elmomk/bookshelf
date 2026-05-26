use dioxus::prelude::*;

use crate::models::{ChangeRow, SnapshotBook, SnapshotInfo};

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

    let preview_label = if alias.is_empty() {
        format!("set_alias({old} → default)")
    } else {
        format!("set_alias({old} → {alias})")
    };
    let rec = crate::server::changelog::ChangeRecorder::begin(
        &tx,
        Some(old.clone()),
        Some(preview_label),
    )
    .map_err(|e| ServerFnError::new(e.to_string()))?;

    let pk: &[&dyn rusqlite::ToSql] = &[&login];
    let new_name = if alias.is_empty() {
        rec.record_update_with("reader_aliases", pk, |t| {
            t.execute(
                "DELETE FROM reader_aliases WHERE login = ?1",
                rusqlite::params![login],
            )
            .map(|_| ())
        })
        .map_err(|e| ServerFnError::new(e.to_string()))?;
        auth::base_name_from_headers(&headers)
    } else {
        rec.record_update_with("reader_aliases", pk, |t| {
            t.execute(
                "INSERT INTO reader_aliases (login, alias, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(login) DO UPDATE SET alias = ?2, updated_at = ?3",
                rusqlite::params![login, alias, now],
            )
            .map(|_| ())
        })
        .map_err(|e| ServerFnError::new(e.to_string()))?;
        alias.clone()
    };

    if new_name != old {
        // `OR IGNORE` on the uniquely-keyed tables: if the new name already has
        // a row (alias collides with another identity) keep the existing one
        // rather than aborting.
        //
        // These bulk renames are NOT logged row-by-row — they could touch
        // hundreds of rows and the canonical record of the decision is the
        // `reader_aliases` change above. The high-level event below summarizes
        // the rewrite; an alias-undo should be implemented as a re-run of
        // set_alias with the old name, not as per-row inverse-replay.
        let mut affected: [i64; 6] = [0; 6];
        for (i, sql) in [
            "UPDATE OR IGNORE reading_progress SET reader = ?1 WHERE reader = ?2",
            "UPDATE book_comments SET author = ?1 WHERE author = ?2",
            "UPDATE notifications SET actor = ?1 WHERE actor = ?2",
            "UPDATE OR IGNORE notification_reads SET user_name = ?1 WHERE user_name = ?2",
            "UPDATE OR IGNORE notification_settings SET user_name = ?1 WHERE user_name = ?2",
            "UPDATE push_subscriptions SET user_name = ?1 WHERE user_name = ?2",
        ]
        .iter()
        .enumerate()
        {
            affected[i] = tx
                .execute(sql, rusqlite::params![new_name, old])
                .map_err(|e| ServerFnError::new(e.to_string()))?
                as i64;
        }

        let details = serde_json::json!({
            "old": old,
            "new": new_name,
            "reading_progress": affected[0],
            "book_comments": affected[1],
            "notifications": affected[2],
            "notification_reads": affected[3],
            "notification_settings": affected[4],
            "push_subscriptions": affected[5],
        })
        .to_string();
        rec.record_event("alias_rewrite", None, None, Some(details))
            .map_err(|e| ServerFnError::new(e.to_string()))?;
    }

    tx.commit()
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(new_name)
}

// ============================================================================
// Snapshots / saved history
// ============================================================================

/// Snapshot the live DB now. Returns the new entry.
#[server(headers: axum::http::HeaderMap)]
pub async fn create_snapshot() -> Result<SnapshotInfo, ServerFnError> {
    use crate::server::{auth, db, snapshots};
    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    let conn = db::pool()
        .get()
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let id = snapshots::create_manual(&conn).map_err(ServerFnError::new)?;
    // Manual cap (50) might just have been exceeded — best-effort sweep.
    snapshots::prune();
    info_for(&id).ok_or_else(|| ServerFnError::new("Snapshot vanished after create"))
}

/// All saved snapshots, newest first.
#[server(headers: axum::http::HeaderMap)]
pub async fn list_snapshots() -> Result<Vec<SnapshotInfo>, ServerFnError> {
    use crate::server::{auth, snapshots};
    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    Ok(snapshots::list_ids()
        .into_iter()
        .filter_map(|id| info_for(&id))
        .collect())
}

/// Books present in a given snapshot — used by the per-book restore picker.
#[server(headers: axum::http::HeaderMap)]
pub async fn list_books_in_snapshot(
    id: String,
) -> Result<Vec<SnapshotBook>, ServerFnError> {
    use crate::server::{auth, snapshots};
    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    let rows = snapshots::list_books_in(&id).map_err(ServerFnError::new)?;
    Ok(rows
        .into_iter()
        .map(|(id, title, author)| SnapshotBook { id, title, author })
        .collect())
}

/// Delete a snapshot file by id.
#[server(headers: axum::http::HeaderMap)]
pub async fn delete_snapshot(id: String) -> Result<(), ServerFnError> {
    use crate::server::{auth, snapshots};
    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    snapshots::delete(&id).map_err(ServerFnError::new)
}

/// Roll the entire database back to the chosen snapshot. Auto-creates a
/// "pre-restore" snapshot first so this is itself undoable.
#[server(headers: axum::http::HeaderMap)]
pub async fn restore_full_from_snapshot(id: String) -> Result<(), ServerFnError> {
    use crate::server::{auth, db, snapshots};
    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    let snap_path = snapshots::path_for(&id)
        .ok_or_else(|| ServerFnError::new("Snapshot not found"))?;
    let snap_str = snap_path.display().to_string().replace('\'', "''");

    let mut conn = db::pool()
        .get()
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    // Safety net: take a pre-restore snapshot so a botched rollback is undoable.
    let _ = snapshots::create_safety(&conn);

    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    tx.execute(&format!("ATTACH DATABASE '{snap_str}' AS snap"), [])
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    // Replace each user-data table from the snapshot, intersecting columns
    // so older snapshots (pre-deleted_at) restore cleanly into a newer schema.
    // Order matters for FK cascade safety: children first on delete.
    for table in [
        "comment_reactions",
        "book_comments",
        "reading_progress",
        "books",
    ] {
        let cols = snapshots::common_cols(&tx, "snap", table)
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        if cols.is_empty() {
            continue;
        }
        tx.execute(&format!("DELETE FROM {table}"), [])
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        let cols_list = cols.join(", ");
        tx.execute(
            &format!(
                "INSERT INTO {table} ({cols_list}) SELECT {cols_list} FROM snap.{table}"
            ),
            [],
        )
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    }

    tx.execute("DETACH DATABASE snap", [])
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    tx.commit()
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(())
}

/// Roll a single book (and its progress + comments + reactions) back to the
/// chosen snapshot — non-destructive to every other book on the shelf.
#[server(headers: axum::http::HeaderMap)]
pub async fn restore_book_from_snapshot(
    snapshot_id: String,
    book_id: String,
) -> Result<(), ServerFnError> {
    use crate::server::{auth, db, snapshots};
    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    let snap_path = snapshots::path_for(&snapshot_id)
        .ok_or_else(|| ServerFnError::new("Snapshot not found"))?;
    let snap_str = snap_path.display().to_string().replace('\'', "''");

    let mut conn = db::pool()
        .get()
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    // Safety net.
    let _ = snapshots::create_safety(&conn);

    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    tx.execute(&format!("ATTACH DATABASE '{snap_str}' AS snap"), [])
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    // Confirm the book actually exists in the snapshot first.
    let exists: i32 = tx
        .query_row(
            "SELECT COUNT(*) FROM snap.books WHERE id = ?1",
            rusqlite::params![book_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if exists == 0 {
        tx.execute("DETACH DATABASE snap", []).ok();
        return Err(ServerFnError::new("This book isn't in that snapshot"));
    }

    // Wipe live rows for this book (children first to honour FKs), then copy
    // the snapshot's version with column intersection.
    tx.execute(
        "DELETE FROM comment_reactions WHERE comment_id IN (
            SELECT id FROM book_comments WHERE book_id = ?1)",
        rusqlite::params![book_id],
    )
    .map_err(|e| ServerFnError::new(e.to_string()))?;
    tx.execute(
        "DELETE FROM book_comments WHERE book_id = ?1",
        rusqlite::params![book_id],
    )
    .map_err(|e| ServerFnError::new(e.to_string()))?;
    tx.execute(
        "DELETE FROM reading_progress WHERE book_id = ?1",
        rusqlite::params![book_id],
    )
    .map_err(|e| ServerFnError::new(e.to_string()))?;
    tx.execute(
        "DELETE FROM books WHERE id = ?1",
        rusqlite::params![book_id],
    )
    .map_err(|e| ServerFnError::new(e.to_string()))?;

    for (table, where_clause) in [
        ("books", "id = ?1".to_string()),
        ("reading_progress", "book_id = ?1".to_string()),
        ("book_comments", "book_id = ?1".to_string()),
        (
            "comment_reactions",
            "comment_id IN (SELECT id FROM snap.book_comments WHERE book_id = ?1)"
                .to_string(),
        ),
    ] {
        let cols = snapshots::common_cols(&tx, "snap", table)
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        if cols.is_empty() {
            continue;
        }
        let cols_list = cols.join(", ");
        let sql = format!(
            "INSERT INTO {table} ({cols_list}) SELECT {cols_list} FROM snap.{table} \
             WHERE {where_clause}"
        );
        tx.execute(&sql, rusqlite::params![book_id])
            .map_err(|e| ServerFnError::new(e.to_string()))?;
    }

    tx.execute("DETACH DATABASE snap", [])
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    tx.commit()
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn info_for(id: &str) -> Option<SnapshotInfo> {
    use crate::server::snapshots;
    let created_at = snapshots::ts_of(id)?;
    let (books, comments, reactions) = snapshots::counts(id).ok()?;
    Some(SnapshotInfo {
        id: id.to_string(),
        created_at,
        size_bytes: snapshots::size_of(id),
        books,
        comments,
        reactions,
    })
}

#[cfg(target_arch = "wasm32")]
fn info_for(_id: &str) -> Option<SnapshotInfo> {
    None
}

// ============================================================================
// Change log (per-write audit & undo)
// ============================================================================

/// Recent rows from `db_changes`, newest first. Capped at 200.
#[server(headers: axum::http::HeaderMap)]
pub async fn list_changes(limit: u32, offset: u32) -> Result<Vec<ChangeRow>, ServerFnError> {
    use crate::server::{auth, db};
    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    let limit = limit.clamp(1, 200);
    let conn = db::pool()
        .get()
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let mut stmt = conn
        .prepare(
            "SELECT id, tx_id, ts, actor, label, op, tbl, row_pk_json, old_json, new_json
             FROM db_changes ORDER BY id DESC LIMIT ?1 OFFSET ?2",
        )
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let rows = stmt
        .query_map(rusqlite::params![limit, offset], |r| {
            Ok(ChangeRow {
                id: r.get(0)?,
                tx_id: r.get(1)?,
                ts: r.get(2)?,
                actor: r.get(3)?,
                label: r.get(4)?,
                op: r.get(5)?,
                tbl: r.get(6)?,
                row_pk_json: r.get(7)?,
                old_json: r.get(8)?,
                new_json: r.get(9)?,
            })
        })
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| ServerFnError::new(e.to_string()))?);
    }
    Ok(out)
}

/// Undo a single logged row change. Refuses if a newer change has touched the
/// same row — that row's state has moved on and the right tool is
/// `restore_to_before_tx`.
#[server(headers: axum::http::HeaderMap)]
pub async fn undo_change(id: i64) -> Result<(), ServerFnError> {
    use crate::server::{auth, changelog, db, snapshots};
    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    let me = auth::display_name_from_headers(&headers);

    let mut conn = db::pool()
        .get()
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    // Safety net: take a pre-undo snapshot.
    let _ = snapshots::create_safety(&conn);

    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    tx.execute("PRAGMA defer_foreign_keys = ON", [])
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let (op, tbl_opt, pk_opt, old_opt, target_tx_id): (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        i64,
    ) = tx
        .query_row(
            "SELECT op, tbl, row_pk_json, old_json, tx_id FROM db_changes WHERE id = ?1",
            rusqlite::params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .map_err(|_| ServerFnError::new("Change not found"))?;

    // If this change is part of a multi-table alias rewrite, single-row undo
    // would leave the cascaded renames in place — direct the user at the
    // whole-transaction restore instead.
    let part_of_alias_rewrite: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM db_changes
                           WHERE tx_id = ?1 AND op = 'alias_rewrite')",
            rusqlite::params![target_tx_id],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if part_of_alias_rewrite {
        return Err(ServerFnError::new(
            "This change is part of a multi-table alias rewrite. \
             Use 'Restore to before this' to revert the full rename.",
        ));
    }

    if !matches!(op.as_str(), "INSERT" | "UPDATE" | "DELETE") {
        return Err(ServerFnError::new(
            "This kind of change can't be undone directly. Use 'Restore to before this' instead.",
        ));
    }
    let tbl = tbl_opt
        .ok_or_else(|| ServerFnError::new("Change has no target table"))?;
    let pk = pk_opt.ok_or_else(|| ServerFnError::new("Change has no primary key"))?;

    let superseded: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM db_changes
                           WHERE id > ?1 AND tbl = ?2 AND row_pk_json = ?3)",
            rusqlite::params![id, tbl, pk],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if superseded {
        return Err(ServerFnError::new(
            "Superseded by a later edit; use 'Restore to before this' instead.",
        ));
    }

    changelog::apply_inverse(&tx, &op, &tbl, &pk, old_opt.as_deref())
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    // Log the undo itself as a new high-level event.
    let tx_id_new = changelog::next_tx_id(&tx)
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let now = chrono::Utc::now().timestamp_millis() as f64;
    let pk_ref = serde_json::json!({ "change_id": id }).to_string();
    tx.execute(
        "INSERT INTO db_changes
            (tx_id, ts, actor, label, op, tbl, row_pk_json, old_json, new_json)
         VALUES (?1, ?2, ?3, ?4, 'undo', NULL, ?5, NULL, NULL)",
        rusqlite::params![tx_id_new, now, me, format!("undo_change({id})"), pk_ref],
    )
    .map_err(|e| ServerFnError::new(e.to_string()))?;

    tx.commit()
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(())
}

/// Roll the DB back to the state immediately before the given transaction —
/// applies inverses of every logged row change with `tx_id >= ?` in reverse
/// id order, atomically and with deferred FK checks.
#[server(headers: axum::http::HeaderMap)]
pub async fn restore_to_before_tx(tx_id: i64) -> Result<(), ServerFnError> {
    use crate::server::{auth, changelog, db, snapshots};
    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    let me = auth::display_name_from_headers(&headers);

    let mut conn = db::pool()
        .get()
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let _ = snapshots::create_safety(&conn);

    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    tx.execute("PRAGMA defer_foreign_keys = ON", [])
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    // Fetch all row-level changes in this tx and everything after, newest first.
    let rows: Vec<(String, Option<String>, Option<String>, Option<String>)> = {
        let mut stmt = tx
            .prepare(
                "SELECT op, tbl, row_pk_json, old_json
                 FROM db_changes WHERE tx_id >= ?1 ORDER BY id DESC",
            )
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        let it = stmt
            .query_map(rusqlite::params![tx_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        let mut v = Vec::new();
        for r in it {
            v.push(r.map_err(|e| ServerFnError::new(e.to_string()))?);
        }
        v
    };

    let mut applied = 0usize;
    for (op, tbl_opt, pk_opt, old_opt) in rows {
        match op.as_str() {
            "INSERT" | "UPDATE" | "DELETE" => {
                let Some(tbl) = tbl_opt else { continue };
                let Some(pk) = pk_opt else { continue };
                changelog::apply_inverse(&tx, &op, &tbl, &pk, old_opt.as_deref())
                    .map_err(|e| ServerFnError::new(e.to_string()))?;
                applied += 1;
            }
            "alias_rewrite" => {
                // The bulk renames live only in this event's details_json
                // (`old_json` slot); replay them in reverse via the helper.
                if let Some(details) = old_opt.as_deref() {
                    changelog::apply_alias_rewrite_inverse(&tx, details)
                        .map_err(|e| ServerFnError::new(e.to_string()))?;
                    applied += 1;
                }
            }
            // Other metadata events (undo, restore_point, restore_full,
            // restore_book) are audit-only — their underlying row changes
            // (if any) are logged separately.
            _ => {}
        }
    }

    let tx_id_new = changelog::next_tx_id(&tx)
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let now = chrono::Utc::now().timestamp_millis() as f64;
    let pk_ref = serde_json::json!({ "tx_id_target": tx_id }).to_string();
    let details = serde_json::json!({ "ops_reverted": applied }).to_string();
    tx.execute(
        "INSERT INTO db_changes
            (tx_id, ts, actor, label, op, tbl, row_pk_json, old_json, new_json)
         VALUES (?1, ?2, ?3, ?4, 'restore_point', NULL, ?5, ?6, NULL)",
        rusqlite::params![
            tx_id_new,
            now,
            me,
            format!("restore_to_before_tx({tx_id}) — {applied} ops"),
            pk_ref,
            details,
        ],
    )
    .map_err(|e| ServerFnError::new(e.to_string()))?;

    tx.commit()
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(())
}
