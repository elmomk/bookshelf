use dioxus::prelude::*;

use crate::api::notifications::{
    clear_notifications, get_notification_enabled, list_notifications, mark_notifications_read,
    set_notification_enabled,
};
use crate::components::icons::BellIcon;
use crate::components::layout::SyncTrigger;
use crate::models::notification::NotificationStatus;

#[component]
pub fn NotificationBell() -> Element {
    let mut status = use_signal(|| NotificationStatus {
        notifications: vec![],
        unread_count: 0,
    });
    let mut dropdown_open = use_signal(|| false);
    let sync_trigger = use_context::<Signal<SyncTrigger>>();

    // Fetch on mount + whenever sync trigger changes
    use_effect(move || {
        let _trigger = sync_trigger.read().0;
        spawn(async move {
            if let Ok(s) = list_notifications().await {
                status.set(s);
            }
        });
    });

    let unread = status.read().unread_count;

    rsx! {
        div { class: "relative",
            // Bell button
            button {
                class: "relative flex items-center justify-center w-8 h-8 rounded-md transition-colors press-scale",
                onclick: move |_| {
                    let is_open = *dropdown_open.read();
                    if !is_open {
                        // Opening — mark as read
                        spawn(async move {
                            let _ = mark_notifications_read().await;
                            if let Ok(s) = list_notifications().await {
                                status.set(s);
                            }
                        });
                    }
                    dropdown_open.set(!is_open);
                },
                BellIcon { class: "w-4 h-4 text-cyber-dim".to_string() }
                if unread > 0 {
                    span {
                        class: "absolute -top-0.5 -right-0.5 min-w-[16px] h-4 flex items-center justify-center rounded-full bg-neon-magenta text-[9px] font-bold text-white px-1",
                        "{unread}"
                    }
                }
            }

            if *dropdown_open.read() {
                div {
                    class: "fixed inset-0 z-40",
                    onclick: move |_| dropdown_open.set(false),
                }
                NotificationDropdown {
                    status: status,
                    on_close: move |_| dropdown_open.set(false),
                }
            }
        }
    }
}

#[component]
fn NotificationDropdown(
    mut status: Signal<NotificationStatus>,
    on_close: EventHandler<()>,
) -> Element {
    let mut enabled = use_signal(|| false);

    use_effect(move || {
        spawn(async move {
            if let Ok(e) = get_notification_enabled().await {
                enabled.set(e);
            }
        });
    });

    let notifications = &status.read().notifications;

    rsx! {
        div {
            class: "absolute right-0 top-10 z-50 w-72 max-h-80 overflow-y-auto rounded-lg bg-cyber-dark border border-cyber-border shadow-lg shadow-black/50 dropdown-enter",

            // Header
            div { class: "sticky top-0 bg-cyber-dark border-b border-cyber-border px-3 py-2 flex items-center justify-between",
                span { class: "text-[10px] font-bold tracking-widest text-neon-cyan uppercase", "NOTIFICATIONS" }
                if !notifications.is_empty() {
                    button {
                        class: "text-[9px] text-neon-magenta uppercase tracking-wider hover:text-neon-magenta/70 active:text-neon-magenta/50",
                        onclick: move |_| {
                            spawn(async move {
                                let _ = clear_notifications().await;
                                if let Ok(s) = list_notifications().await {
                                    status.set(s);
                                }
                            });
                        },
                        "CLEAR"
                    }
                }
            }

            if notifications.is_empty() {
                div { class: "px-3 py-6 text-center text-cyber-dim text-xs", "No notifications" }
            } else {
                for notif in notifications.iter() {
                    {
                        let module_color = match notif.module.as_str() {
                            "todo" => "text-neon-cyan",
                            "grocery" => "text-neon-green",
                            "shopee" => "text-neon-orange",
                            "watchlist" => "text-neon-purple",
                            _ => "text-cyber-dim",
                        };
                        let action_label = notif.action.as_str();
                        let elapsed = format_elapsed(notif.created_at);
                        let display_actor = if notif.actor.starts_with("_system:") {
                            notif.actor.trim_start_matches("_system:").to_string()
                        } else {
                            notif.actor.clone()
                        };
                        rsx! {
                            div { class: "px-3 py-2 border-b border-cyber-border/50 hover:bg-cyber-card/30",
                                div { class: "text-xs text-cyber-text leading-snug",
                                    span { class: "font-semibold text-neon-green", "{display_actor}" }
                                    " {action_label} "
                                    span { class: "text-cyber-text/80", "{notif.item_text}" }
                                }
                                div { class: "flex items-center gap-2 mt-0.5",
                                    span { class: "text-[9px] {module_color} uppercase font-bold", "{notif.module}" }
                                    span { class: "text-[9px] text-cyber-dim", "{elapsed}" }
                                }
                            }
                        }
                    }
                }
            }

            // Settings toggle
            div { class: "sticky bottom-0 bg-cyber-dark border-t border-cyber-border px-3 py-2",
                button {
                    class: "flex items-center gap-2 w-full text-left",
                    onclick: move |_| {
                        let new_val = !*enabled.read();
                        enabled.set(new_val);
                        spawn(async move {
                            let _ = set_notification_enabled(new_val).await;
                            if new_val {
                                subscribe_push().await;
                            } else {
                                unsubscribe_push().await;
                            }
                        });
                    },
                    div {
                        class: if *enabled.read() { "w-7 h-4 rounded-full bg-neon-cyan/30 relative transition-colors" } else { "w-7 h-4 rounded-full bg-cyber-border relative transition-colors" },
                        div {
                            class: if *enabled.read() { "absolute top-0.5 right-0.5 w-3 h-3 rounded-full bg-neon-cyan transition-transform" } else { "absolute top-0.5 left-0.5 w-3 h-3 rounded-full bg-cyber-dim transition-transform" },
                        }
                    }
                    span { class: "text-[10px] text-cyber-dim uppercase tracking-wider", "Push notifications" }
                }
            }
        }
    }
}

/// Subscribe to web push: request permission, get subscription, send to server.
async fn subscribe_push() {
    #[cfg(target_arch = "wasm32")]
    {
        use crate::api::notifications::{get_vapid_public_key, save_push_subscription};
        let vapid_key = match get_vapid_public_key().await {
            Ok(k) if !k.is_empty() => k,
            _ => return,
        };

        // Use eval to interact with the Push API (not available via web-sys features we have)
        let mut eval = document::eval(&format!(
            r#"
            try {{
                const permission = await Notification.requestPermission();
                if (permission !== 'granted') {{
                    dioxus.send(JSON.stringify({{ error: 'Permission denied' }}));
                    return;
                }}

                const reg = await navigator.serviceWorker.ready;

                // Convert VAPID public key from base64url to Uint8Array
                const vapidKey = '{vapid_key}';
                const padding = '='.repeat((4 - vapidKey.length % 4) % 4);
                const base64 = (vapidKey + padding).replace(/-/g, '+').replace(/_/g, '/');
                const rawData = atob(base64);
                const outputArray = new Uint8Array(rawData.length);
                for (let i = 0; i < rawData.length; ++i) {{
                    outputArray[i] = rawData.charCodeAt(i);
                }}

                const subscription = await reg.pushManager.subscribe({{
                    userVisibleOnly: true,
                    applicationServerKey: outputArray,
                }});

                const json = subscription.toJSON();
                dioxus.send(JSON.stringify({{
                    endpoint: json.endpoint,
                    p256dh: json.keys.p256dh,
                    auth: json.keys.auth,
                }}));
            }} catch (e) {{
                dioxus.send(JSON.stringify({{ error: e.message }}));
            }}
            "#
        ));

        if let Ok(val) = eval.recv::<serde_json::Value>().await {
            if let Some(endpoint) = val.get("endpoint").and_then(|v: &serde_json::Value| v.as_str()) {
                let p256dh = val.get("p256dh").and_then(|v: &serde_json::Value| v.as_str()).unwrap_or("");
                let auth = val.get("auth").and_then(|v: &serde_json::Value| v.as_str()).unwrap_or("");
                let _ = save_push_subscription(
                    endpoint.to_string(),
                    p256dh.to_string(),
                    auth.to_string(),
                )
                .await;
            }
        }
    }
}

/// Unsubscribe from web push.
async fn unsubscribe_push() {
    #[cfg(target_arch = "wasm32")]
    {
        use crate::api::notifications::remove_push_subscription;
        let mut eval = document::eval(
            r#"
            try {
                const reg = await navigator.serviceWorker.ready;
                const subscription = await reg.pushManager.getSubscription();
                if (subscription) {
                    await subscription.unsubscribe();
                }
                dioxus.send("ok");
            } catch (e) {
                dioxus.send("error");
            }
            "#,
        );
        let _ = eval.recv::<String>().await;
        let _ = remove_push_subscription().await;
    }
}

fn format_elapsed(created_at: f64) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let now = js_sys::Date::now();
        let diff_secs = ((now - created_at) / 1000.0) as u64;
        if diff_secs < 60 {
            "just now".to_string()
        } else if diff_secs < 3600 {
            format!("{}m ago", diff_secs / 60)
        } else if diff_secs < 86400 {
            format!("{}h ago", diff_secs / 3600)
        } else {
            format!("{}d ago", diff_secs / 86400)
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = created_at;
        String::new()
    }
}
