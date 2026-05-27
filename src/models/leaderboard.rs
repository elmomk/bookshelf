use serde::{Deserialize, Serialize};

/// Which time window the leaderboard is summarizing.
///
/// `Last7Days` and `Last30Days` aggregate per-update deltas from the
/// `db_changes` audit log; `AllTime` reads from the raw user-data tables
/// because the change log only goes back to v0.1.37.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum LeaderboardWindow {
    Last7Days,
    Last30Days,
    AllTime,
}

impl LeaderboardWindow {
    /// Cutoff (epoch ms) for `db_changes.ts > ?`. `None` for all-time.
    pub fn cutoff_ms(self, now_ms: i64) -> Option<i64> {
        const DAY_MS: i64 = 24 * 60 * 60 * 1000;
        match self {
            LeaderboardWindow::Last7Days => Some(now_ms - 7 * DAY_MS),
            LeaderboardWindow::Last30Days => Some(now_ms - 30 * DAY_MS),
            LeaderboardWindow::AllTime => None,
        }
    }

    /// Stable short tag for cache keys / labels.
    pub fn tag(self) -> &'static str {
        match self {
            LeaderboardWindow::Last7Days => "7d",
            LeaderboardWindow::Last30Days => "30d",
            LeaderboardWindow::AllTime => "all",
        }
    }
}

/// One row of the leaderboard — per-reader counters and the composite score.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LeaderboardEntry {
    pub reader: String,
    pub pages_read: i32,
    pub books_finished: i32,
    pub comments_posted: i32,
    pub reactions_given: i32,
    pub score: i32,
}
