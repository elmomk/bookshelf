use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub id: String,
    pub actor: String,
    pub action: String,
    pub module: String,
    pub item_text: String,
    pub created_at: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NotificationStatus {
    pub notifications: Vec<Notification>,
    pub unread_count: u32,
}
