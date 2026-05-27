use dioxus::prelude::*;

use crate::api::books as books_api;
use crate::api::leaderboard as api;
use crate::cache::{self, SyncStatus};
use crate::components::error_banner::ErrorBanner;
use crate::components::layout::SyncTrigger;
use crate::models::{LeaderboardEntry, LeaderboardWindow};

#[component]
pub fn Leaderboard() -> Element {
    let window = use_signal(|| LeaderboardWindow::Last7Days);
    let mut rows = use_signal(Vec::<LeaderboardEntry>::new);
    let mut error_msg = use_signal(|| None::<String>);
    let mut me = use_signal(String::new);

    let mut sync_status = use_context::<Signal<SyncStatus>>();
    let sync_trigger = use_context::<Signal<SyncTrigger>>();

    let reload = move || {
        let w = *window.read();
        spawn(async move {
            sync_status.set(SyncStatus::Syncing);
            match api::get_leaderboard(w).await {
                Ok(loaded) => {
                    cache::write(&format!("leaderboard_{}", w.tag()), &loaded);
                    cache::write_sync_time();
                    rows.set(loaded);
                    sync_status.set(SyncStatus::Synced);
                }
                Err(e) => {
                    if rows.read().is_empty() {
                        error_msg.set(Some(format!("Failed to load: {e}")));
                    }
                    sync_status.set(SyncStatus::CachedOnly);
                }
            }
        });
    };

    // First mount: paint from cache (if any) and identify the viewer, then
    // fetch fresh data.
    use_effect(move || {
        let w = *window.read();
        if let Some(cached) =
            cache::read::<Vec<LeaderboardEntry>>(&format!("leaderboard_{}", w.tag()))
        {
            rows.set(cached);
        }
        spawn(async move {
            if let Ok(name) = books_api::whoami().await {
                me.set(name);
            }
        });
        reload();
    });

    // Sync button in the header re-pulls the board.
    use_effect(move || {
        let _t = sync_trigger.read().0;
        reload();
    });

    rsx! {
        div { class: "px-4 py-5 space-y-4",
            ErrorBanner { message: error_msg }

            // Window selector
            div { class: "bg-cyber-card/80 border border-cyber-border rounded-xl p-4 space-y-3",
                p { class: "text-[10px] text-neon-cyan tracking-[0.2em] uppercase font-bold", "Window" }
                div { class: "flex gap-2",
                    {window_button(LeaderboardWindow::Last7Days,  "7 Days",   window)}
                    {window_button(LeaderboardWindow::Last30Days, "30 Days",  window)}
                    {window_button(LeaderboardWindow::AllTime,    "All Time", window)}
                }
            }

            // Board
            {render_board(rows.read().as_slice(), me.read().as_str())}
        }
    }
}

fn window_button(
    target: LeaderboardWindow,
    label: &str,
    mut window: Signal<LeaderboardWindow>,
) -> Element {
    let active = *window.read() == target;
    let cls = if active {
        "bg-neon-cyan/20 border-neon-cyan/60 text-neon-cyan"
    } else {
        "bg-cyber-dark border-cyber-border text-cyber-dim"
    };
    rsx! {
        button {
            r#type: "button",
            class: "flex-1 rounded-lg border px-2 py-2 text-[10px] font-bold tracking-wider uppercase press-scale {cls}",
            onclick: move |_| window.set(target),
            "{label}"
        }
    }
}

fn render_board(rows: &[LeaderboardEntry], me: &str) -> Element {
    if rows.is_empty() {
        return rsx! {
            div { class: "bg-cyber-card/80 border border-cyber-border rounded-xl p-6 text-center",
                p { class: "text-sm text-cyber-dim", "No activity yet — go read something." }
            }
        };
    }
    if rows.len() == 1 {
        let solo = rows[0].clone();
        return rsx! {
            div { class: "space-y-3",
                div { class: "bg-cyber-card/80 border border-cyber-border rounded-xl p-4 text-center",
                    p { class: "text-sm text-cyber-dim",
                        span { class: "text-neon-green font-semibold", "{solo.reader}" }
                        " is reading alone. Join in!"
                    }
                }
                {board_card(0, &solo, me)}
            }
        };
    }
    rsx! {
        div { class: "space-y-3",
            for (i, r) in rows.iter().enumerate() {
                {board_card(i, r, me)}
            }
        }
    }
}

fn board_card(rank_idx: usize, r: &LeaderboardEntry, me: &str) -> Element {
    let is_me = r.reader == me && !me.is_empty();
    let frame = if is_me {
        "bg-cyber-card/80 border-2 border-neon-cyan/60 rounded-xl p-4 space-y-2"
    } else {
        "bg-cyber-card/80 border border-cyber-border rounded-xl p-4 space-y-2"
    };
    let rank_chip = match rank_idx {
        0 => rsx! { span { class: "text-2xl leading-none", "🥇" } },
        1 => rsx! { span { class: "text-2xl leading-none", "🥈" } },
        2 => rsx! { span { class: "text-2xl leading-none", "🥉" } },
        n => rsx! {
            span { class: "text-[11px] font-bold text-cyber-dim tracking-wider", "#{n + 1}" }
        },
    };
    rsx! {
        div { class: "{frame}",
            div { class: "flex items-center gap-3",
                div { class: "w-7 flex justify-center", {rank_chip} }
                div { class: "flex-1 min-w-0",
                    p { class: "text-sm font-semibold text-neon-green truncate", "{r.reader}" }
                }
                div { class: "text-right",
                    p { class: "text-lg font-bold text-neon-cyan leading-none", "{r.score}" }
                    p { class: "text-[9px] text-cyber-dim tracking-wider uppercase mt-0.5", "Score" }
                }
            }
            div { class: "flex flex-wrap gap-1.5 pl-10",
                {chip("📖", r.pages_read, "pages", "text-neon-purple")}
                {chip("✅", r.books_finished, "finished", "text-neon-green")}
                {chip("💬", r.comments_posted, "comments", "text-neon-orange")}
                {chip("👍", r.reactions_given, "reactions", "text-neon-magenta")}
            }
        }
    }
}

fn chip(emoji: &str, n: i32, label: &str, color: &str) -> Element {
    rsx! {
        span { class: "inline-flex items-center gap-1 bg-cyber-dark/80 border border-cyber-border rounded-md px-2 py-0.5 text-[10px] font-mono",
            span { "{emoji}" }
            span { class: "{color} font-bold", "{n}" }
            span { class: "text-cyber-dim", "{label}" }
        }
    }
}
