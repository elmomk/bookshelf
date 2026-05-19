use dioxus::prelude::*;

#[component]
pub fn ProgressBar(watched: i32, total: i32) -> Element {
    let pct = if total > 0 {
        ((watched as f64 / total as f64) * 100.0).min(100.0)
    } else {
        0.0
    };

    // Animate transform (GPU: no layout/paint) instead of width.
    let frac = pct / 100.0;
    rsx! {
        div { class: "w-full h-1.5 bg-cyber-dark rounded-full overflow-hidden",
            div {
                class: "h-full w-full rounded-full origin-left transition-transform duration-200 [transition-timing-function:cubic-bezier(0.23,1,0.32,1)] {color_class(pct)}",
                style: "transform: scaleX({frac})",
            }
        }
    }
}

fn color_class(pct: f64) -> &'static str {
    if pct >= 100.0 {
        "bg-neon-green shadow-[0_0_6px_theme(colors.neon-green)]"
    } else if pct >= 50.0 {
        "bg-neon-cyan shadow-[0_0_6px_theme(colors.neon-cyan)]"
    } else {
        "bg-neon-purple shadow-[0_0_6px_theme(colors.neon-purple)]"
    }
}
