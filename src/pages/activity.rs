use dioxus::prelude::*;

use crate::api::books as api;
use crate::cache::SyncStatus;
use crate::components::layout::SyncTrigger;
use crate::models::notification::Notification;

#[component]
pub fn Activity() -> Element {
    let mut items = use_signal(Vec::<Notification>::new);
    let mut sync_status = use_context::<Signal<SyncStatus>>();
    let sync_trigger = use_context::<Signal<SyncTrigger>>();

    let reload = move || {
        spawn(async move {
            sync_status.set(SyncStatus::Syncing);
            match api::list_activity().await {
                Ok(list) => {
                    items.set(list);
                    sync_status.set(SyncStatus::Synced);
                }
                Err(_) => sync_status.set(SyncStatus::CachedOnly),
            }
        });
    };

    use_effect(move || {
        reload();
    });
    use_effect(move || {
        let _t = sync_trigger.read().0;
        reload();
    });

    rsx! {
        div { class: "px-4 py-4 space-y-2",
            p { class: "text-[10px] text-neon-cyan tracking-[0.3em] uppercase font-bold pb-1", "Club Activity" }
            if items.read().is_empty() {
                div { class: "text-center py-16",
                    p { class: "text-2xl mb-3 opacity-30", "📡" }
                    p { class: "text-xs tracking-[0.3em] uppercase text-cyber-dim", "No activity yet" }
                }
            }
            for n in items.read().iter() {
                div { class: "bg-cyber-card/60 border border-cyber-border rounded-lg px-3 py-2",
                    div { class: "text-xs text-cyber-text leading-snug",
                        span { class: "font-semibold text-neon-green", "{n.actor}" }
                        " {n.action} "
                        span { class: "text-cyber-text/80", "{n.item_text}" }
                    }
                    div { class: "flex items-center gap-2 mt-0.5",
                        span { class: "text-[9px] text-neon-purple uppercase font-bold", "{n.module}" }
                        span { class: "text-[9px] text-cyber-dim", {format_ago(n.created_at)} }
                    }
                }
            }
        }
    }
}

fn format_ago(created_at: f64) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let now = js_sys::Date::now();
        let diff = ((now - created_at) / 1000.0).max(0.0) as u64;
        if diff < 60 {
            "just now".to_string()
        } else if diff < 3600 {
            format!("{}m ago", diff / 60)
        } else if diff < 86400 {
            format!("{}h ago", diff / 3600)
        } else {
            format!("{}d ago", diff / 86400)
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = created_at;
        String::new()
    }
}
