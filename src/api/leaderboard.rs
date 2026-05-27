use dioxus::prelude::*;

use crate::models::{LeaderboardEntry, LeaderboardWindow};

// Composite-score weights. Tuned so each metric matters but no single one
// dominates: a 200-page novel earns 200 + 200 = 400 pts; thirteen comments
// also earns ~390 pts; eighty reactions earns 400 pts. Kept here (not in a
// config table) because changing them is a deliberate product decision.
pub const PTS_PER_PAGE: i32 = 1;
pub const PTS_PER_REACTION: i32 = 5;
pub const PTS_PER_COMMENT: i32 = 30;
pub const PTS_PER_BOOK: i32 = 200;

/// Pure composite-score formula — extracted so the unit test pins the
/// invariants independent of any DB state.
pub fn compute_score(pages: i32, books: i32, comments: i32, reactions: i32) -> i32 {
    pages * PTS_PER_PAGE
        + reactions * PTS_PER_REACTION
        + comments * PTS_PER_COMMENT
        + books * PTS_PER_BOOK
}

#[server(headers: axum::http::HeaderMap)]
pub async fn get_leaderboard(
    window: LeaderboardWindow,
) -> Result<Vec<LeaderboardEntry>, ServerFnError> {
    use crate::server::{auth, db};

    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    let conn = db::pool().get().map_err(|e| ServerFnError::new(e.to_string()))?;

    let now_ms = chrono::Utc::now().timestamp_millis();
    let rows = match window.cutoff_ms(now_ms) {
        Some(cutoff) => aggregate_windowed(&conn, cutoff)
            .map_err(|e| ServerFnError::new(e.to_string()))?,
        None => aggregate_all_time(&conn)
            .map_err(|e| ServerFnError::new(e.to_string()))?,
    };

    Ok(rows)
}

#[cfg(not(target_arch = "wasm32"))]
use rusqlite::Connection;
#[cfg(not(target_arch = "wasm32"))]
use std::collections::HashMap;

/// `&mut HashMap`-backed row accessor. Closure form runs into lifetime
/// inference quirks (two ref inputs, one ref output) — a freestanding
/// function with an explicit lifetime sidesteps that cleanly.
#[cfg(not(target_arch = "wasm32"))]
fn row<'a>(
    acc: &'a mut HashMap<String, LeaderboardEntry>,
    name: String,
) -> &'a mut LeaderboardEntry {
    acc.entry(name.clone()).or_insert_with(move || LeaderboardEntry {
        reader: name,
        pages_read: 0,
        books_finished: 0,
        comments_posted: 0,
        reactions_given: 0,
        score: 0,
    })
}

/// Sum per-update deltas from `db_changes` since `cutoff_ms`. Negative deltas
/// (a reader backtracking) are honest and count against them; this is the
/// price of a delta-based metric and keeps the leaderboard truthful.
#[cfg(not(target_arch = "wasm32"))]
pub fn aggregate_windowed(
    conn: &Connection,
    cutoff_ms: i64,
) -> rusqlite::Result<Vec<LeaderboardEntry>> {
    let cutoff = cutoff_ms as f64;
    let mut acc: HashMap<String, LeaderboardEntry> = HashMap::new();

    // Pages read: sum of (new.current_page - old.current_page) across UPDATEs.
    let mut stmt = conn.prepare(
        "SELECT actor, SUM(
             COALESCE(CAST(json_extract(new_json, '$.current_page') AS INTEGER), 0)
           - COALESCE(CAST(json_extract(old_json, '$.current_page') AS INTEGER), 0)
         ) AS delta
         FROM db_changes
         WHERE tbl = 'reading_progress' AND op = 'UPDATE'
           AND ts > ?1 AND actor IS NOT NULL
         GROUP BY actor",
    )?;
    let pages = stmt
        .query_map(rusqlite::params![cutoff], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0)))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (actor, delta) in pages {
        row(&mut acc, actor).pages_read = delta as i32;
    }

    // Books finished: COUNT of UPDATEs whose status flipped to 'finished'.
    let mut stmt = conn.prepare(
        "SELECT actor, COUNT(*) FROM db_changes
         WHERE tbl = 'reading_progress' AND op = 'UPDATE'
           AND ts > ?1 AND actor IS NOT NULL
           AND json_extract(new_json, '$.status') = 'finished'
           AND COALESCE(json_extract(old_json, '$.status'), '') != 'finished'
         GROUP BY actor",
    )?;
    let books = stmt
        .query_map(rusqlite::params![cutoff], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (actor, n) in books {
        row(&mut acc, actor).books_finished = n as i32;
    }

    // Comments posted: INSERTs into book_comments.
    let mut stmt = conn.prepare(
        "SELECT actor, COUNT(*) FROM db_changes
         WHERE tbl = 'book_comments' AND op = 'INSERT'
           AND ts > ?1 AND actor IS NOT NULL
         GROUP BY actor",
    )?;
    let comments = stmt
        .query_map(rusqlite::params![cutoff], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (actor, n) in comments {
        row(&mut acc, actor).comments_posted = n as i32;
    }

    // Reactions given: INSERTs into comment_reactions. (Toggling off shows up
    // as a DELETE which we deliberately don't count — only positive acts.)
    let mut stmt = conn.prepare(
        "SELECT actor, COUNT(*) FROM db_changes
         WHERE tbl = 'comment_reactions' AND op = 'INSERT'
           AND ts > ?1 AND actor IS NOT NULL
         GROUP BY actor",
    )?;
    let reactions = stmt
        .query_map(rusqlite::params![cutoff], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (actor, n) in reactions {
        row(&mut acc, actor).reactions_given = n as i32;
    }

    Ok(finalize(acc))
}

/// All-time: the change log only has data since v0.1.37, so for the full
/// history we read the raw user-data tables. Pages-read is the sum of
/// current_page across the reader's progress rows — *approximate* (it tracks
/// the reader's current position, not lifetime turned-pages), but it's the
/// best signal available from the underlying data.
#[cfg(not(target_arch = "wasm32"))]
pub fn aggregate_all_time(conn: &Connection) -> rusqlite::Result<Vec<LeaderboardEntry>> {
    let mut acc: HashMap<String, LeaderboardEntry> = HashMap::new();

    // Pages: where the reader is right now across all started/finished books.
    let mut stmt = conn.prepare(
        "SELECT reader, SUM(COALESCE(current_page, 0))
         FROM reading_progress
         WHERE status != 'to_read'
         GROUP BY reader",
    )?;
    for r in stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0)))
    })? {
        let (reader, pages) = r?;
        row(&mut acc, reader).pages_read = pages as i32;
    }

    // Books finished.
    let mut stmt = conn.prepare(
        "SELECT reader, COUNT(*) FROM reading_progress
         WHERE status = 'finished' GROUP BY reader",
    )?;
    for r in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
        let (reader, n) = r?;
        row(&mut acc, reader).books_finished = n as i32;
    }

    // Comments (live, not soft-deleted).
    let mut stmt = conn.prepare(
        "SELECT author, COUNT(*) FROM book_comments
         WHERE deleted_at IS NULL GROUP BY author",
    )?;
    for r in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
        let (author, n) = r?;
        row(&mut acc, author).comments_posted = n as i32;
    }

    // Reactions.
    let mut stmt = conn.prepare(
        "SELECT reader, COUNT(*) FROM comment_reactions GROUP BY reader",
    )?;
    for r in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
        let (reader, n) = r?;
        row(&mut acc, reader).reactions_given = n as i32;
    }

    Ok(finalize(acc))
}

/// Stamp the composite score on each row and sort by it (desc), with reader
/// name as a stable tiebreaker.
#[cfg(not(target_arch = "wasm32"))]
fn finalize(map: HashMap<String, LeaderboardEntry>) -> Vec<LeaderboardEntry> {
    let mut rows: Vec<LeaderboardEntry> = map.into_values().collect();
    for r in &mut rows {
        r.score = compute_score(
            r.pages_read,
            r.books_finished,
            r.comments_posted,
            r.reactions_given,
        );
    }
    rows.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.reader.cmp(&b.reader)));
    rows
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn compute_score_weights_match_constants() {
        // 0 pages, 0 books, 0 comments, 0 reactions → 0
        assert_eq!(compute_score(0, 0, 0, 0), 0);
        // Each metric in isolation evaluates to its weight.
        assert_eq!(compute_score(1, 0, 0, 0), PTS_PER_PAGE);
        assert_eq!(compute_score(0, 1, 0, 0), PTS_PER_BOOK);
        assert_eq!(compute_score(0, 0, 1, 0), PTS_PER_COMMENT);
        assert_eq!(compute_score(0, 0, 0, 1), PTS_PER_REACTION);
        // Linear combination.
        assert_eq!(
            compute_score(50, 2, 3, 4),
            50 * PTS_PER_PAGE
                + 2 * PTS_PER_BOOK
                + 3 * PTS_PER_COMMENT
                + 4 * PTS_PER_REACTION
        );
    }

    fn setup() -> Connection {
        // Minimal schema covering the tables both aggregators read from.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE reading_progress (
                 id TEXT PRIMARY KEY,
                 book_id TEXT NOT NULL,
                 reader TEXT NOT NULL,
                 current_page INTEGER,
                 current_chapter INTEGER,
                 status TEXT NOT NULL DEFAULT 'to_read',
                 updated_at REAL NOT NULL
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
             );",
        )
        .unwrap();
        conn
    }

    fn log(
        conn: &Connection,
        ts: f64,
        actor: &str,
        op: &str,
        tbl: &str,
        old_json: Option<&str>,
        new_json: Option<&str>,
    ) {
        conn.execute(
            "INSERT INTO db_changes(tx_id, ts, actor, op, tbl, old_json, new_json)
             VALUES(1, ?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![ts, actor, op, tbl, old_json, new_json],
        )
        .unwrap();
    }

    #[test]
    fn windowed_aggregator_counts_per_reader() {
        let conn = setup();
        // Mo: +50 pages, finishes a book, posts 2 comments, 3 reactions.
        log(
            &conn,
            100.0,
            "Mo",
            "UPDATE",
            "reading_progress",
            Some(r#"{"current_page":10,"status":"reading"}"#),
            Some(r#"{"current_page":60,"status":"reading"}"#),
        );
        log(
            &conn,
            110.0,
            "Mo",
            "UPDATE",
            "reading_progress",
            Some(r#"{"current_page":60,"status":"reading"}"#),
            Some(r#"{"current_page":60,"status":"finished"}"#),
        );
        log(&conn, 120.0, "Mo", "INSERT", "book_comments", None, Some("{}"));
        log(&conn, 121.0, "Mo", "INSERT", "book_comments", None, Some("{}"));
        log(&conn, 122.0, "Mo", "INSERT", "comment_reactions", None, Some("{}"));
        log(&conn, 123.0, "Mo", "INSERT", "comment_reactions", None, Some("{}"));
        log(&conn, 124.0, "Mo", "INSERT", "comment_reactions", None, Some("{}"));

        // Pei: +10 pages only.
        log(
            &conn,
            130.0,
            "Pei",
            "UPDATE",
            "reading_progress",
            Some(r#"{"current_page":0,"status":"reading"}"#),
            Some(r#"{"current_page":10,"status":"reading"}"#),
        );

        // Pre-cutoff row that must be ignored.
        log(
            &conn,
            10.0,
            "Mo",
            "UPDATE",
            "reading_progress",
            Some(r#"{"current_page":0,"status":"reading"}"#),
            Some(r#"{"current_page":999,"status":"reading"}"#),
        );

        let rows = aggregate_windowed(&conn, 50).unwrap();
        assert_eq!(rows.len(), 2, "two readers in window");
        // Top of the board is the higher-scorer.
        assert_eq!(rows[0].reader, "Mo");
        assert_eq!(rows[0].pages_read, 50);
        assert_eq!(rows[0].books_finished, 1);
        assert_eq!(rows[0].comments_posted, 2);
        assert_eq!(rows[0].reactions_given, 3);
        assert_eq!(
            rows[0].score,
            50 + PTS_PER_BOOK + 2 * PTS_PER_COMMENT + 3 * PTS_PER_REACTION
        );
        assert_eq!(rows[1].reader, "Pei");
        assert_eq!(rows[1].pages_read, 10);
        assert_eq!(rows[1].score, 10);
    }

    #[test]
    fn all_time_aggregator_reads_raw_tables() {
        let conn = setup();
        conn.execute_batch(
            "INSERT INTO reading_progress(id, book_id, reader, current_page, status, updated_at)
             VALUES
               ('p1', 'b1', 'Mo', 120, 'finished', 1.0),
               ('p2', 'b2', 'Mo', 40,  'reading',  1.0),
               ('p3', 'b1', 'Pei', 0,  'to_read',  1.0);
             INSERT INTO book_comments(id, book_id, author, body, created_at)
             VALUES
               ('c1','b1','Mo','hi',1.0),
               ('c2','b1','Mo','hi',1.0),
               ('c3','b1','Pei','hi',1.0);
             INSERT INTO comment_reactions(comment_id, reader, emoji, created_at)
             VALUES
               ('c1','Pei','👍',1.0),
               ('c2','Pei','🔥',1.0);",
        )
        .unwrap();

        let rows = aggregate_all_time(&conn).unwrap();
        let mo = rows.iter().find(|r| r.reader == "Mo").unwrap();
        let pei = rows.iter().find(|r| r.reader == "Pei").unwrap();
        // Mo: 120+40 pages, 1 finish, 2 comments, 0 reactions.
        assert_eq!(mo.pages_read, 160);
        assert_eq!(mo.books_finished, 1);
        assert_eq!(mo.comments_posted, 2);
        assert_eq!(mo.reactions_given, 0);
        // Pei: 0 pages (to_read filtered), 0 finishes, 1 comment, 2 reactions.
        assert_eq!(pei.pages_read, 0);
        assert_eq!(pei.books_finished, 0);
        assert_eq!(pei.comments_posted, 1);
        assert_eq!(pei.reactions_given, 2);
        // Ordering: Mo (260) > Pei (40).
        assert_eq!(rows[0].reader, "Mo");
        assert_eq!(rows[1].reader, "Pei");
    }

    #[test]
    fn windowed_skips_soft_deleted_comments_via_op_filter() {
        // Soft-deletes of comments show up as UPDATEs (deleted_at set), so
        // they don't inflate the comments_posted count which only watches
        // INSERTs. This pins that invariant.
        let conn = setup();
        log(&conn, 100.0, "Mo", "INSERT", "book_comments", None, Some("{}"));
        log(
            &conn,
            110.0,
            "Mo",
            "UPDATE",
            "book_comments",
            Some("{}"),
            Some(r#"{"deleted_at":1.0}"#),
        );
        let rows = aggregate_windowed(&conn, 0).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].comments_posted, 1);
    }

    #[test]
    fn cutoff_ms_brackets_match_window() {
        let now: i64 = 1_000_000_000;
        let day: i64 = 24 * 60 * 60 * 1000;
        assert_eq!(LeaderboardWindow::Last7Days.cutoff_ms(now), Some(now - 7 * day));
        assert_eq!(LeaderboardWindow::Last30Days.cutoff_ms(now), Some(now - 30 * day));
        assert_eq!(LeaderboardWindow::AllTime.cutoff_ms(now), None);
    }
}
