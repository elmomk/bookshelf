//! Per-write audit log. Server fns wrap their mutating SQL inside an
//! `IMMEDIATE` transaction together with one or more `ChangeRecorder` calls;
//! the recorder captures pre/post row JSON via `json_object()` and appends a
//! row to `db_changes` atomically with the data write. The `Settings → Change
//! log` UI lists these rows and offers per-change undo / restore-to-before.
//!
//! Constraint: a `ChangeRecorder` can only be built from a `&Transaction`, so
//! the type system enforces that recorded writes are atomic.

// Phase 0 ships the foundation only — the first caller arrives in Phase 1.
#![allow(dead_code)]

use rusqlite::OptionalExtension;
use rusqlite::{params_from_iter, ToSql, Transaction};

/// Returns the ordered list of PK columns for a logged table. Empty slice
/// signals "not a logged table" — call sites should error out.
pub fn pk_cols_of(tbl: &str) -> &'static [&'static str] {
    match tbl {
        "books" => &["id"],
        "reading_progress" => &["id"],
        "book_comments" => &["id"],
        "comment_reactions" => &["comment_id", "reader", "emoji"],
        "reader_aliases" => &["login"],
        "notification_settings" => &["user_name"],
        _ => &[],
    }
}

/// Returns the ordered list of all columns in a logged table. Used to build
/// the `json_object('col', col, ...)` SQL for old/new row capture.
///
/// **Invariant**: any future column added to a logged table must be NULLable
/// or have a DEFAULT — inverse-replay would otherwise violate NOT NULL when
/// restoring an `old_json` captured before that column existed.
pub fn data_cols_of(tbl: &str) -> &'static [&'static str] {
    match tbl {
        "books" => &[
            "id",
            "title",
            "author",
            "cover_url",
            "total_pages",
            "total_chapters",
            "description",
            "google_books_id",
            "isbn",
            "added_by",
            "created_at",
            "toc_json",
            "deleted_at",
        ],
        "reading_progress" => &[
            "id",
            "book_id",
            "reader",
            "current_page",
            "current_chapter",
            "status",
            "updated_at",
        ],
        "book_comments" => &[
            "id",
            "book_id",
            "author",
            "body",
            "page",
            "chapter",
            "created_at",
            "parent_id",
            "deleted_at",
        ],
        "comment_reactions" => &["comment_id", "reader", "emoji", "created_at"],
        "reader_aliases" => &["login", "alias", "updated_at"],
        "notification_settings" => &["user_name", "enabled"],
        _ => &[],
    }
}

fn json_object_sql(tbl: &str) -> String {
    let cols = data_cols_of(tbl);
    let parts: Vec<String> = cols.iter().map(|c| format!("'{c}', {c}")).collect();
    format!("json_object({})", parts.join(", "))
}

fn pk_where_sql(tbl: &str) -> String {
    pk_cols_of(tbl)
        .iter()
        .enumerate()
        .map(|(i, c)| format!("{c} = ?{}", i + 1))
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn now_ms() -> f64 {
    chrono::Utc::now().timestamp_millis() as f64
}

fn invalid(reason: &'static str) -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(reason.to_string())
}

/// Allocate a fresh transaction id from `change_tx_seq`.
pub fn next_tx_id(tx: &Transaction) -> rusqlite::Result<i64> {
    tx.query_row(
        "UPDATE change_tx_seq SET v = v + 1 WHERE only_row = 1 RETURNING v",
        [],
        |r| r.get(0),
    )
}

/// Captures row-level pre/post state for every write inside the wrapping tx.
pub struct ChangeRecorder<'tx> {
    tx: &'tx Transaction<'tx>,
    tx_id: i64,
    actor: Option<String>,
    label: Option<String>,
}

impl<'tx> ChangeRecorder<'tx> {
    /// Open a recorder bound to this transaction. `actor` is typically
    /// `auth::display_name_from_headers(&headers)`; `label` is a short human
    /// summary like `"delete_book(13f5…)"`.
    pub fn begin(
        tx: &'tx Transaction<'tx>,
        actor: Option<String>,
        label: Option<String>,
    ) -> rusqlite::Result<Self> {
        let tx_id = next_tx_id(tx)?;
        Ok(Self {
            tx,
            tx_id,
            actor,
            label,
        })
    }

    /// `tx_id` of this recorder — exposed so callers can refer back to it
    /// (e.g. to log a follow-up "restore_point" event under the same group).
    pub fn tx_id(&self) -> i64 {
        self.tx_id
    }

    /// Record an INSERT. The caller must run the INSERT *before* calling
    /// this (so the new row exists to be read back).
    pub fn record_insert(
        &self,
        tbl: &str,
        pk_vals: &[&dyn ToSql],
    ) -> rusqlite::Result<()> {
        let new_json = self
            .fetch_row_json(tbl, pk_vals)?
            .ok_or_else(|| invalid("record_insert: row not found after insert"))?;
        let pk_json = self.pk_json_for(tbl, pk_vals)?;
        self.write_log("INSERT", Some(tbl), Some(pk_json), None, Some(new_json))
    }

    /// Record an UPDATE around a caller-supplied mutator closure. Reads the
    /// row before and after — guarantees `old_json`/`new_json` agree with the
    /// actual change by construction. If the row didn't exist before, the
    /// event is logged as an INSERT instead.
    pub fn record_update_with<F>(
        &self,
        tbl: &str,
        pk_vals: &[&dyn ToSql],
        mutate: F,
    ) -> rusqlite::Result<()>
    where
        F: FnOnce(&Transaction) -> rusqlite::Result<()>,
    {
        let old_json = self.fetch_row_json(tbl, pk_vals)?;
        mutate(self.tx)?;
        let new_json = self.fetch_row_json(tbl, pk_vals)?;
        let pk_json = self.pk_json_for(tbl, pk_vals)?;
        match (&old_json, &new_json) {
            (None, None) => Ok(()),
            (None, Some(_)) => {
                self.write_log("INSERT", Some(tbl), Some(pk_json), None, new_json)
            }
            (Some(_), None) => {
                self.write_log("DELETE", Some(tbl), Some(pk_json), old_json, None)
            }
            _ => self.write_log("UPDATE", Some(tbl), Some(pk_json), old_json, new_json),
        }
    }

    /// Record a DELETE. The recorder captures `old_json`, then runs the
    /// `DELETE FROM tbl WHERE <pk>` itself.
    pub fn record_delete(
        &self,
        tbl: &str,
        pk_vals: &[&dyn ToSql],
    ) -> rusqlite::Result<()> {
        let old_json = self
            .fetch_row_json(tbl, pk_vals)?
            .ok_or_else(|| invalid("record_delete: row not found"))?;
        let pk_json = self.pk_json_for(tbl, pk_vals)?;
        let where_sql = pk_where_sql(tbl);
        if where_sql.is_empty() {
            return Err(invalid("record_delete: not a logged table"));
        }
        self.tx.execute(
            &format!("DELETE FROM {tbl} WHERE {where_sql}"),
            params_from_iter(pk_vals.iter().copied()),
        )?;
        self.write_log("DELETE", Some(tbl), Some(pk_json), Some(old_json), None)
    }

    /// Log a high-level event (`restore_full`, `restore_book`, `undo`,
    /// `restore_point`). `tbl` may be `None`; `pk_json` typically encodes
    /// the snapshot id or referenced book id.
    pub fn record_event(
        &self,
        op: &str,
        tbl: Option<&str>,
        pk_json: Option<String>,
        details_json: Option<String>,
    ) -> rusqlite::Result<()> {
        self.write_log(op, tbl, pk_json, details_json, None)
    }

    fn fetch_row_json(
        &self,
        tbl: &str,
        pk_vals: &[&dyn ToSql],
    ) -> rusqlite::Result<Option<String>> {
        let cols = pk_cols_of(tbl);
        if cols.is_empty() {
            return Err(invalid("not a logged table"));
        }
        if cols.len() != pk_vals.len() {
            return Err(invalid("pk column / value count mismatch"));
        }
        let where_sql = pk_where_sql(tbl);
        let json_sql = json_object_sql(tbl);
        let sql = format!("SELECT {json_sql} FROM {tbl} WHERE {where_sql}");
        self.tx
            .query_row(&sql, params_from_iter(pk_vals.iter().copied()), |r| {
                r.get::<_, String>(0)
            })
            .optional()
    }

    fn pk_json_for(
        &self,
        tbl: &str,
        pk_vals: &[&dyn ToSql],
    ) -> rusqlite::Result<String> {
        let cols = pk_cols_of(tbl);
        if cols.len() != pk_vals.len() {
            return Err(invalid("pk col/value mismatch"));
        }
        let args = cols
            .iter()
            .enumerate()
            .map(|(i, c)| format!("'{c}', ?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("SELECT json_object({args})");
        self.tx
            .query_row(&sql, params_from_iter(pk_vals.iter().copied()), |r| {
                r.get::<_, String>(0)
            })
    }

    fn write_log(
        &self,
        op: &str,
        tbl: Option<&str>,
        row_pk_json: Option<String>,
        old_json: Option<String>,
        new_json: Option<String>,
    ) -> rusqlite::Result<()> {
        self.tx.execute(
            "INSERT INTO db_changes
                (tx_id, ts, actor, label, op, tbl, row_pk_json, old_json, new_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                self.tx_id,
                now_ms(),
                self.actor,
                self.label,
                op,
                tbl,
                row_pk_json,
                old_json,
                new_json,
            ],
        )?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// Spin up an in-memory DB with the minimal schema we need.
    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE books (
                 id TEXT PRIMARY KEY,
                 title TEXT NOT NULL,
                 author TEXT,
                 cover_url TEXT,
                 total_pages INTEGER,
                 total_chapters INTEGER,
                 description TEXT,
                 google_books_id TEXT,
                 isbn TEXT,
                 added_by TEXT,
                 created_at REAL NOT NULL,
                 toc_json TEXT,
                 deleted_at REAL
             );
             CREATE TABLE book_comments (
                 id TEXT PRIMARY KEY,
                 book_id TEXT NOT NULL,
                 author TEXT NOT NULL,
                 body TEXT NOT NULL,
                 page INTEGER,
                 chapter INTEGER,
                 created_at REAL NOT NULL,
                 parent_id TEXT,
                 deleted_at REAL
             );
             CREATE TABLE comment_reactions (
                 comment_id TEXT NOT NULL,
                 reader TEXT NOT NULL,
                 emoji TEXT NOT NULL,
                 created_at REAL NOT NULL,
                 PRIMARY KEY (comment_id, reader, emoji)
             );
             CREATE TABLE db_changes (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 tx_id INTEGER NOT NULL,
                 ts REAL NOT NULL,
                 actor TEXT,
                 label TEXT,
                 op TEXT NOT NULL,
                 tbl TEXT,
                 row_pk_json TEXT,
                 old_json TEXT,
                 new_json TEXT
             );
             CREATE TABLE change_tx_seq (
                 only_row INTEGER PRIMARY KEY CHECK (only_row = 1),
                 v INTEGER NOT NULL
             );
             INSERT INTO change_tx_seq(only_row, v) VALUES (1, 0);",
        )
        .unwrap();
        conn
    }

    fn last_change(conn: &Connection) -> (String, Option<String>, Option<String>, Option<String>) {
        conn.query_row(
            "SELECT op, tbl, old_json, new_json FROM db_changes ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap()
    }

    #[test]
    fn record_insert_captures_new_row() {
        let mut conn = setup();
        let tx = conn.transaction().unwrap();
        tx.execute(
            "INSERT INTO books(id, title, created_at) VALUES('b1', 'Dune', 1.0)",
            [],
        )
        .unwrap();
        let rec = ChangeRecorder::begin(&tx, Some("Mo".into()), Some("add_book".into())).unwrap();
        rec.record_insert("books", &[&"b1"]).unwrap();
        let (op, tbl, old, new) = last_change(&tx);
        tx.commit().unwrap();
        assert_eq!(op, "INSERT");
        assert_eq!(tbl.as_deref(), Some("books"));
        assert!(old.is_none());
        let new_str = new.expect("new_json present");
        assert!(new_str.contains("\"title\":\"Dune\""));
        assert!(new_str.contains("\"id\":\"b1\""));
        assert!(new_str.contains("\"deleted_at\":null"));
    }

    #[test]
    fn record_update_with_captures_old_and_new() {
        let mut conn = setup();
        let tx = conn.transaction().unwrap();
        tx.execute(
            "INSERT INTO books(id, title, created_at) VALUES('b1', 'Old Title', 1.0)",
            [],
        )
        .unwrap();
        let rec = ChangeRecorder::begin(&tx, None, None).unwrap();
        rec.record_update_with("books", &[&"b1"], |t| {
            t.execute("UPDATE books SET title = 'New Title' WHERE id = 'b1'", [])
                .map(|_| ())
        })
        .unwrap();
        let (op, _, old, new) = last_change(&tx);
        tx.commit().unwrap();
        assert_eq!(op, "UPDATE");
        let old = old.unwrap();
        let new = new.unwrap();
        assert!(old.contains("\"title\":\"Old Title\""));
        assert!(new.contains("\"title\":\"New Title\""));
    }

    #[test]
    fn record_delete_captures_old_and_removes_row() {
        let mut conn = setup();
        let tx = conn.transaction().unwrap();
        tx.execute(
            "INSERT INTO books(id, title, created_at) VALUES('b1', 'Gone', 1.0)",
            [],
        )
        .unwrap();
        let rec = ChangeRecorder::begin(&tx, None, None).unwrap();
        rec.record_delete("books", &[&"b1"]).unwrap();
        let n: i32 = tx
            .query_row("SELECT COUNT(*) FROM books WHERE id = 'b1'", [], |r| r.get(0))
            .unwrap();
        let (op, _, old, new) = last_change(&tx);
        tx.commit().unwrap();
        assert_eq!(n, 0);
        assert_eq!(op, "DELETE");
        assert!(new.is_none());
        assert!(old.unwrap().contains("\"title\":\"Gone\""));
    }

    #[test]
    fn composite_pk_round_trips_in_pk_json() {
        let mut conn = setup();
        let tx = conn.transaction().unwrap();
        tx.execute(
            "INSERT INTO comment_reactions(comment_id, reader, emoji, created_at)
             VALUES('c1', 'Mo', '👍', 1.0)",
            [],
        )
        .unwrap();
        let rec = ChangeRecorder::begin(&tx, None, None).unwrap();
        rec.record_insert("comment_reactions", &[&"c1", &"Mo", &"👍"])
            .unwrap();
        let pk: String = tx
            .query_row(
                "SELECT row_pk_json FROM db_changes ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        tx.commit().unwrap();
        assert!(pk.contains("\"comment_id\":\"c1\""));
        assert!(pk.contains("\"reader\":\"Mo\""));
        assert!(pk.contains("\"emoji\":\"👍\""));
    }

    #[test]
    fn multiple_records_share_tx_id_but_distinct_ids() {
        let mut conn = setup();
        let tx = conn.transaction().unwrap();
        tx.execute(
            "INSERT INTO books(id, title, created_at) VALUES('b1', 'A', 1.0)",
            [],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO books(id, title, created_at) VALUES('b2', 'B', 1.0)",
            [],
        )
        .unwrap();
        let rec = ChangeRecorder::begin(&tx, None, None).unwrap();
        let txid = rec.tx_id();
        rec.record_insert("books", &[&"b1"]).unwrap();
        rec.record_insert("books", &[&"b2"]).unwrap();
        let rows: Vec<(i64, i64)> = {
            let mut stmt = tx
                .prepare("SELECT id, tx_id FROM db_changes ORDER BY id")
                .unwrap();
            let mapped = stmt
                .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
                .unwrap();
            mapped.collect::<Result<Vec<_>, _>>().unwrap()
        };
        tx.commit().unwrap();
        assert_eq!(rows.len(), 2);
        assert_ne!(rows[0].0, rows[1].0);
        assert_eq!(rows[0].1, txid);
        assert_eq!(rows[1].1, txid);
    }

    #[test]
    fn update_with_when_row_didnt_exist_logs_as_insert() {
        let mut conn = setup();
        let tx = conn.transaction().unwrap();
        let rec = ChangeRecorder::begin(&tx, None, None).unwrap();
        rec.record_update_with("books", &[&"b9"], |t| {
            t.execute(
                "INSERT INTO books(id, title, created_at) VALUES('b9', 'Fresh', 1.0)",
                [],
            )
            .map(|_| ())
        })
        .unwrap();
        let (op, _, old, new) = last_change(&tx);
        tx.commit().unwrap();
        assert_eq!(op, "INSERT");
        assert!(old.is_none());
        assert!(new.unwrap().contains("\"title\":\"Fresh\""));
    }

    #[test]
    fn nullable_columns_serialize_as_null() {
        let mut conn = setup();
        let tx = conn.transaction().unwrap();
        tx.execute(
            "INSERT INTO books(id, title, created_at) VALUES('b1', 'X', 1.0)",
            [],
        )
        .unwrap();
        let rec = ChangeRecorder::begin(&tx, None, None).unwrap();
        rec.record_insert("books", &[&"b1"]).unwrap();
        let new: String = tx
            .query_row(
                "SELECT new_json FROM db_changes ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        tx.commit().unwrap();
        for col in ["author", "isbn", "deleted_at", "toc_json"] {
            assert!(
                new.contains(&format!("\"{col}\":null")),
                "expected {col}:null in {new}"
            );
        }
    }

    #[test]
    fn record_event_writes_high_level_row() {
        let mut conn = setup();
        let tx = conn.transaction().unwrap();
        let rec = ChangeRecorder::begin(&tx, Some("Mo".into()), Some("test".into())).unwrap();
        rec.record_event(
            "restore_point",
            None,
            Some("{\"tx_id_target\":7}".into()),
            Some("{\"reason\":\"manual\"}".into()),
        )
        .unwrap();
        let (op, tbl, old, new) = last_change(&tx);
        let pk: Option<String> = tx
            .query_row(
                "SELECT row_pk_json FROM db_changes ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        tx.commit().unwrap();
        assert_eq!(op, "restore_point");
        assert!(tbl.is_none());
        assert!(pk.unwrap().contains("\"tx_id_target\":7"));
        assert!(old.unwrap().contains("\"reason\":\"manual\""));
        assert!(new.is_none());
    }
}
