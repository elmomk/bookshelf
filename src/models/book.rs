use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[derive(Default)]
pub enum ReadingStatus {
    #[default]
    ToRead,
    Reading,
    Finished,
}


impl fmt::Display for ReadingStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadingStatus::ToRead => write!(f, "to_read"),
            ReadingStatus::Reading => write!(f, "reading"),
            ReadingStatus::Finished => write!(f, "finished"),
        }
    }
}

impl ReadingStatus {
    pub fn from_str(s: &str) -> Self {
        match s {
            "reading" => ReadingStatus::Reading,
            "finished" => ReadingStatus::Finished,
            _ => ReadingStatus::ToRead,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ReadingStatus::ToRead => "To Read",
            ReadingStatus::Reading => "Reading",
            ReadingStatus::Finished => "Finished",
        }
    }

    /// Next status in the swipe-right cycle: To Read → Reading → Finished → To Read.
    pub fn next(&self) -> Self {
        match self {
            ReadingStatus::ToRead => ReadingStatus::Reading,
            ReadingStatus::Reading => ReadingStatus::Finished,
            ReadingStatus::Finished => ReadingStatus::ToRead,
        }
    }
}

/// One reader's progress on a shared book.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReadingProgress {
    pub id: String,
    pub book_id: String,
    pub reader: String,
    pub current_page: Option<i32>,
    pub current_chapter: Option<i32>,
    #[serde(default)]
    pub status: ReadingStatus,
    pub updated_at: f64,
}

/// A book on the shared club shelf.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Book {
    pub id: String,
    pub title: String,
    pub author: Option<String>,
    pub cover_url: Option<String>,
    pub total_pages: Option<i32>,
    pub total_chapters: Option<i32>,
    pub description: Option<String>,
    pub google_books_id: Option<String>,
    pub isbn: Option<String>,
    pub added_by: Option<String>,
    pub created_at: f64,
    /// JSON-encoded `Vec<TocEntry>` from Open Library (best-effort), or None.
    #[serde(default)]
    pub toc_json: Option<String>,
    // joined for the club view
    #[serde(default)]
    pub progress: Vec<ReadingProgress>,
    #[serde(default)]
    pub my_progress: Option<ReadingProgress>,
    #[serde(default)]
    pub comment_count: i32,
}

impl Book {
    /// Parse the stored Open Library table of contents (empty if absent/invalid).
    pub fn toc(&self) -> Vec<TocEntry> {
        self.toc_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<Vec<TocEntry>>(s).ok())
            .unwrap_or_default()
    }

    /// Reading progress as `(done, total)` for the progress bar.
    ///
    /// When the book has a table of contents, progress is chapter-based:
    /// the reader's current section number out of the total number of
    /// sections. Otherwise it falls back to page count. `Finished` is always
    /// full; `ToRead` (or no usable data) yields `None` (no bar).
    pub fn reading_fraction(&self, p: &ReadingProgress) -> Option<(i32, i32)> {
        match p.status {
            ReadingStatus::Finished => return Some((1, 1)),
            ReadingStatus::ToRead => return None,
            ReadingStatus::Reading => {}
        }
        let sections = self.toc().len() as i32;
        if sections > 0 {
            if let Some(ch) = p.current_chapter.filter(|c| *c > 0) {
                return Some((ch.min(sections), sections));
            }
        }
        match (p.current_page, self.total_pages) {
            (Some(c), Some(t)) if t > 0 => Some((c.clamp(0, t), t)),
            _ => None,
        }
    }
}

/// One table-of-contents entry (chapter or, when `level > 0`, a subchapter).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TocEntry {
    pub title: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub page: Option<i32>,
    #[serde(default)]
    pub level: i32,
}

/// A discussion comment. Optionally anchored to a page/chapter for spoiler gating.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BookComment {
    pub id: String,
    pub book_id: String,
    pub author: String,
    pub body: String,
    pub page: Option<i32>,
    pub chapter: Option<i32>,
    pub created_at: f64,
    /// Set by the server when the anchor is past the requesting reader's progress.
    /// When `true`, `body` is blanked.
    #[serde(default)]
    pub hidden: bool,
    /// Per-emoji reaction tallies for this comment (server-populated).
    #[serde(default)]
    pub reactions: Vec<Reaction>,
}

/// The fixed set of allowed comment reactions (kept in sync client/server).
pub const REACTION_EMOJIS: [&str; 6] = ["👍", "❤️", "😂", "😮", "😢", "🔥"];

/// Aggregated reaction count for one emoji on one comment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reaction {
    pub emoji: String,
    pub count: i32,
    /// True if the requesting reader is one of the reactors.
    pub mine: bool,
}

/// A Google Books search hit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BookSearchResult {
    pub google_books_id: String,
    pub title: String,
    pub author: Option<String>,
    pub cover_url: Option<String>,
    pub total_pages: Option<i32>,
    pub description: Option<String>,
    pub isbn: Option<String>,
}
