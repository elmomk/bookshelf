use serde::{Deserialize, Serialize};

/// One row from the `db_changes` audit log, surfaced to the client for the
/// Settings → Change log UI.
// Phase 0 ships the type; the first caller arrives in Phase 3.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChangeRow {
    /// Auto-increment id; also defines replay order (newest = highest).
    pub id: i64,
    /// Groups all rows from one server-fn call so a multi-table operation
    /// (e.g. `set_alias` rewriting 6 tables) can be undone as a unit.
    pub tx_id: i64,
    /// Unix millis when the change was recorded.
    pub ts: f64,
    /// Display name of the user that triggered the change; `None` for system
    /// actions like the auto pre-restore snapshot.
    pub actor: Option<String>,
    /// Human-readable label, e.g. `"delete_book(13f5…)"`.
    pub label: Option<String>,
    /// `INSERT` | `UPDATE` | `DELETE` for row-level diffs; `restore_full`,
    /// `restore_book`, `undo`, `restore_point` for high-level events.
    pub op: String,
    /// Table the change touched. `None` for full-DB events like `restore_full`.
    pub tbl: Option<String>,
    /// Object form of the primary key — `{"id": "…"}` or for composite keys
    /// `{"comment_id": "…", "reader": "…", "emoji": "…"}`. `None` for high-level.
    pub row_pk_json: Option<String>,
    /// Pre-change row as `{col: value}` JSON. `None` on `INSERT`.
    pub old_json: Option<String>,
    /// Post-change row as `{col: value}` JSON. `None` on `DELETE`.
    pub new_json: Option<String>,
}
