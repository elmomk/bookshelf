//! Filesystem-side snapshot management.
//!
//! Each snapshot is a standalone SQLite file produced by `VACUUM INTO`, the
//! engine's native consistent-online-backup mechanism (safe with WAL).
//! Snapshots live next to the live DB inside the Docker volume so the server
//! can always read them back.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

const SNAP_DIR: &str = "snapshots";
const PREFIX: &str = "snap-";
const SUFFIX: &str = ".db";

/// `<dir of live db>/snapshots/`. Created if missing.
pub fn snapshots_dir() -> PathBuf {
    let db_path =
        std::env::var("DATABASE_PATH").unwrap_or_else(|_| "bookclub.db".to_string());
    let parent = Path::new(&db_path).parent().unwrap_or(Path::new("."));
    let dir = parent.join(SNAP_DIR);
    let _ = fs::create_dir_all(&dir);
    dir
}

/// Build the absolute path for a snapshot id (e.g. `snap-1716429327000.db`).
/// Returns `None` if the id contains anything dodgy or the file is missing.
pub fn path_for(id: &str) -> Option<PathBuf> {
    if !id.starts_with(PREFIX) || !id.ends_with(SUFFIX) {
        return None;
    }
    if id.contains('/') || id.contains('\\') || id.contains("..") {
        return None;
    }
    let p = snapshots_dir().join(id);
    if p.is_file() {
        Some(p)
    } else {
        None
    }
}

/// Capture the live database into a new snapshot file. Returns the filename id.
pub fn create(conn: &Connection) -> Result<String, String> {
    let ts = chrono::Utc::now().timestamp_millis();
    let dir = snapshots_dir();
    let id = format!("{PREFIX}{ts}{SUFFIX}");
    let p = dir.join(&id);
    // VACUUM INTO is single-statement, atomic, and WAL-safe.
    conn.execute(
        &format!("VACUUM INTO '{}'", p.display().to_string().replace('\'', "''")),
        [],
    )
    .map_err(|e| e.to_string())?;
    Ok(id)
}

/// Snapshot filename ids sorted newest first.
pub fn list_ids() -> Vec<String> {
    let mut out = vec![];
    if let Ok(rd) = fs::read_dir(snapshots_dir()) {
        for entry in rd.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with(PREFIX) && name.ends_with(SUFFIX) {
                    out.push(name.to_string());
                }
            }
        }
    }
    out.sort_by(|a, b| b.cmp(a));
    out
}

/// Parse the unix-millis timestamp embedded in the snapshot id.
pub fn ts_of(id: &str) -> Option<f64> {
    id.strip_prefix(PREFIX)?
        .strip_suffix(SUFFIX)?
        .parse::<f64>()
        .ok()
}

/// Unix-millis timestamp of the newest snapshot, or `None` if there are none.
pub fn ts_of_newest() -> Option<f64> {
    list_ids().into_iter().filter_map(|id| ts_of(&id)).next()
}

/// File size in bytes (0 if unreadable).
pub fn size_of(id: &str) -> u64 {
    path_for(id)
        .and_then(|p| fs::metadata(&p).ok())
        .map(|m| m.len())
        .unwrap_or(0)
}

/// Delete a snapshot file by id. Errors are bubbled up.
pub fn delete(id: &str) -> Result<(), String> {
    let p = path_for(id).ok_or_else(|| "Snapshot not found".to_string())?;
    fs::remove_file(p).map_err(|e| e.to_string())
}

/// Quick counts for a snapshot file. Opens read-only.
pub fn counts(id: &str) -> Result<(i32, i32, i32), String> {
    let p = path_for(id).ok_or_else(|| "Snapshot not found".to_string())?;
    let c = Connection::open_with_flags(&p, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    // Older snapshots may not have `deleted_at`; count everything in that case.
    let has_deleted_books = table_has_col(&c, "books", "deleted_at");
    let books: i32 = c
        .query_row(
            if has_deleted_books {
                "SELECT COUNT(*) FROM books WHERE deleted_at IS NULL"
            } else {
                "SELECT COUNT(*) FROM books"
            },
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let has_deleted_comments = table_has_col(&c, "book_comments", "deleted_at");
    let comments: i32 = c
        .query_row(
            if has_deleted_comments {
                "SELECT COUNT(*) FROM book_comments WHERE deleted_at IS NULL"
            } else {
                "SELECT COUNT(*) FROM book_comments"
            },
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let reactions: i32 = c
        .query_row("SELECT COUNT(*) FROM comment_reactions", [], |r| r.get(0))
        .unwrap_or(0);
    Ok((books, comments, reactions))
}

/// List the (id, title, author) of every book present in a snapshot.
pub fn list_books_in(id: &str) -> Result<Vec<(String, String, Option<String>)>, String> {
    let p = path_for(id).ok_or_else(|| "Snapshot not found".to_string())?;
    let c = Connection::open_with_flags(&p, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    let has_deleted = table_has_col(&c, "books", "deleted_at");
    let sql = if has_deleted {
        "SELECT id, title, author FROM books WHERE deleted_at IS NULL ORDER BY title"
    } else {
        "SELECT id, title, author FROM books ORDER BY title"
    };
    let mut stmt = c.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(|e| e.to_string())?;
    let mut out = vec![];
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

fn table_has_col(c: &Connection, table: &str, col: &str) -> bool {
    let mut stmt = match c.prepare(&format!("PRAGMA table_info({table})")) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let rows = match stmt.query_map([], |r| r.get::<_, String>(1)) {
        Ok(r) => r,
        Err(_) => return false,
    };
    for r in rows {
        if let Ok(name) = r {
            if name == col {
                return true;
            }
        }
    }
    false
}

/// Columns the live and snapshot DBs agree on for a table, preserving live order.
pub fn common_cols(
    conn: &Connection,
    snap_alias: &str,
    table: &str,
) -> Result<Vec<String>, rusqlite::Error> {
    let live: Vec<String> = conn
        .prepare(&format!("PRAGMA table_info({table})"))?
        .query_map([], |r| r.get::<_, String>(1))?
        .filter_map(|r| r.ok())
        .collect();
    let snap: Vec<String> = conn
        .prepare(&format!("PRAGMA \"{snap_alias}\".table_info({table})"))?
        .query_map([], |r| r.get::<_, String>(1))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(live.into_iter().filter(|c| snap.contains(c)).collect())
}
