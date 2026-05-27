use dioxus::prelude::*;

use crate::cache::SyncStatus;
use crate::components::icons::SettingsIcon;
use crate::components::notification_bell::NotificationBell;
use crate::components::sync_indicator::SyncIndicator;
use crate::components::tab_bar::TabBar;
use crate::route::Route;

/// Global signal that pages set to trigger a re-sync from the header button.
/// Pages listen for changes to this value and re-fetch when it increments.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SyncTrigger(pub u32);

#[component]
pub fn AppLayout() -> Element {
    let route: Route = use_route();
    let sync_status = use_context_provider(|| Signal::new(SyncStatus::Syncing));
    let mut sync_trigger = use_context_provider(|| Signal::new(SyncTrigger(0)));

    let title = match route {
        Route::Books {} => "THE SHELF",
        Route::BookDetail { .. } => "BOOK",
        Route::Leaderboard {} => "COMPETE",
        Route::Activity {} => "CLUB ACTIVITY",
        Route::Settings {} => "SETTINGS",
        _ => "BOOKCLUB",
    };

    rsx! {
        div { class: "scanlines min-h-screen bg-cyber-black text-cyber-text font-mono",
            // Header
            header { class: "fixed top-0 left-0 right-0 z-50 bg-cyber-dark/90 backdrop-blur-lg border-b border-neon-cyan/20",
                div { class: "flex items-center justify-between h-14 px-4 max-w-lg mx-auto",
                    h1 { class: "text-sm font-bold tracking-[0.12em] text-neon-cyan text-glow-cyan", "{title}" }
                    div { class: "flex items-center gap-1",
                        Link {
                            to: Route::Settings {},
                            class: "p-2 text-cyber-dim hover:text-neon-cyan transition-color press-scale",
                            "aria-label": "Settings",
                            SettingsIcon { class: "w-5 h-5".to_string() }
                        }
                        NotificationBell {}
                        SyncIndicator {
                            status: sync_status,
                            on_sync: move |_| {
                                let cur = sync_trigger.read().0;
                                sync_trigger.set(SyncTrigger(cur + 1));
                            },
                        }
                    }
                }
            }

            // Content
            main { class: "pt-14 pb-20 max-w-lg mx-auto page-enter",
                Outlet::<Route> {}
            }

            TabBar {}
        }
    }
}
