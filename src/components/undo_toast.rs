use dioxus::prelude::*;

/// Floating "X removed. UNDO" toast above the bottom tab bar.
///
/// The owning page controls `target`: `Some((token, id, label))` shows it,
/// `None` hides. `token` is a monotonically-increasing counter so a stale
/// auto-dismiss can't clear a fresher toast (race protection).
#[component]
pub fn UndoToast(
    mut target: Signal<Option<(u64, String, String)>>,
    on_undo: EventHandler<String>,
) -> Element {
    let v = target.read().clone();
    let Some((_tok, id, label)) = v else {
        return rsx! {};
    };
    let id_for_undo = id.clone();
    rsx! {
        div { class: "fixed bottom-20 left-1/2 -translate-x-1/2 z-50 pop-in flex items-center gap-3 bg-cyber-dark border border-cyber-border rounded-lg px-3 py-2 shadow-lg shadow-black/60",
            span { class: "text-xs text-cyber-text whitespace-nowrap", "{label}" }
            button {
                r#type: "button",
                class: "text-[10px] font-bold tracking-wider uppercase text-neon-cyan press-scale",
                onclick: move |_| {
                    on_undo.call(id_for_undo.clone());
                    target.set(None);
                },
                "UNDO"
            }
            button {
                r#type: "button",
                "aria-label": "Dismiss",
                class: "text-cyber-dim text-sm leading-none press-scale",
                onclick: move |_| target.set(None),
                "×"
            }
        }
    }
}
