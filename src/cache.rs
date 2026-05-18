use serde::{de::DeserializeOwned, Serialize};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SyncStatus {
    Synced,
    Syncing,
    CachedOnly,
}

/// Read a value from localStorage (WASM only, no-op on server).
pub fn read<T: DeserializeOwned>(key: &str) -> Option<T> {
    #[cfg(target_arch = "wasm32")]
    {
        let storage = web_sys::window()?.local_storage().ok()??;
        let json = storage.get_item(&format!("bc_{key}")).ok()??;
        serde_json::from_str(&json).ok()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = key;
        None
    }
}

/// Write a value to localStorage (WASM only, no-op on server).
pub fn write<T: Serialize>(key: &str, data: &T) {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(storage) = web_sys::window()
            .and_then(|w| w.local_storage().ok())
            .flatten()
        {
            if let Ok(json) = serde_json::to_string(data) {
                let _ = storage.set_item(&format!("bc_{key}"), &json);
            }
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (key, data);
    }
}

/// Write the last-synced timestamp to localStorage.
pub fn write_sync_time() {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(storage) = web_sys::window()
            .and_then(|w| w.local_storage().ok())
            .flatten()
        {
            let now = js_sys::Date::now() as u64;
            let _ = storage.set_item("bc_last_sync", &now.to_string());
        }
    }
}

/// Read the last-synced timestamp from localStorage (millis since epoch).
pub fn read_sync_time() -> Option<u64> {
    #[cfg(target_arch = "wasm32")]
    {
        let storage = web_sys::window()?.local_storage().ok()??;
        let val = storage.get_item("bc_last_sync").ok()??;
        val.parse().ok()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        None
    }
}
