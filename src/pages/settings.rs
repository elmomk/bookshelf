use dioxus::prelude::*;

use crate::api::settings as api;
use crate::cache;
use crate::components::error_banner::ErrorBanner;
use crate::components::layout::SyncTrigger;

#[component]
pub fn Settings() -> Element {
    let mut current = use_signal(String::new);
    let mut default_name = use_signal(String::new);
    let mut alias_input = use_signal(String::new);
    let mut saving = use_signal(|| false);
    let mut error_msg = use_signal(|| None::<String>);
    let mut saved = use_signal(|| false);

    let mut sync_trigger = use_context::<Signal<SyncTrigger>>();

    use_effect(move || {
        spawn(async move {
            match api::get_identity().await {
                Ok((cur, def)) => {
                    if cur != def {
                        alias_input.set(cur.clone());
                    }
                    current.set(cur);
                    default_name.set(def);
                }
                Err(e) => error_msg.set(Some(format!("Failed to load: {e}"))),
            }
        });
    });

    let apply = move |value: String| {
        spawn(async move {
            saving.set(true);
            saved.set(false);
            error_msg.set(None);
            match api::set_alias(value).await {
                Ok(name) => {
                    current.set(name.clone());
                    cache::write("me", &name);
                    saved.set(true);
                    // Nudge other pages to re-sync under the new identity.
                    let cur = sync_trigger.read().0;
                    sync_trigger.set(SyncTrigger(cur + 1));
                }
                Err(e) => error_msg.set(Some(format!("Failed to save: {e}"))),
            }
            saving.set(false);
        });
    };

    let save = move |_| {
        let v = alias_input.read().trim().to_string();
        apply.clone()(v);
    };

    let reset = move |_| {
        alias_input.set(String::new());
        apply.clone()(String::new());
    };

    let cur = current.read().clone();
    let def = default_name.read().clone();
    let has_alias = !cur.is_empty() && cur != def;

    rsx! {
        div { class: "px-4 py-4 space-y-4",
            ErrorBanner { message: error_msg }

            div { class: "bg-cyber-card/80 border border-cyber-border rounded-xl p-4 space-y-3",
                p { class: "text-[10px] text-neon-cyan tracking-[0.2em] uppercase font-bold", "Identity" }

                div { class: "flex items-center justify-between text-xs",
                    span { class: "text-cyber-dim", "Shown to the club as" }
                    span { class: "text-neon-cyan font-bold", "{cur}" }
                }
                if has_alias {
                    div { class: "flex items-center justify-between text-[11px]",
                        span { class: "text-cyber-dim", "Default name" }
                        span { class: "text-cyber-dim", "{def}" }
                    }
                }

                div { class: "space-y-1 pt-1",
                    label { class: "text-[9px] text-cyber-dim uppercase tracking-wider", "Alias" }
                    input {
                        class: "w-full bg-cyber-dark border border-cyber-border rounded-lg px-3 py-2 text-sm text-cyber-text outline-none focus:border-neon-cyan/60 font-mono",
                        r#type: "text",
                        maxlength: "50",
                        placeholder: "{def}",
                        value: "{alias_input}",
                        oninput: move |e| alias_input.set(e.value()),
                    }
                    p { class: "text-[9px] text-cyber-dim leading-relaxed",
                        "Changing this renames your existing comments, reading progress and activity so your history stays under one name."
                    }
                }

                if *saved.read() {
                    p { class: "text-[10px] text-neon-green tracking-wider uppercase", "✓ Saved" }
                }

                button {
                    class: "w-full bg-neon-cyan/15 border border-neon-cyan/40 text-neon-cyan rounded-lg px-4 py-2 text-xs font-bold tracking-wider uppercase press-scale disabled:opacity-50",
                    disabled: *saving.read(),
                    onclick: save,
                    { if *saving.read() { "SAVING…" } else { "SAVE ALIAS" } }
                }
                if has_alias {
                    button {
                        r#type: "button",
                        class: "w-full bg-cyber-dark border border-cyber-border text-cyber-dim rounded-lg px-3 py-2 text-[10px] font-bold tracking-wider uppercase press-scale disabled:opacity-50",
                        disabled: *saving.read(),
                        onclick: reset,
                        "RESET TO DEFAULT ({def})"
                    }
                }
            }
        }
    }
}
