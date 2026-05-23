use serde::{Deserialize, Serialize};

/// One saved point-in-time database snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapshotInfo {
    /// Filename id (e.g. `snap-1716429327000.db`) — opaque to the client.
    pub id: String,
    /// Unix millis when the snapshot was taken.
    pub created_at: f64,
    pub size_bytes: u64,
    pub books: i32,
    pub comments: i32,
    pub reactions: i32,
}

/// A book entry inside a snapshot, used by the per-book restore picker.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapshotBook {
    pub id: String,
    pub title: String,
    pub author: Option<String>,
}
