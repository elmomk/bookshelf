use dioxus::prelude::*;

use crate::cache::{self, SyncStatus};

#[component]
pub fn SyncIndicator(
    status: Signal<SyncStatus>,
    on_sync: EventHandler<()>,
) -> Element {
    let last_sync = cache::read_sync_time();

    let label = match *status.read() {
        SyncStatus::Synced => format_sync_time(last_sync),
        SyncStatus::Syncing => "SYNCING...".to_string(),
        SyncStatus::CachedOnly => "OFFLINE".to_string(),
    };

    let (dot_color, dot_anim) = match *status.read() {
        SyncStatus::Synced => ("bg-neon-green", ""),
        SyncStatus::Syncing => ("bg-neon-cyan", "animate-pulse"),
        SyncStatus::CachedOnly => ("bg-neon-orange", ""),
    };

    rsx! {
        button {
            class: "flex items-center gap-1.5 px-2 py-1 rounded-md transition-colors press-scale",
            onclick: move |_| on_sync.call(()),
            span { class: "w-1.5 h-1.5 rounded-full {dot_color} {dot_anim}" }
            span { class: "text-[10px] text-cyber-dim tracking-wider uppercase font-medium", "{label}" }
        }
    }
}

fn format_sync_time(millis: Option<u64>) -> String {
    let Some(ms) = millis else {
        return "SYNCED".to_string();
    };

    #[cfg(target_arch = "wasm32")]
    {
        let now = js_sys::Date::now() as u64;
        let diff_secs = now.saturating_sub(ms) / 1000;
        if diff_secs < 10 {
            "JUST NOW".to_string()
        } else if diff_secs < 60 {
            format!("{diff_secs}s AGO")
        } else if diff_secs < 3600 {
            format!("{}m AGO", diff_secs / 60)
        } else {
            format!("{}h AGO", diff_secs / 3600)
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = ms;
        "SYNCED".to_string()
    }
}
