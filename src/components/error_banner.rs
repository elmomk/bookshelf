use dioxus::prelude::*;

#[component]
pub fn ErrorBanner(message: Signal<Option<String>>) -> Element {
    let msg = message.read().clone();

    if let Some(text) = msg {
        let mut message = message;
        rsx! {
            div { class: "bg-neon-magenta/10 border border-neon-magenta/40 text-neon-magenta rounded-lg px-4 py-2 text-xs font-mono flex items-center gap-2 glow-magenta banner-enter",
                span { class: "flex-1", "{text}" }
                button {
                    class: "w-10 h-10 flex items-center justify-center text-neon-magenta font-bold flex-shrink-0 rounded-md transition-colors press-scale",
                    onclick: move |_| message.set(None),
                    "×"
                }
            }
        }
    } else {
        rsx! {}
    }
}
