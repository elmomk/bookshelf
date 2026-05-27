use dioxus::prelude::*;

use crate::components::icons::*;
use crate::route::Route;

#[component]
pub fn TabBar() -> Element {
    let route: Route = use_route();

    let tabs: Vec<(Route, &str, Element)> = vec![
        (Route::Books {}, "Shelf", rsx! { BookIcon { class: "w-5 h-5".to_string() } }),
        (Route::Leaderboard {}, "Compete", rsx! { TrophyIcon { class: "w-5 h-5".to_string() } }),
        (Route::Activity {}, "Activity", rsx! { ActivityIcon { class: "w-5 h-5".to_string() } }),
    ];

    rsx! {
        nav { class: "fixed bottom-0 left-0 right-0 z-50 bg-cyber-dark/90 backdrop-blur-lg border-t border-neon-cyan/20 safe-bottom",
            div { class: "flex justify-around items-center h-16 max-w-lg mx-auto",
                for (target, label, icon) in tabs {
                    { render_tab(target, label, icon, &route) }
                }
            }
        }
    }
}

fn render_tab(target: Route, label: &str, icon: Element, current: &Route) -> Element {
    let is_active = std::mem::discriminant(&target) == std::mem::discriminant(current);
    let color = if is_active {
        "text-neon-cyan text-glow-cyan [filter:drop-shadow(0_0_4px_currentColor)]"
    } else {
        "text-cyber-dim"
    };

    rsx! {
        Link {
            to: target,
            class: "flex flex-col items-center gap-0.5 px-2 py-2.5 {color} transition-color press-scale",
            {icon}
            span { class: "text-[10px] font-medium tracking-wider uppercase", "{label}" }
        }
    }
}
