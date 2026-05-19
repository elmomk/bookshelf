use dioxus::prelude::*;

use crate::models::{Book, BookComment, BookSearchResult, ReadingProgress, ReadingStatus, TocEntry};

/// Best-effort table-of-contents lookup from Open Library by ISBN.
/// Returns a JSON-encoded `Vec<TocEntry>`, or None if unavailable/empty.
#[cfg(not(target_arch = "wasm32"))]
async fn fetch_ol_toc(isbn: &str) -> Option<String> {
    let isbn = isbn.trim();
    if isbn.is_empty() {
        return None;
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .ok()?;
    let resp = client
        .get(format!("https://openlibrary.org/isbn/{isbn}.json"))
        .header("User-Agent", "bookclub/0.1 (reading club PWA)")
        .header("Accept", "application/json")
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: serde_json::Value = resp.json().await.ok()?;
    let arr = json.get("table_of_contents")?.as_array()?;

    let mut entries: Vec<TocEntry> = Vec::new();
    for e in arr {
        let (title, label, page, level) = if let Some(s) = e.as_str() {
            (s.trim().to_string(), None, None, 0)
        } else {
            let title = e
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let label = e
                .get("label")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let page = e
                .get("pagenum")
                .and_then(|v| v.as_str())
                .and_then(|s| s.trim().parse::<i32>().ok());
            let level = e.get("level").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            (title, label, page, level)
        };
        if title.is_empty() && label.is_none() {
            continue;
        }
        entries.push(TocEntry {
            title: title.chars().take(200).collect(),
            label,
            page,
            level: level.clamp(0, 5),
        });
        if entries.len() >= 1000 {
            break;
        }
    }

    if entries.is_empty() {
        return None;
    }
    serde_json::to_string(&entries).ok()
}

// --- Shelf ---

#[server(headers: axum::http::HeaderMap)]
pub async fn list_books() -> Result<Vec<Book>, ServerFnError> {
    use crate::server::{auth, db};

    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    let me = auth::display_name_from_headers(&headers);
    let conn = db::pool().get().map_err(|e| ServerFnError::new(e.to_string()))?;

    let mut stmt = conn
        .prepare(
            "SELECT id, title, author, cover_url, total_pages, total_chapters,
                    description, google_books_id, isbn, added_by, created_at, toc_json
             FROM books ORDER BY created_at DESC",
        )
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let mut books: Vec<Book> = stmt
        .query_map([], |row| {
            Ok(Book {
                id: row.get(0)?,
                title: row.get(1)?,
                author: row.get(2)?,
                cover_url: row.get(3)?,
                total_pages: row.get(4)?,
                total_chapters: row.get(5)?,
                description: row.get(6)?,
                google_books_id: row.get(7)?,
                isbn: row.get(8)?,
                added_by: row.get(9)?,
                created_at: row.get(10)?,
                toc_json: row.get(11)?,
                progress: vec![],
                my_progress: None,
                comment_count: 0,
            })
        })
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let mut pstmt = conn
        .prepare(
            "SELECT id, book_id, reader, current_page, current_chapter, status, updated_at
             FROM reading_progress",
        )
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let all_progress: Vec<ReadingProgress> = pstmt
        .query_map([], |row| {
            let st: String = row.get(5)?;
            Ok(ReadingProgress {
                id: row.get(0)?,
                book_id: row.get(1)?,
                reader: row.get(2)?,
                current_page: row.get(3)?,
                current_chapter: row.get(4)?,
                status: ReadingStatus::from_str(&st),
                updated_at: row.get(6)?,
            })
        })
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();

    let mut cstmt = conn
        .prepare("SELECT book_id, COUNT(*) FROM book_comments GROUP BY book_id")
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let counts: std::collections::HashMap<String, i32> = cstmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i32>(1)?))
        })
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();

    for b in &mut books {
        b.progress = all_progress
            .iter()
            .filter(|p| p.book_id == b.id)
            .cloned()
            .collect();
        b.my_progress = b.progress.iter().find(|p| p.reader == me).cloned();
        b.comment_count = *counts.get(&b.id).unwrap_or(&0);
    }

    fn rank(b: &Book) -> u8 {
        match b.my_progress.as_ref().map(|p| &p.status) {
            Some(ReadingStatus::Reading) => 0,
            Some(ReadingStatus::Finished) => 2,
            _ => 1,
        }
    }
    books.sort_by(|a, b| {
        rank(a).cmp(&rank(b)).then(
            b.created_at
                .partial_cmp(&a.created_at)
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });

    Ok(books)
}

#[server(_headers: axum::http::HeaderMap)]
pub async fn get_book(id: String) -> Result<Option<Book>, ServerFnError> {
    let books = list_books().await?;
    Ok(books.into_iter().find(|b| b.id == id))
}

// --- Google Books search ---

#[server]
pub async fn search_books(query: String) -> Result<Vec<BookSearchResult>, ServerFnError> {
    if query.trim().is_empty() {
        return Ok(vec![]);
    }

    let key = std::env::var("GOOGLE_BOOKS_API_KEY").unwrap_or_default();
    let mut params: Vec<(&str, &str)> = vec![("q", query.as_str()), ("maxResults", "20")];
    if !key.is_empty() {
        params.push(("key", key.as_str()));
    }

    let resp = reqwest::Client::new()
        .get("https://www.googleapis.com/books/v1/volumes")
        .query(&params)
        .send()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let mut out = vec![];
    if let Some(items) = json.get("items").and_then(|v| v.as_array()) {
        for it in items {
            let id = it
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let vi = it.get("volumeInfo").cloned().unwrap_or_default();
            let title = vi
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if id.is_empty() || title.is_empty() {
                continue;
            }
            let author = vi
                .get("authors")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let cover_url = vi
                .get("imageLinks")
                .and_then(|il| il.get("thumbnail").or_else(|| il.get("smallThumbnail")))
                .and_then(|v| v.as_str())
                .map(|s| s.replace("http://", "https://"));
            let total_pages = vi
                .get("pageCount")
                .and_then(|v| v.as_i64())
                .filter(|n| *n > 0)
                .map(|n| n as i32);
            let description = vi
                .get("description")
                .and_then(|v| v.as_str())
                .map(|s| s.chars().take(2000).collect());
            let isbn = vi.get("industryIdentifiers").and_then(|v| v.as_array()).and_then(|arr| {
                arr.iter()
                    .find(|x| x.get("type").and_then(|t| t.as_str()) == Some("ISBN_13"))
                    .or_else(|| arr.first())
                    .and_then(|x| x.get("identifier").and_then(|i| i.as_str()))
                    .map(|s| s.to_string())
            });

            out.push(BookSearchResult {
                google_books_id: id,
                title,
                author,
                cover_url,
                total_pages,
                description,
                isbn,
            });
        }
    }

    Ok(out)
}

#[server(headers: axum::http::HeaderMap)]
pub async fn add_book(result: BookSearchResult) -> Result<String, ServerFnError> {
    use crate::server::{auth, db, validate};

    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    let me = auth::display_name_from_headers(&headers);
    validate::text(&result.title, "title")?;
    let conn = db::pool().get().map_err(|e| ServerFnError::new(e.to_string()))?;

    // De-dupe by Google Books id — the shelf is shared.
    if !result.google_books_id.is_empty() {
        let existing: Option<String> = conn
            .query_row(
                "SELECT id FROM books WHERE google_books_id = ?1",
                rusqlite::params![result.google_books_id],
                |r| r.get(0),
            )
            .ok();
        if let Some(id) = existing {
            return Ok(id);
        }
    }

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis() as f64;
    let gbid = if result.google_books_id.is_empty() {
        None
    } else {
        Some(result.google_books_id)
    };

    conn.execute(
        "INSERT INTO books
            (id, title, author, cover_url, total_pages, total_chapters,
             description, google_books_id, isbn, added_by, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            id,
            result.title,
            result.author,
            result.cover_url,
            result.total_pages,
            result.description,
            gbid,
            result.isbn,
            me,
            now
        ],
    )
    .map_err(|e| ServerFnError::new(e.to_string()))?;

    // Best-effort: enrich with a chapter list from Open Library (never fails the add).
    if let Some(isbn) = result.isbn.as_deref() {
        if let Some(toc) = fetch_ol_toc(isbn).await {
            let _ = conn.execute(
                "UPDATE books SET toc_json = ?1 WHERE id = ?2",
                rusqlite::params![toc, id],
            );
        }
    }

    crate::server::notify::create_notification(&me, "added", "books", &result.title);
    Ok(id)
}

#[server(headers: axum::http::HeaderMap)]
pub async fn add_book_manual(
    title: String,
    author: Option<String>,
    total_pages: Option<i32>,
    total_chapters: Option<i32>,
) -> Result<String, ServerFnError> {
    use crate::server::{auth, db, validate};

    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    let me = auth::display_name_from_headers(&headers);
    validate::text(&title, "title")?;
    if title.trim().is_empty() {
        return Err(ServerFnError::new("Title is required"));
    }
    let conn = db::pool().get().map_err(|e| ServerFnError::new(e.to_string()))?;

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis() as f64;

    conn.execute(
        "INSERT INTO books
            (id, title, author, cover_url, total_pages, total_chapters,
             description, google_books_id, isbn, added_by, created_at)
         VALUES (?1, ?2, ?3, NULL, ?4, ?5, NULL, NULL, NULL, ?6, ?7)",
        rusqlite::params![id, title, author, total_pages, total_chapters, me, now],
    )
    .map_err(|e| ServerFnError::new(e.to_string()))?;

    crate::server::notify::create_notification(&me, "added", "books", &title);
    Ok(id)
}

#[server(headers: axum::http::HeaderMap)]
pub async fn delete_book(id: String) -> Result<(), ServerFnError> {
    use crate::server::{auth, db};

    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    let me = auth::display_name_from_headers(&headers);
    let conn = db::pool().get().map_err(|e| ServerFnError::new(e.to_string()))?;

    let title: Option<String> = conn
        .query_row(
            "SELECT title FROM books WHERE id = ?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .ok();

    conn.execute("DELETE FROM books WHERE id = ?1", rusqlite::params![id])
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    if let Some(t) = title {
        crate::server::notify::create_notification(&me, "deleted", "books", &t);
    }
    Ok(())
}

/// Re-fetch the Open Library table of contents for an existing book.
/// Returns true if a chapter list was found and stored.
#[server(headers: axum::http::HeaderMap)]
pub async fn refresh_toc(book_id: String) -> Result<bool, ServerFnError> {
    use crate::server::{auth, db};

    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    let conn = db::pool().get().map_err(|e| ServerFnError::new(e.to_string()))?;

    let isbn: Option<String> = conn
        .query_row(
            "SELECT isbn FROM books WHERE id = ?1",
            rusqlite::params![book_id],
            |r| r.get(0),
        )
        .map_err(|_| ServerFnError::new("Book not found"))?;

    let Some(isbn) = isbn.filter(|s| !s.trim().is_empty()) else {
        return Ok(false);
    };

    match fetch_ol_toc(&isbn).await {
        Some(toc) => {
            conn.execute(
                "UPDATE books SET toc_json = ?1 WHERE id = ?2",
                rusqlite::params![toc, book_id],
            )
            .map_err(|e| ServerFnError::new(e.to_string()))?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Manually set (or clear) the shared chapter/section list for a book.
/// An empty `entries` clears it. Stored on the shared book so the whole club
/// sees the dropdown.
#[server(headers: axum::http::HeaderMap)]
pub async fn set_toc(book_id: String, entries: Vec<TocEntry>) -> Result<(), ServerFnError> {
    use crate::server::{auth, db};

    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    let conn = db::pool().get().map_err(|e| ServerFnError::new(e.to_string()))?;

    let exists: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM books WHERE id = ?1",
            rusqlite::params![book_id],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if !exists {
        return Err(ServerFnError::new("Book not found"));
    }
    if entries.len() > 1000 {
        return Err(ServerFnError::new("Too many entries (max 1000)"));
    }

    let json: Option<String> = if entries.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&entries).map_err(|e| ServerFnError::new(e.to_string()))?)
    };

    conn.execute(
        "UPDATE books SET toc_json = ?1 WHERE id = ?2",
        rusqlite::params![json, book_id],
    )
    .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(())
}

/// Light cleanup of raw OCR text into the manual-editor format.
///
/// Real ToC pages use dot leaders (`Chapter 3 ........ 42`). Tesseract turns
/// those leaders into long runs of dots interspersed with junk letters, so a
/// naive trailing-digit strip leaves 40+ chars of noise per line.
///
/// - drops header noise (Contents / Index) and digit-only / no-letter lines
/// - cuts the title at the first dot-leader run (>=2 consecutive `.·…`)
/// - extracts the last short digit group (1–4 digits, 1..=9999) as the page,
///   after a leader or directly after the title (`Conclusion 142`)
/// - trims leader/punct tails and OCR's stray single-letter residue
///
/// Still best-effort — the user reviews/fixes it in the editor before saving.
#[cfg(not(target_arch = "wasm32"))]
fn clean_ocr_toc(raw: &str) -> String {
    const DOTS: [char; 3] = ['.', '·', '…'];
    const TRIM: [char; 9] = ['.', '·', '…', ',', ':', ';', '-', '–', '_'];

    let mut out: Vec<String> = Vec::new();
    for line in raw.lines() {
        if out.len() >= 1000 {
            break;
        }
        let indent_spaces: usize = line
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .map(|c| if c == '\t' { 4 } else { 1 })
            .sum();
        let body = line.trim();
        if body.is_empty() || !body.chars().any(|c| c.is_alphabetic()) {
            continue;
        }
        let lower = body.to_lowercase();
        if lower.starts_with("contents")
            || lower.starts_with("table of contents")
            || lower == "index"
        {
            continue;
        }

        let chars: Vec<char> = body.chars().collect();

        // (1) First dot-leader run: earliest index whose run of `.·… ` holds >=2 dots.
        let mut leader_at: Option<usize> = None;
        for i in 0..chars.len() {
            if !DOTS.contains(&chars[i]) {
                continue;
            }
            let mut j = i;
            let mut dots = 0;
            while j < chars.len() && (DOTS.contains(&chars[j]) || chars[j] == ' ') {
                if chars[j] != ' ' {
                    dots += 1;
                }
                j += 1;
            }
            if dots >= 2 {
                leader_at = Some(i);
                break;
            }
        }

        // (2) Page = last short digit group (1..=9999) that is the real tail:
        // not glued to a letter, with NO alphabetic char anywhere after it
        // (so "Phase 3: Boost…" is not misread as page 3).
        let mut page: Option<i32> = None;
        let mut page_start: Option<usize> = None;
        let mut end = chars.len();
        while end > 0 {
            while end > 0 && !chars[end - 1].is_ascii_digit() {
                end -= 1;
            }
            if end == 0 {
                break;
            }
            let mut b = end;
            while b > 0 && chars[b - 1].is_ascii_digit() {
                b -= 1;
            }
            let run: String = chars[b..end].iter().collect();
            let before_ok = b == 0 || !chars[b - 1].is_alphabetic();
            let after_ok = chars[end..].iter().all(|c| !c.is_alphabetic());
            if run.len() <= 4 && before_ok && after_ok {
                if let Ok(n) = run.parse::<i32>() {
                    if (1..=9999).contains(&n) {
                        page = Some(n);
                        page_start = Some(b);
                        break;
                    }
                }
            }
            end = b; // not a valid page — keep looking further left
        }

        // (2b) Garbage/leader token cut. Tesseract renders dot leaders as dots
        // OR (on lossy JPEG) as letter-soup; cut the title at the first such
        // token that follows at least one real word.
        let is_garbage = |t: &str| -> bool {
            let cs: Vec<char> = t.chars().collect();
            if cs.is_empty() {
                return false;
            }
            if cs.iter().all(|c| !c.is_alphanumeric()) {
                return true; // pure punctuation ("....", "|", ",.,")
            }
            let mut consec_dot = 0;
            let mut consec_same = 0;
            let mut prev = '\0';
            for &c in &cs {
                consec_dot = if DOTS.contains(&c) { consec_dot + 1 } else { 0 };
                consec_same = if c == prev && c.is_ascii_alphabetic() {
                    consec_same + 1
                } else {
                    1
                };
                if consec_dot >= 2 || consec_same >= 3 {
                    return true; // ">=2 dots" or ">=3 identical letters" (ooo)
                }
                prev = c;
            }
            let letters: Vec<char> = cs
                .iter()
                .filter(|c| c.is_alphabetic())
                .map(|c| c.to_ascii_lowercase())
                .collect();
            if letters.len() >= 18 {
                return true; // implausibly long single token = OCR word-soup
            }
            if letters.len() >= 5 {
                let max = letters
                    .iter()
                    .map(|x| letters.iter().filter(|y| *y == x).count())
                    .max()
                    .unwrap_or(0);
                if max * 5 >= letters.len() * 3 {
                    return true; // one letter is >=60% ("eeeeeeeeeteeee")
                }
            }
            false
        };

        let region_end = page_start.unwrap_or(chars.len());
        let mut garbage_at: Option<usize> = None;
        let mut seen_word = false;
        let mut i = 0;
        while i < region_end {
            while i < region_end && chars[i] == ' ' {
                i += 1;
            }
            let start = i;
            while i < region_end && chars[i] != ' ' {
                i += 1;
            }
            if start == i {
                break;
            }
            let tok: String = chars[start..i].iter().collect();
            if seen_word && is_garbage(&tok) {
                garbage_at = Some(start);
                break;
            }
            if tok.chars().filter(|c| c.is_alphabetic()).count() >= 2 && !is_garbage(&tok) {
                seen_word = true;
            }
        }

        // (3) Title = up to the earliest of: leader, garbage token, page run.
        let cut = [leader_at, garbage_at, Some(region_end)]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(chars.len());
        let mut title: String = chars[..cut]
            .iter()
            .collect::<String>()
            .trim()
            .trim_end_matches(|c: char| TRIM.contains(&c) || c == ' ')
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        // (4) Drop OCR's trailing lowercase-letter leader residue: a single
        // lowercase letter ("...o", ", e") or a 2-letter all-same-lowercase
        // token ("oo", "ll", "ii" — dot leaders misread). Conservative: never
        // touches "Appendix A", "Part I" (uppercase), or real words.
        for _ in 0..4 {
            let toks: Vec<&str> = title.split(' ').collect();
            let drop = toks.len() > 1
                && toks
                    .last()
                    .map(|t| {
                        let cs: Vec<char> = t.chars().collect();
                        match cs.as_slice() {
                            [a] => a.is_ascii_lowercase(),
                            [a, b] => a.is_ascii_lowercase() && a == b,
                            _ => false,
                        }
                    })
                    .unwrap_or(false);
            if drop {
                title = toks[..toks.len() - 1].join(" ");
            } else {
                break;
            }
        }

        if title.chars().filter(|c| c.is_alphabetic()).count() < 2 {
            continue;
        }

        let indent = " ".repeat((indent_spaces.min(8) / 2) * 2);
        let suffix = page.map(|n| format!(" | {n}")).unwrap_or_default();
        let line_out: String = format!("{indent}{title}{suffix}").chars().take(200).collect();
        out.push(line_out);
    }

    // (5) Auto-join wrapped entries: a long ToC entry that spilled onto a
    // second physical line becomes two cleaned lines. Merge a line into the
    // previous one when it's clearly a continuation, not a new entry. High
    // precision (favours NOT joining): never joins across a page'd previous
    // line, a structural keyword / numbered / bulleted start, or differing
    // indent. At most two continuations fold into one entry.
    let mut merged: Vec<String> = Vec::with_capacity(out.len());
    let mut folds = 0;
    for line in &out {
        let (ind, title, page) = split_clean(line);
        if let Some(prev) = merged.last() {
            let (pind, ptitle, ppage) = split_clean(prev);
            if folds < 2 && ppage.is_none() && pind == ind && is_continuation(&ptitle, &title) {
                let joined = if ptitle.trim_end().ends_with('-') {
                    format!("{}{}", ptitle.trim_end(), title)
                } else {
                    format!("{ptitle} {title}")
                };
                let joined: String = joined.chars().take(200).collect();
                let suffix = page.map(|n| format!(" | {n}")).unwrap_or_default();
                *merged.last_mut().unwrap() = format!("{pind}{joined}{suffix}");
                folds += 1;
                continue;
            }
        }
        merged.push(line.clone());
        folds = 0;
    }
    merged.join("\n")
}

/// Split a cleaned ToC line into (indent, title, page).
#[cfg(not(target_arch = "wasm32"))]
fn split_clean(s: &str) -> (String, String, Option<i32>) {
    let indent: String = s.chars().take_while(|c| *c == ' ').collect();
    let rest = &s[indent.len()..];
    if let Some(idx) = rest.rfind(" | ") {
        if let Ok(n) = rest[idx + 3..].trim().parse::<i32>() {
            return (indent, rest[..idx].trim().to_string(), Some(n));
        }
    }
    (indent, rest.trim().to_string(), None)
}

/// True when `cur` looks like the wrapped continuation of `prev`, not a new
/// entry. Conservative: a structural keyword / numbered / bulleted start is
/// always a new entry; otherwise join only on a strong wrap signal
/// (continuation starts lowercase, previous ends on a function word, or
/// previous ends with a hyphen).
#[cfg(not(target_arch = "wasm32"))]
fn is_continuation(prev: &str, cur: &str) -> bool {
    const STRUCT: &[&str] = &[
        "phase", "part", "chapter", "section", "appendix", "appendices", "book", "volume",
        "unit", "lesson", "step", "module", "intro", "introduction", "prologue", "preface",
        "foreword", "epilogue", "afterword", "conclusion", "outro", "notes", "references",
        "bibliography", "glossary", "index", "acknowledgements", "acknowledgments", "contents",
    ];
    const STOP: &[&str] = &[
        "a", "an", "the", "of", "and", "or", "to", "in", "on", "at", "by", "for", "with",
        "from", "into", "your", "our", "my", "his", "her", "their", "its", "as", "is",
        "that", "this",
    ];
    let ct = cur.trim();
    let Some(c0) = ct.chars().next() else {
        return false;
    };
    if c0.is_ascii_digit() {
        let lead = ct.chars().take_while(|c| c.is_ascii_digit()).count();
        let after = &ct[lead..];
        if after.is_empty() || after.starts_with([' ', '.', ')', ':']) {
            return false; // "12." / "3) " — a new numbered entry
        }
    }
    if matches!(c0, '-' | '*' | '•' | '–' | '>') {
        return false; // bulleted = new entry
    }
    let norm = |w: &str| {
        w.trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase()
    };
    let first_word = norm(ct.split_whitespace().next().unwrap_or(""));
    if STRUCT.contains(&first_word.as_str()) {
        return false;
    }
    let prev_last = norm(prev.split_whitespace().last().unwrap_or(""));
    c0.is_ascii_lowercase()
        || STOP.contains(&prev_last.as_str())
        || prev.trim_end().ends_with('-')
}

/// Common ToC front/back-matter words Tesseract reliably mangles (e.g.
/// "INTRODUCTION" -> "INEFODUCTION", "ONE" -> "ONG"). Fed via `--user-words`
/// to bias recognition / dictionary correction toward these.
#[cfg(not(target_arch = "wasm32"))]
const TOC_USER_WORDS: &str = "Contents\nIntroduction\nConclusion\nChapter\nChapters\nPart\nParts\nSection\nSections\nPreface\nForeword\nPrologue\nEpilogue\nAfterword\nAppendix\nAppendices\nAcknowledgements\nAcknowledgments\nBibliography\nReferences\nGlossary\nIndex\nNotes\nEndnotes\nFootnotes\nSummary\nOverview\nAbstract\nDedication\nEpigraph\nVolume\nBook\nPhase\nPhases\nStep\nSteps\nLesson\nLessons\nModule\nModules\nUnit\nUnits\nExercise\nWorksheet\nToolkit\nOne\nTwo\nThree\nFour\nFive\nSix\nSeven\nEight\nNine\nTen\n";

/// Page-number shapes for `--user-patterns` (1–4 digit runs).
#[cfg(not(target_arch = "wasm32"))]
const TOC_USER_PATTERNS: &str = "\\d\n\\d\\d\n\\d\\d\\d\n\\d\\d\\d\\d\n";

/// Decode + preprocess an image for OCR. Grayscale and, crucially, upscale
/// small images: Tesseract's LSTM engine wants a generous cap-height, and
/// upscaling is the single biggest, real-photo-safe quality lever (no global
/// threshold — that would wreck photos with uneven lighting). On any decode
/// failure (e.g. HEIC) the original bytes are returned so Tesseract can still
/// try them — never regresses below the previous behaviour.
#[cfg(not(target_arch = "wasm32"))]
fn preprocess_for_ocr(bytes: &[u8]) -> Vec<u8> {
    use image::{DynamicImage, ImageFormat};
    const TARGET: u32 = 2400;
    const MAX: u32 = 4000;

    let Ok(img) = image::load_from_memory(bytes) else {
        return bytes.to_vec();
    };
    let gray = img.to_luma8();
    let (w, h) = (gray.width(), gray.height());
    if w == 0 || h == 0 {
        return bytes.to_vec();
    }
    let longest = w.max(h);
    let processed = if longest < TARGET {
        let scale = (TARGET as f32 / longest as f32).clamp(1.0, 2.0);
        let nw = ((w as f32 * scale) as u32).clamp(1, MAX);
        let nh = ((h as f32 * scale) as u32).clamp(1, MAX);
        image::imageops::resize(&gray, nw, nh, image::imageops::FilterType::Lanczos3)
    } else if longest > MAX {
        let scale = MAX as f32 / longest as f32;
        let nw = ((w as f32 * scale) as u32).max(1);
        let nh = ((h as f32 * scale) as u32).max(1);
        image::imageops::resize(&gray, nw, nh, image::imageops::FilterType::Triangle)
    } else {
        gray
    };
    let mut out = Vec::new();
    if DynamicImage::ImageLuma8(processed)
        .write_to(&mut std::io::Cursor::new(&mut out), ImageFormat::Png)
        .is_ok()
    {
        out
    } else {
        bytes.to_vec()
    }
}

/// OCR a photo of a book's table-of-contents/index page into editor text.
/// Returns lightly-cleaned text for the user to review in the manual editor;
/// nothing is persisted here.
#[server(headers: axum::http::HeaderMap)]
pub async fn ocr_toc(image_base64: String) -> Result<String, ServerFnError> {
    use crate::server::auth;
    use base64::Engine;
    use std::io::Write;

    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;

    let raw_b64 = match image_base64.find(',') {
        Some(pos) => &image_base64[pos + 1..],
        None => &image_base64[..],
    };
    const MAX_B64: usize = 10 * 1024 * 1024 * 4 / 3;
    if raw_b64.len() > MAX_B64 {
        return Err(ServerFnError::new("Image too large (max 10MB)"));
    }

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(raw_b64)
        .map_err(|e| ServerFnError::new(format!("Base64 decode error: {e}")))?;

    // Preprocess off the async runtime (CPU-bound image resize).
    let img_bytes = tokio::task::spawn_blocking(move || preprocess_for_ocr(&bytes))
        .await
        .map_err(|e| ServerFnError::new(format!("Preprocess error: {e}")))?;

    let mut tmp = tempfile::NamedTempFile::new()
        .map_err(|e| ServerFnError::new(format!("Temp file error: {e}")))?;
    tmp.write_all(&img_bytes)
        .map_err(|e| ServerFnError::new(format!("Write error: {e}")))?;
    let path = tmp.path().to_string_lossy().to_string();

    // ToC vocabulary + page-number patterns to bias recognition.
    let mut words_f = tempfile::NamedTempFile::new()
        .map_err(|e| ServerFnError::new(format!("Temp file error: {e}")))?;
    words_f
        .write_all(TOC_USER_WORDS.as_bytes())
        .map_err(|e| ServerFnError::new(format!("Write error: {e}")))?;
    let words_path = words_f.path().to_string_lossy().to_string();

    let mut pat_f = tempfile::NamedTempFile::new()
        .map_err(|e| ServerFnError::new(format!("Temp file error: {e}")))?;
    pat_f
        .write_all(TOC_USER_PATTERNS.as_bytes())
        .map_err(|e| ServerFnError::new(format!("Write error: {e}")))?;
    let pat_path = pat_f.path().to_string_lossy().to_string();

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(45),
        tokio::process::Command::new("tesseract")
            .arg(&path)
            .arg("stdout")
            .arg("-l")
            .arg("eng")
            .arg("--psm")
            .arg("4")
            .arg("--dpi")
            .arg("300")
            .arg("--user-words")
            .arg(&words_path)
            .arg("--user-patterns")
            .arg(&pat_path)
            .output(),
    )
    .await
    .map_err(|_| ServerFnError::new("OCR timed out (45s limit)"))?
    .map_err(|e| ServerFnError::new(format!("Tesseract error: {e} (is tesseract installed?)")))?;

    if !output.status.success() {
        tracing::error!("tesseract failed: {}", String::from_utf8_lossy(&output.stderr));
        return Err(ServerFnError::new("OCR processing failed"));
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    let cleaned = clean_ocr_toc(&raw);
    if cleaned.trim().is_empty() {
        return Err(ServerFnError::new("No text recognized in the image"));
    }
    Ok(cleaned)
}

// --- Per-reader progress ---

#[server(headers: axum::http::HeaderMap)]
pub async fn set_reading_progress(
    book_id: String,
    current_page: Option<i32>,
    current_chapter: Option<i32>,
    status: ReadingStatus,
) -> Result<(), ServerFnError> {
    use crate::server::{auth, db};

    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    let me = auth::display_name_from_headers(&headers);
    let conn = db::pool().get().map_err(|e| ServerFnError::new(e.to_string()))?;

    let title: String = conn
        .query_row(
            "SELECT title FROM books WHERE id = ?1",
            rusqlite::params![book_id],
            |r| r.get(0),
        )
        .map_err(|_| ServerFnError::new("Book not found"))?;

    let prev: Option<String> = conn
        .query_row(
            "SELECT status FROM reading_progress WHERE book_id = ?1 AND reader = ?2",
            rusqlite::params![book_id, me],
            |r| r.get(0),
        )
        .ok();

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis() as f64;
    let status_str = status.to_string();

    conn.execute(
        "INSERT INTO reading_progress
            (id, book_id, reader, current_page, current_chapter, status, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(book_id, reader) DO UPDATE SET
            current_page = ?4,
            current_chapter = ?5,
            status = ?6,
            updated_at = ?7",
        rusqlite::params![id, book_id, me, current_page, current_chapter, status_str, now],
    )
    .map_err(|e| ServerFnError::new(e.to_string()))?;

    let prev_status = prev.map(|s| ReadingStatus::from_str(&s));
    match status {
        ReadingStatus::Finished if prev_status.as_ref() != Some(&ReadingStatus::Finished) => {
            crate::server::notify::create_notification(&me, "finished", "books", &title);
        }
        ReadingStatus::Reading if prev_status.as_ref() != Some(&ReadingStatus::Reading) => {
            crate::server::notify::create_notification(&me, "started reading", "books", &title);
        }
        _ => {}
    }

    Ok(())
}

// --- Comments (server-side spoiler gating) ---

#[server(headers: axum::http::HeaderMap)]
pub async fn list_comments(book_id: String) -> Result<Vec<BookComment>, ServerFnError> {
    use crate::server::{auth, db};

    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    let me = auth::display_name_from_headers(&headers);
    let conn = db::pool().get().map_err(|e| ServerFnError::new(e.to_string()))?;

    let (my_page, my_chapter): (Option<i32>, Option<i32>) = conn
        .query_row(
            "SELECT current_page, current_chapter FROM reading_progress
             WHERE book_id = ?1 AND reader = ?2",
            rusqlite::params![book_id, me],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap_or((None, None));

    let mut stmt = conn
        .prepare(
            "SELECT id, book_id, author, body, page, chapter, created_at, parent_id
             FROM book_comments WHERE book_id = ?1 ORDER BY created_at DESC",
        )
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let comments = stmt
        .query_map(rusqlite::params![book_id], |row| {
            Ok(BookComment {
                id: row.get(0)?,
                book_id: row.get(1)?,
                author: row.get(2)?,
                body: row.get(3)?,
                page: row.get(4)?,
                chapter: row.get(5)?,
                created_at: row.get(6)?,
                hidden: false,
                reactions: Vec::new(),
                parent_id: row.get(7)?,
            })
        })
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    // Reaction tallies for every comment on this book, in one query.
    let mut reactions: std::collections::HashMap<String, Vec<crate::models::Reaction>> =
        std::collections::HashMap::new();
    {
        let mut rstmt = conn
            .prepare(
                "SELECT cr.comment_id, cr.emoji, COUNT(*),
                        MAX(CASE WHEN cr.reader = ?2 THEN 1 ELSE 0 END)
                 FROM comment_reactions cr
                 JOIN book_comments bc ON bc.id = cr.comment_id
                 WHERE bc.book_id = ?1
                 GROUP BY cr.comment_id, cr.emoji",
            )
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        let rows = rstmt
            .query_map(rusqlite::params![book_id, me], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    crate::models::Reaction {
                        emoji: row.get(1)?,
                        count: row.get(2)?,
                        mine: row.get::<_, i32>(3)? != 0,
                    },
                ))
            })
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        for r in rows {
            let (cid, reaction) = r.map_err(|e| ServerFnError::new(e.to_string()))?;
            reactions.entry(cid).or_default().push(reaction);
        }
        // Stable display order: most-reacted first, then emoji.
        for v in reactions.values_mut() {
            v.sort_by(|a, b| b.count.cmp(&a.count).then(a.emoji.cmp(&b.emoji)));
        }
    }

    let gated = comments
        .into_iter()
        .map(|mut c| {
            c.reactions = reactions.remove(&c.id).unwrap_or_default();
            if c.author != me {
                let page_spoiler = matches!(c.page, Some(p) if p > my_page.unwrap_or(0));
                let chap_spoiler =
                    matches!(c.chapter, Some(ch) if ch > my_chapter.unwrap_or(0));
                if page_spoiler || chap_spoiler {
                    c.hidden = true;
                    c.body = String::new();
                }
            }
            c
        })
        .collect();

    Ok(gated)
}

/// Toggle the calling reader's reaction (one of `REACTION_EMOJIS`) on a
/// comment: removes it if already set, otherwise adds it.
#[server(headers: axum::http::HeaderMap)]
pub async fn react_to_comment(
    comment_id: String,
    emoji: String,
) -> Result<(), ServerFnError> {
    use crate::server::{auth, db};

    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    let me = auth::display_name_from_headers(&headers);

    // Accept any emoji from the user's keyboard, but guard against abuse:
    // non-empty, short, no ASCII letters / whitespace / control, and at
    // least one non-ASCII char (so it can't be used as a free-text tag).
    let emoji = emoji.trim().to_string();
    let invalid = emoji.is_empty()
        || emoji.len() > 32
        || emoji
            .chars()
            .any(|ch| ch.is_ascii_alphabetic() || ch.is_whitespace() || ch.is_control())
        || !emoji.chars().any(|ch| !ch.is_ascii());
    if invalid {
        return Err(ServerFnError::new("Invalid reaction"));
    }

    let conn = db::pool()
        .get()
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM book_comments WHERE id = ?1)",
            rusqlite::params![comment_id],
            |r| r.get(0),
        )
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    if !exists {
        return Err(ServerFnError::new("Comment not found"));
    }

    let removed = conn
        .execute(
            "DELETE FROM comment_reactions
             WHERE comment_id = ?1 AND reader = ?2 AND emoji = ?3",
            rusqlite::params![comment_id, me, emoji],
        )
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    if removed == 0 {
        let now = chrono::Utc::now().timestamp_millis() as f64;
        conn.execute(
            "INSERT INTO comment_reactions (comment_id, reader, emoji, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![comment_id, me, emoji, now],
        )
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    }

    Ok(())
}

#[server(headers: axum::http::HeaderMap)]
pub async fn add_comment(
    book_id: String,
    body: String,
    parent_id: Option<String>,
) -> Result<(), ServerFnError> {
    use crate::server::{auth, db, validate};

    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    let me = auth::display_name_from_headers(&headers);
    if body.trim().is_empty() {
        return Err(ServerFnError::new("Comment cannot be empty"));
    }
    validate::comment(&body)?;
    let conn = db::pool().get().map_err(|e| ServerFnError::new(e.to_string()))?;

    // Replies are flattened to a single level: the parent must exist and be on
    // this book; if the target is itself a reply, anchor to its root instead.
    let parent_id: Option<String> = match parent_id {
        Some(pid) => {
            let row: Option<Option<String>> = conn
                .query_row(
                    "SELECT parent_id FROM book_comments WHERE id = ?1 AND book_id = ?2",
                    rusqlite::params![pid, book_id],
                    |r| r.get(0),
                )
                .ok();
            match row {
                None => return Err(ServerFnError::new("Parent comment not found")),
                Some(grandparent) => Some(grandparent.unwrap_or(pid)),
            }
        }
        None => None,
    };

    // Every comment is anchored to the commenter's current reading position
    // so spoiler-gating always applies (no opt-in). If they have no progress
    // yet, it stays unanchored (visible to all).
    let (page, chapter): (Option<i32>, Option<i32>) = conn
        .query_row(
            "SELECT current_page, current_chapter FROM reading_progress
             WHERE book_id = ?1 AND reader = ?2",
            rusqlite::params![book_id, me],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap_or((None, None));

    let title: Option<String> = conn
        .query_row(
            "SELECT title FROM books WHERE id = ?1",
            rusqlite::params![book_id],
            |r| r.get(0),
        )
        .ok();

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis() as f64;

    conn.execute(
        "INSERT INTO book_comments
            (id, book_id, author, body, page, chapter, created_at, parent_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![id, book_id, me, body, page, chapter, now, parent_id],
    )
    .map_err(|e| ServerFnError::new(e.to_string()))?;

    if let Some(t) = title {
        crate::server::notify::create_notification(&me, "commented on", "books", &t);
    }
    Ok(())
}

#[server(headers: axum::http::HeaderMap)]
pub async fn delete_comment(id: String) -> Result<(), ServerFnError> {
    use crate::server::{auth, db};

    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    let me = auth::display_name_from_headers(&headers);
    let mut conn = db::pool()
        .get()
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    // Deleting a thread root must not orphan its replies: promote them to
    // top-level first, then delete — atomically.
    let tx = conn
        .transaction()
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    tx.execute(
        "UPDATE book_comments SET parent_id = NULL WHERE parent_id = ?1",
        rusqlite::params![id],
    )
    .map_err(|e| ServerFnError::new(e.to_string()))?;
    tx.execute(
        "DELETE FROM book_comments WHERE id = ?1 AND author = ?2",
        rusqlite::params![id, me],
    )
    .map_err(|e| ServerFnError::new(e.to_string()))?;
    tx.commit()
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(())
}

#[server(headers: axum::http::HeaderMap)]
pub async fn whoami() -> Result<String, ServerFnError> {
    use crate::server::auth;
    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    Ok(auth::display_name_from_headers(&headers))
}

// --- Activity feed ---

#[server(headers: axum::http::HeaderMap)]
pub async fn list_activity() -> Result<Vec<crate::models::notification::Notification>, ServerFnError>
{
    use crate::models::notification::Notification;
    use crate::server::{auth, db};

    auth::user_from_headers(&headers).map_err(ServerFnError::new)?;
    let conn = db::pool().get().map_err(|e| ServerFnError::new(e.to_string()))?;

    let mut stmt = conn
        .prepare(
            "SELECT id, actor, action, module, item_text, created_at
             FROM notifications ORDER BY created_at DESC LIMIT 50",
        )
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let rows = stmt
        .query_map([], |row| {
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

    Ok(rows)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::clean_ocr_toc;

    // Verbatim `tesseract --psm 4 -l eng` output on two synthetic ToC photos
    // (dot leaders + page numbers), the exact case the user hit as garbage.
    const OCR_P1: &str = "Contents\n\nINErodUCtioN ... .. ..o I\n\nPhase 1: Ditch the Negative . ..............ccocee 1.\nBreak the Cycle ..........cccoovviiiiiiiie e, R\nNeuroToolkit One ,............cccccvvvveicineiiens 21....\n\nPhase 2: Shift Your Narrative 45\n\nRewire Your Subconscious 47\n";
    const OCR_P2: &str = "Contents (cont.)\n\nLeave Your Phone Alone ...............c.c...... 22.\nVisualization and Attention _.....................58\nPhase 3: Boost the Positive ,..................ccccoveee 91..\nBuild Better Habits ..., 95....\nConclusion 142\n";

    #[test]
    fn strips_dot_leaders_and_extracts_pages() {
        assert_eq!(
            clean_ocr_toc(OCR_P1).lines().collect::<Vec<_>>(),
            vec![
                "INErodUCtioN",
                "Phase 1: Ditch the Negative | 1",
                "Break the Cycle",
                "NeuroToolkit One | 21",
                "Phase 2: Shift Your Narrative | 45",
                "Rewire Your Subconscious | 47",
            ]
        );
        assert_eq!(
            clean_ocr_toc(OCR_P2).lines().collect::<Vec<_>>(),
            vec![
                "Leave Your Phone Alone | 22",
                "Visualization and Attention | 58",
                "Phase 3: Boost the Positive | 91",
                "Build Better Habits | 95",
                "Conclusion | 142",
            ]
        );
        // No line should still carry leader/residue garbage.
        for l in clean_ocr_toc(OCR_P1).lines().chain(clean_ocr_toc(OCR_P2).lines()) {
            assert!(!l.contains(".."), "leader leaked through: {l:?}");
        }
    }

    #[test]
    fn survives_jpeg_letter_soup() {
        // Lines the live run produced as unusable garbage on lossy JPEG.
        assert_eq!(
            clean_ocr_toc("Build Better Habits ooo. eeeeeeeeeteeee 99.0"),
            "Build Better Habits | 99"
        );
        assert_eq!(
            clean_ocr_toc("Break: the Cycle: csvecenavaesawenern 0 vs as 0 | 4"),
            "Break: the Cycle | 4"
        );
        // A title-internal number must not be misread as the page.
        assert_eq!(
            clean_ocr_toc("Phase 3: Boost the Positive ,.........ccccoveee"),
            "Phase 3: Boost the Positive"
        );
        // Trailing leader-dot residue read as "oo"/"ll" must be dropped.
        assert_eq!(clean_ocr_toc("Build Better Habits oo"), "Build Better Habits");
        assert_eq!(clean_ocr_toc("Leave Your Phone Alone ll"), "Leave Your Phone Alone");
        // No output line may carry a long junk tail.
        let blob = "INErodUCtioN ... ... oot o\nB Gy T 0o [ R—— e —— . N\nNeuroToolkit One ,...........ccccovvvverierneiienins 21....";
        for l in clean_ocr_toc(blob).lines() {
            assert!(!l.contains(".."), "leader leaked: {l:?}");
            assert!(l.chars().count() <= 40, "junk tail leaked: {l:?}");
        }
    }

    #[test]
    fn auto_joins_wrapped_titles() {
        // The three real wrap splits from the live Rewire photo.
        assert_eq!(
            clean_ocr_toc("NeuroToolkit 1: How to Ditch the\nNegative"),
            "NeuroToolkit 1: How to Ditch the Negative"
        );
        assert_eq!(
            clean_ocr_toc("Your Muscles Communicate Directly\nwith Your Brain"),
            "Your Muscles Communicate Directly with Your Brain"
        );
        assert_eq!(
            clean_ocr_toc("Sleep Is Your Number-One Optimi\nzation Tool"),
            "Sleep Is Your Number-One Optimi zation Tool"
        );
        // A trailing-page wrap keeps the page on the joined entry.
        assert_eq!(
            clean_ocr_toc("A very long chapter title that\nwraps ........ 142"),
            "A very long chapter title that wraps | 142"
        );
        // Must NOT join genuinely separate entries.
        assert_eq!(
            clean_ocr_toc("Negativity Bias\nThe Power of Your Thoughts").lines().count(),
            2
        );
        assert_eq!(
            clean_ocr_toc("Phase 1: Ditch the Negative\nPhase 2: Shift Your Narrative")
                .lines()
                .count(),
            2
        );
        assert_eq!(
            clean_ocr_toc("Break the Cycle | 9\nNegativity Bias").lines().count(),
            2
        );
        assert_eq!(
            clean_ocr_toc("1. Leave Your Phone Alone\n2. Visualization").lines().count(),
            2
        );
        assert_eq!(clean_ocr_toc("Outro\nPower").lines().count(), 2);
    }

    #[test]
    fn keeps_meaningful_short_suffixes() {
        // Real entries that look like OCR residue must survive.
        assert_eq!(clean_ocr_toc("Appendix A ......... 201"), "Appendix A | 201");
        assert_eq!(clean_ocr_toc("Part I"), "Part I");
        // Pure noise / headers are dropped.
        assert_eq!(clean_ocr_toc("Contents"), "");
        assert_eq!(clean_ocr_toc("........ 42"), "");
    }
}
