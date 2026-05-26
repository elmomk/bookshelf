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
    let safeties: Vec<(String, f64)> = list_ids()
        .into_iter()
        .filter(|id| source_of(id) == Source::Safety)
        .filter_map(|id| ts_of(&id).map(|ts| (id, ts)))
        .collect();
    for id in safety_drop_ids(now, &safeties) {
        let _ = delete(&id);
    }
}

fn prune_manual() {
    let mut manuals: Vec<(String, f64)> = list_ids()
        .into_iter()
        .filter(|id| source_of(id) == Source::Manual)
        .filter_map(|id| ts_of(&id).map(|ts| (id, ts)))
        .collect();
    manuals.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    for id in manual_drop_ids(&manuals, MANUAL_SOFT_CAP) {
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
    autos.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let keep = gfs_keep_ids(now, &autos);
    for (id, _) in &autos {
        if !keep.contains(id) {
            let _ = delete(id);
        }
    }
}

// --- Pure helpers — testable, no filesystem ---------------------------------

/// IDs older than the 30-day cutoff.
fn safety_drop_ids(now_ms: f64, safeties: &[(String, f64)]) -> Vec<String> {
    let cutoff = now_ms - SAFETY_MAX_AGE_DAYS * DAY_MS;
    safeties
        .iter()
        .filter(|(_, ts)| *ts < cutoff)
        .map(|(id, _)| id.clone())
        .collect()
}

/// IDs to drop given a list already sorted **newest first** and a soft cap.
fn manual_drop_ids(manuals_sorted: &[(String, f64)], soft_cap: usize) -> Vec<String> {
    if manuals_sorted.len() <= soft_cap {
        return vec![];
    }
    manuals_sorted
        .iter()
        .skip(soft_cap)
        .map(|(id, _)| id.clone())
        .collect()
}

/// Compute the GFS-kept set from a list of `(id, ts_ms)` **sorted newest
/// first**. Keep all of last `AUTO_DAILY_DAYS` + 1 per ISO-ish week for the
/// next `AUTO_WEEKLY_DAYS-AUTO_DAILY_DAYS` window + 1 per calendar month for
/// the next `AUTO_MONTHLY_DAYS-AUTO_WEEKLY_DAYS` window. Older → drop.
fn gfs_keep_ids(
    now_ms: f64,
    autos_sorted: &[(String, f64)],
) -> std::collections::HashSet<String> {
    let daily_cutoff = now_ms - AUTO_DAILY_DAYS * DAY_MS;
    let weekly_cutoff = now_ms - AUTO_WEEKLY_DAYS * DAY_MS;
    let monthly_cutoff = now_ms - AUTO_MONTHLY_DAYS * DAY_MS;

    let mut keep: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut seen_weeks: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let mut seen_months: std::collections::HashSet<i64> = std::collections::HashSet::new();

    for (id, ts) in autos_sorted {
        if *ts >= daily_cutoff {
            keep.insert(id.clone());
        } else if *ts >= weekly_cutoff {
            // Week bucket: ms → whole days → integer-divide by 7. Iteration
            // is newest-first, so the first hit per bucket is the newest.
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
    }
    keep
}

// ---------------------------------------------------------------------------
// Tests for the pure retention helpers
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(now_ms: f64, age_days: f64) -> f64 {
        now_ms - age_days * DAY_MS
    }
    fn id(age_days: f64) -> String {
        format!("auto-{}.db", (age_days * 1000.0) as i64) // unique per age
    }
    fn pair(now_ms: f64, age_days: f64) -> (String, f64) {
        (id(age_days), ts(now_ms, age_days))
    }
    /// Build a list of `n_days` auto-snapshots (one at each integer day age 0..n_days-1),
    /// newest first.
    fn daily_set(now_ms: f64, n_days: usize) -> Vec<(String, f64)> {
        (0..n_days).map(|d| pair(now_ms, d as f64)).collect()
    }

    // ---- safety ----

    #[test]
    fn safety_drops_only_older_than_30_days() {
        // Use a real-ish "now" so chrono's month bucketing wouldn't trip up
        // an absolute-time-based test (irrelevant here but consistent).
        let now = 1_780_000_000_000.0;
        let items = vec![
            pair(now, 1.0),
            pair(now, 10.0),
            pair(now, 20.0),
            pair(now, 29.0),
            pair(now, 31.0),
            pair(now, 60.0),
        ];
        let dropped = safety_drop_ids(now, &items);
        assert_eq!(dropped.len(), 2);
        assert!(dropped.contains(&id(31.0)));
        assert!(dropped.contains(&id(60.0)));
    }

    #[test]
    fn safety_keeps_all_when_all_fresh() {
        let now = 1_780_000_000_000.0;
        let items: Vec<_> = (0..5).map(|d| pair(now, d as f64)).collect();
        assert!(safety_drop_ids(now, &items).is_empty());
    }

    // ---- manual ----

    #[test]
    fn manual_cap_keeps_first_n() {
        let now = 1_780_000_000_000.0;
        let items: Vec<_> = (0..60).map(|d| pair(now, d as f64)).collect(); // sorted newest first
        let dropped = manual_drop_ids(&items, 50);
        assert_eq!(dropped.len(), 10);
        // The 10 dropped should be the oldest (highest age = end of the list).
        for d in 50..60 {
            assert!(dropped.contains(&id(d as f64)));
        }
    }

    #[test]
    fn manual_under_cap_drops_nothing() {
        let now = 1_780_000_000_000.0;
        let items: Vec<_> = (0..20).map(|d| pair(now, d as f64)).collect();
        assert!(manual_drop_ids(&items, 50).is_empty());
    }

    // ---- gfs ----

    #[test]
    fn gfs_keeps_all_within_daily_window() {
        let now = 1_780_000_000_000.0;
        let items = daily_set(now, 7);
        let keep = gfs_keep_ids(now, &items);
        assert_eq!(keep.len(), 7);
        for (id, _) in &items {
            assert!(keep.contains(id), "expected {id} kept");
        }
    }

    #[test]
    fn gfs_drops_beyond_400_days() {
        let now = 1_780_000_000_000.0;
        let items = vec![pair(now, 0.0), pair(now, 500.0), pair(now, 1000.0)];
        let keep = gfs_keep_ids(now, &items);
        assert!(keep.contains(&id(0.0)));
        assert!(!keep.contains(&id(500.0)));
        assert!(!keep.contains(&id(1000.0)));
    }

    #[test]
    fn gfs_long_horizon_caps_at_about_23() {
        // 365 daily snapshots → ~7 daily + ~4 weekly + ~12 monthly. Bucket
        // alignment can shift the count by a couple in either direction.
        let now = 1_780_000_000_000.0;
        let items = daily_set(now, 365);
        let keep = gfs_keep_ids(now, &items);
        assert!(
            (20..=27).contains(&keep.len()),
            "expected ~23 kept, got {} from 365 days",
            keep.len()
        );

        // Sanity: every one of the last 7 days is kept (daily window).
        for d in 0..7 {
            assert!(
                keep.contains(&id(d as f64)),
                "missing daily {d}d ago in keep set"
            );
        }
    }

    #[test]
    fn gfs_weekly_window_dedupes_per_week_picking_newest() {
        let now = 1_780_000_000_000.0;
        // Three snapshots all inside the same calendar week, ages 10/11/12d
        // (outside daily window of 7d, inside weekly window of 35d).
        let items = vec![pair(now, 10.0), pair(now, 11.0), pair(now, 12.0)];
        let keep = gfs_keep_ids(now, &items);
        // Same week-bucket → only one kept; iteration is newest-first so the
        // 10-day-old wins.
        assert_eq!(keep.len(), 1);
        assert!(keep.contains(&id(10.0)));
    }

    #[test]
    fn gfs_empty_input_returns_empty_set() {
        let now = 1_780_000_000_000.0;
        assert!(gfs_keep_ids(now, &[]).is_empty());
    }
}
