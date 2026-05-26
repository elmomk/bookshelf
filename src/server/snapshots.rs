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
const SUFFIX: &str = ".db";

// Source-tagged prefixes drive retention policy.
const PREFIX_AUTO: &str = "auto-"; // daily background snapshot → GFS prune
const PREFIX_SAFETY: &str = "safety-"; // pre-restore safety net → 30-day cutoff
const PREFIX_MANUAL: &str = "manual-"; // user 📸 button → soft cap at 50
/// Legacy: snapshots created before v0.1.43 used `snap-`. We treat them as
/// `manual` (never auto-pruned) so old files aren't surprise-deleted.
const PREFIX_LEGACY: &str = "snap-";

const ALL_PREFIXES: [&str; 4] = [PREFIX_AUTO, PREFIX_SAFETY, PREFIX_MANUAL, PREFIX_LEGACY];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Auto,
    Safety,
    Manual,
}

pub fn source_of(id: &str) -> Source {
    if id.starts_with(PREFIX_AUTO) {
        Source::Auto
    } else if id.starts_with(PREFIX_SAFETY) {
        Source::Safety
    } else {
        // Both `manual-` and the legacy `snap-` map to Manual.
        Source::Manual
    }
}

/// `<dir of live db>/snapshots/`. Created if missing.
pub fn snapshots_dir() -> PathBuf {
    let db_path =
        std::env::var("DATABASE_PATH").unwrap_or_else(|_| "bookclub.db".to_string());
    let parent = Path::new(&db_path).parent().unwrap_or(Path::new("."));
    let dir = parent.join(SNAP_DIR);
    let _ = fs::create_dir_all(&dir);
    dir
}

/// Build the absolute path for a snapshot id. Returns `None` if the id
/// doesn't match a known prefix or the file is missing.
pub fn path_for(id: &str) -> Option<PathBuf> {
    if !ALL_PREFIXES.iter().any(|p| id.starts_with(p)) || !id.ends_with(SUFFIX) {
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

fn create_with_prefix(conn: &Connection, prefix: &str) -> Result<String, String> {
    let ts = chrono::Utc::now().timestamp_millis();
    let id = format!("{prefix}{ts}{SUFFIX}");
    let p = snapshots_dir().join(&id);
    conn.execute(
        &format!(
            "VACUUM INTO '{}'",
            p.display().to_string().replace('\'', "''")
        ),
        [],
    )
    .map_err(|e| e.to_string())?;
    Ok(id)
}

/// User-triggered snapshot (Settings → 📸). Subject to the manual soft-cap.
pub fn create_manual(conn: &Connection) -> Result<String, String> {
    create_with_prefix(conn, PREFIX_MANUAL)
}

/// Pre-restore safety net. Auto-pruned after 30 days.
pub fn create_safety(conn: &Connection) -> Result<String, String> {
    create_with_prefix(conn, PREFIX_SAFETY)
}

/// Daily background snapshot. Subject to GFS retention.
pub fn create_auto(conn: &Connection) -> Result<String, String> {
    create_with_prefix(conn, PREFIX_AUTO)
}

/// Snapshot filename ids sorted newest first (across all sources).
pub fn list_ids() -> Vec<String> {
    let mut out = vec![];
    if let Ok(rd) = fs::read_dir(snapshots_dir()) {
        for entry in rd.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if ALL_PREFIXES.iter().any(|p| name.starts_with(p))
                    && name.ends_with(SUFFIX)
                {
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
    for p in ALL_PREFIXES {
        if let Some(rest) = id.strip_prefix(p) {
            return rest.strip_suffix(SUFFIX)?.parse::<f64>().ok();
        }
    }
    None
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

// ---------------------------------------------------------------------------
// Retention
// ---------------------------------------------------------------------------

const DAY_MS: f64 = 86_400_000.0;
const MANUAL_SOFT_CAP: usize = 50;
const SAFETY_MAX_AGE_DAYS: f64 = 30.0;
// GFS thresholds for auto.
const AUTO_DAILY_DAYS: f64 = 7.0;
const AUTO_WEEKLY_DAYS: f64 = 35.0; // weekly kept between day 7 and day 35
const AUTO_MONTHLY_DAYS: f64 = 400.0; // monthly kept up to ~13 months

/// Best-effort retention pass: auto → GFS, safety → 30-day cutoff,
/// manual → soft cap at 50. Errors are silently swallowed (the next call
/// will retry).
pub fn prune() {
    prune_safety();
    prune_manual();
    prune_auto_gfs();
}

fn prune_safety() {
    let now = chrono::Utc::now().timestamp_millis() as f64;
    let cutoff = now - SAFETY_MAX_AGE_DAYS * DAY_MS;
    for id in list_ids() {
        if source_of(&id) == Source::Safety {
            if let Some(ts) = ts_of(&id) {
                if ts < cutoff {
                    let _ = delete(&id);
                }
            }
        }
    }
}

fn prune_manual() {
    let mut manuals: Vec<(String, f64)> = list_ids()
        .into_iter()
        .filter(|id| source_of(id) == Source::Manual)
        .filter_map(|id| ts_of(&id).map(|ts| (id, ts)))
        .collect();
    if manuals.len() <= MANUAL_SOFT_CAP {
        return;
    }
    // Newest first; drop the tail.
    manuals.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    for (id, _) in manuals.into_iter().skip(MANUAL_SOFT_CAP) {
        let _ = delete(&id);
    }
}

fn prune_auto_gfs() {
    let now = chrono::Utc::now().timestamp_millis() as f64;
    let mut autos: Vec<(String, f64)> = list_ids()
        .into_iter()
        .filter(|id| source_of(id) == Source::Auto)
        .filter_map(|id| ts_of(&id).map(|ts| (id, ts)))
        .collect();
    if autos.is_empty() {
        return;
    }
    // Newest first.
    autos.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let daily_cutoff = now - AUTO_DAILY_DAYS * DAY_MS;
    let weekly_cutoff = now - AUTO_WEEKLY_DAYS * DAY_MS;
    let monthly_cutoff = now - AUTO_MONTHLY_DAYS * DAY_MS;

    let mut keep: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut seen_weeks: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let mut seen_months: std::collections::HashSet<i64> = std::collections::HashSet::new();

    for (id, ts) in &autos {
        if *ts >= daily_cutoff {
            keep.insert(id.clone());
        } else if *ts >= weekly_cutoff {
            // Bucket by ISO week: convert ms → days → integer-divide by 7.
            let week = ((*ts / DAY_MS) / 7.0).floor() as i64;
            if seen_weeks.insert(week) {
                keep.insert(id.clone());
            }
        } else if *ts >= monthly_cutoff {
            let bucket = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(*ts as i64)
                .map(|dt| {
                    use chrono::Datelike;
                    (dt.year() as i64) * 100 + dt.month() as i64
                })
                .unwrap_or(0);
            if seen_months.insert(bucket) {
                keep.insert(id.clone());
            }
        }
        // else: older than monthly_cutoff → drop
    }

    for (id, _) in &autos {
        if !keep.contains(id) {
            let _ = delete(id);
        }
    }
}
