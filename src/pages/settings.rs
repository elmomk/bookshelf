use dioxus::prelude::*;

use crate::api::settings as api;
use crate::cache;
use crate::components::error_banner::ErrorBanner;
use crate::components::layout::SyncTrigger;
use crate::models::{ChangeRow, SnapshotBook, SnapshotInfo};

#[component]
pub fn Settings() -> Element {
    let mut current = use_signal(String::new);
    let mut default_name = use_signal(String::new);
    let mut alias_input = use_signal(String::new);
    let mut saving = use_signal(|| false);
    let mut error_msg = use_signal(|| None::<String>);
    let mut saved = use_signal(|| false);

    // History state.
    let mut snapshots = use_signal(Vec::<SnapshotInfo>::new);
    let mut history_busy = use_signal(|| false);
    // The snapshot id whose "Restore everything" is one tap away from firing.
    let mut confirm_full: Signal<Option<String>> = use_signal(|| None);
    // The snapshot id whose per-book picker is expanded, with its book list.
    let mut pick_for: Signal<Option<(String, Vec<SnapshotBook>)>> = use_signal(|| None);

    // Change log state.
    let mut changes = use_signal(Vec::<ChangeRow>::new);
    let mut changes_offset = use_signal(|| 0_u32);
    let mut changes_more = use_signal(|| true);
    let mut changes_busy = use_signal(|| false);
    // Change ids one tap away from firing their "Restore to before this".
    let mut confirm_restore_tx: Signal<Option<i64>> = use_signal(|| None);
    const CHANGES_PAGE: u32 = 30;

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
            if let Ok(list) = api::list_snapshots().await {
                snapshots.set(list);
            }
            if let Ok(page) = api::list_changes(CHANGES_PAGE, 0).await {
                changes_more.set(page.len() as u32 == CHANGES_PAGE);
                changes_offset.set(page.len() as u32);
                changes.set(page);
            }
        });
    });

    let load_more_changes = move |_| {
        spawn(async move {
            changes_busy.set(true);
            let off = *changes_offset.read();
            match api::list_changes(CHANGES_PAGE, off).await {
                Ok(page) => {
                    let got = page.len() as u32;
                    let mut v = changes.read().clone();
                    v.extend(page);
                    changes.set(v);
                    changes_offset.set(off + got);
                    changes_more.set(got == CHANGES_PAGE);
                }
                Err(e) => error_msg.set(Some(format!("Failed to load changes: {e}"))),
            }
            changes_busy.set(false);
        });
    };


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

    let take_snapshot = move |_| {
        spawn(async move {
            history_busy.set(true);
            error_msg.set(None);
            match api::create_snapshot().await {
                Ok(info) => {
                    let mut v = snapshots.read().clone();
                    v.insert(0, info);
                    snapshots.set(v);
                }
                Err(e) => error_msg.set(Some(format!("Snapshot failed: {e}"))),
            }
            history_busy.set(false);
        });
    };

    let cur = current.read().clone();
    let def = default_name.read().clone();
    let has_alias = !cur.is_empty() && cur != def;
    let snaps = snapshots.read().clone();
    let confirm_id_now = confirm_full.read().clone();
    let pick_now = pick_for.read().clone();

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

            div { class: "bg-cyber-card/80 border border-cyber-border rounded-xl p-4 space-y-3",
                p { class: "text-[10px] text-neon-purple tracking-[0.2em] uppercase font-bold", "History" }
                p { class: "text-[9px] text-cyber-dim leading-relaxed",
                    "Snapshots are point-in-time backups of the whole shelf. Restore everything at once, or roll a single book back without touching the others. Every restore quietly takes a fresh snapshot first so it's reversible."
                }

                button {
                    r#type: "button",
                    class: "w-full bg-neon-purple/15 border border-neon-purple/40 text-neon-purple rounded-lg px-3 py-2 text-[10px] font-bold tracking-wider uppercase press-scale disabled:opacity-50",
                    disabled: *history_busy.read(),
                    onclick: take_snapshot,
                    { if *history_busy.read() { "TAKING…" } else { "📸 Take snapshot now" } }
                }

                if snaps.is_empty() {
                    p { class: "text-xs text-cyber-dim text-center py-2", "No snapshots yet." }
                }
                for s in snaps.iter() {
                    {
                        let info = s.clone();
                        let id = info.id.clone();
                        let id_for_full = id.clone();
                        let id_for_pick = id.clone();
                        let id_for_delete = id.clone();
                        let label_id = id.clone();
                        let ts_label = format_ts(info.created_at);
                        let size_label = format_size(info.size_bytes);
                        let is_confirm = confirm_id_now.as_deref() == Some(id.as_str());
                        let pick_open = pick_now.as_ref().map(|(i, _)| i.as_str()) == Some(id.as_str());
                        rsx! {
                            div { class: "border border-cyber-border rounded-lg p-3 space-y-2",
                                div { class: "flex items-baseline justify-between gap-2",
                                    span { class: "text-xs text-neon-cyan font-mono", "{ts_label}" }
                                    span { class: "text-[9px] text-cyber-dim font-mono", "{size_label}" }
                                }
                                div { class: "flex flex-wrap gap-2 text-[10px] text-cyber-dim",
                                    span { "📚 {info.books} books" }
                                    span { "💬 {info.comments} comments" }
                                    span { "👍 {info.reactions} reactions" }
                                }
                                div { class: "flex flex-col gap-1 pt-1",
                                    button {
                                        r#type: "button",
                                        class: if is_confirm {
                                            "w-full bg-neon-magenta/20 border border-neon-magenta text-neon-magenta rounded-md px-3 py-2 text-[10px] font-bold tracking-wider uppercase press-scale"
                                        } else {
                                            "w-full bg-cyber-dark border border-cyber-border text-cyber-text rounded-md px-3 py-2 text-[10px] font-bold tracking-wider uppercase press-scale"
                                        },
                                        disabled: *history_busy.read(),
                                        onclick: {
                                            let id = id_for_full.clone();
                                            let mut sync_trigger = sync_trigger;
                                            move |_| {
                                                let id = id.clone();
                                                let am_confirming = confirm_full.read().as_deref() == Some(id.as_str());
                                                if !am_confirming {
                                                    confirm_full.set(Some(id));
                                                    return;
                                                }
                                                confirm_full.set(None);
                                                spawn(async move {
                                                    history_busy.set(true);
                                                    error_msg.set(None);
                                                    match api::restore_full_from_snapshot(id).await {
                                                        Ok(()) => {
                                                            if let Ok(list) = api::list_snapshots().await {
                                                                snapshots.set(list);
                                                            }
                                                            let cur = sync_trigger.read().0;
                                                            sync_trigger.set(SyncTrigger(cur + 1));
                                                        }
                                                        Err(e) => error_msg.set(Some(format!("Restore failed: {e}"))),
                                                    }
                                                    history_busy.set(false);
                                                });
                                            }
                                        },
                                        { if is_confirm { "Tap again to wipe & restore" } else { "↺ Restore everything" } }
                                    }
                                    button {
                                        r#type: "button",
                                        class: "w-full bg-cyber-dark border border-cyber-border text-cyber-dim rounded-md px-3 py-2 text-[10px] font-bold tracking-wider uppercase press-scale disabled:opacity-50",
                                        disabled: *history_busy.read(),
                                        onclick: {
                                            let id = id_for_pick.clone();
                                            move |_| {
                                                let already = pick_for.read().as_ref().map(|(i, _)| i == &id).unwrap_or(false);
                                                if already {
                                                    pick_for.set(None);
                                                    return;
                                                }
                                                let id2 = id.clone();
                                                spawn(async move {
                                                    history_busy.set(true);
                                                    match api::list_books_in_snapshot(id2.clone()).await {
                                                        Ok(books) => pick_for.set(Some((id2, books))),
                                                        Err(e) => error_msg.set(Some(format!("Couldn't read snapshot: {e}"))),
                                                    }
                                                    history_busy.set(false);
                                                });
                                            }
                                        },
                                        { if pick_open { "× Close book picker" } else { "📖 Restore one book…" } }
                                    }
                                    button {
                                        r#type: "button",
                                        "aria-label": "Delete snapshot",
                                        class: "w-full text-[9px] text-cyber-dim/60 tracking-wider uppercase press-scale",
                                        onclick: {
                                            let id = id_for_delete.clone();
                                            let label = label_id.clone();
                                            move |_| {
                                                let id = id.clone();
                                                let label = label.clone();
                                                spawn(async move {
                                                    if let Err(e) = api::delete_snapshot(id).await {
                                                        error_msg.set(Some(format!("Couldn't delete {label}: {e}")));
                                                        return;
                                                    }
                                                    let mut v = snapshots.read().clone();
                                                    v.retain(|s| s.id != label);
                                                    snapshots.set(v);
                                                });
                                            }
                                        },
                                        "Delete snapshot"
                                    }
                                }
                                if pick_open {
                                    {
                                        let books = pick_now
                                            .as_ref()
                                            .map(|(_, b)| b.clone())
                                            .unwrap_or_default();
                                        rsx! {
                                            div { class: "pop-in pt-1 space-y-1 border-t border-cyber-border/60",
                                                if books.is_empty() {
                                                    p { class: "text-[10px] text-cyber-dim py-1", "This snapshot has no books." }
                                                }
                                                for b in books.iter() {
                                                    {
                                                        let book = b.clone();
                                                        let snap_id = id.clone();
                                                        let mut sync_trigger = sync_trigger;
                                                        rsx! {
                                                            button {
                                                                r#type: "button",
                                                                class: "w-full flex flex-col items-start text-left bg-cyber-dark border border-cyber-border rounded-md px-2 py-1.5 text-[11px] text-cyber-text press-scale disabled:opacity-50",
                                                                disabled: *history_busy.read(),
                                                                onclick: move |_| {
                                                                    let snap_id = snap_id.clone();
                                                                    let book = book.clone();
                                                                    spawn(async move {
                                                                        history_busy.set(true);
                                                                        error_msg.set(None);
                                                                        match api::restore_book_from_snapshot(snap_id, book.id.clone()).await {
                                                                            Ok(()) => {
                                                                                pick_for.set(None);
                                                                                let cur = sync_trigger.read().0;
                                                                                sync_trigger.set(SyncTrigger(cur + 1));
                                                                            }
                                                                            Err(e) => error_msg.set(Some(format!("Restore failed: {e}"))),
                                                                        }
                                                                        history_busy.set(false);
                                                                    });
                                                                },
                                                                span { class: "font-semibold", "{book.title}" }
                                                                if let Some(a) = &book.author {
                                                                    span { class: "text-[9px] text-cyber-dim", "{a}" }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            div { class: "bg-cyber-card/80 border border-cyber-border rounded-xl p-4 space-y-3",
                p { class: "text-[10px] text-neon-pink tracking-[0.2em] uppercase font-bold", "Change log" }
                p { class: "text-[9px] text-cyber-dim leading-relaxed",
                    "Every change to books, comments, reactions, progress and your alias is captured here. Undo just one change if it's still the latest for its row, or roll back to before the whole transaction it belonged to. A safety snapshot is taken first so a wrong rollback is itself reversible."
                }

                if changes.read().is_empty() {
                    p { class: "text-xs text-cyber-dim text-center py-2", "No changes recorded yet." }
                }

                for c in changes.read().clone().iter() {
                    {
                        let id = c.id;
                        let tx_id = c.tx_id;
                        let ts = c.ts;
                        let actor = c.actor.clone().unwrap_or_default();
                        let label = c.label.clone().unwrap_or_default();
                        let op = c.op.clone();
                        let tbl = c.tbl.clone().unwrap_or_default();
                        let is_row_op = matches!(op.as_str(), "INSERT" | "UPDATE" | "DELETE");
                        let is_confirm = *confirm_restore_tx.read() == Some(tx_id);
                        let op_cls = match op.as_str() {
                            "INSERT" => "text-neon-green",
                            "UPDATE" => "text-neon-cyan",
                            "DELETE" => "text-neon-magenta",
                            _ => "text-neon-orange",
                        };
                        let op_label = if tbl.is_empty() {
                            op.clone()
                        } else {
                            format!("{op} · {tbl}")
                        };
                        rsx! {
                            div { class: "border border-cyber-border rounded-lg p-2 space-y-1",
                                div { class: "flex items-baseline justify-between gap-2 text-[10px]",
                                    span { class: "text-cyber-dim font-mono", {format_ts(ts)} }
                                    if !actor.is_empty() {
                                        span { class: "text-cyber-text", "{actor}" }
                                    }
                                    span { class: "ml-auto font-mono {op_cls}", "{op_label}" }
                                }
                                if !label.is_empty() {
                                    p { class: "text-[11px] text-cyber-text break-words", "{label}" }
                                }
                                if is_row_op {
                                    div { class: "flex flex-col gap-1 pt-1",
                                        button {
                                            r#type: "button",
                                            class: "w-full bg-cyber-dark border border-cyber-border text-cyber-text rounded-md px-3 py-1.5 text-[10px] font-bold tracking-wider uppercase press-scale disabled:opacity-50",
                                            disabled: *changes_busy.read(),
                                            onclick: {
                                                let mut sync_trigger = sync_trigger;
                                                move |_| {
                                                    spawn(async move {
                                                        changes_busy.set(true);
                                                        error_msg.set(None);
                                                        match api::undo_change(id).await {
                                                            Ok(()) => {
                                                                if let Ok(page) = api::list_changes(CHANGES_PAGE, 0).await {
                                                                    changes_more.set(page.len() as u32 == CHANGES_PAGE);
                                                                    changes_offset.set(page.len() as u32);
                                                                    changes.set(page);
                                                                }
                                                                if let Ok(ls) = api::list_snapshots().await {
                                                                    snapshots.set(ls);
                                                                }
                                                                let cur = sync_trigger.read().0;
                                                                sync_trigger.set(SyncTrigger(cur + 1));
                                                            }
                                                            Err(e) => error_msg.set(Some(format!("Undo failed: {e}"))),
                                                        }
                                                        changes_busy.set(false);
                                                    });
                                                }
                                            },
                                            "↶ Undo this change"
                                        }
                                        button {
                                            r#type: "button",
                                            class: if is_confirm {
                                                "w-full bg-neon-magenta/20 border border-neon-magenta text-neon-magenta rounded-md px-3 py-1.5 text-[10px] font-bold tracking-wider uppercase press-scale"
                                            } else {
                                                "w-full bg-cyber-dark border border-cyber-border text-cyber-dim rounded-md px-3 py-1.5 text-[10px] font-bold tracking-wider uppercase press-scale disabled:opacity-50"
                                            },
                                            disabled: *changes_busy.read(),
                                            onclick: {
                                                let mut sync_trigger = sync_trigger;
                                                move |_| {
                                                    let am_confirming = *confirm_restore_tx.read() == Some(tx_id);
                                                    if !am_confirming {
                                                        confirm_restore_tx.set(Some(tx_id));
                                                        return;
                                                    }
                                                    confirm_restore_tx.set(None);
                                                    spawn(async move {
                                                        changes_busy.set(true);
                                                        error_msg.set(None);
                                                        match api::restore_to_before_tx(tx_id).await {
                                                            Ok(()) => {
                                                                if let Ok(page) = api::list_changes(CHANGES_PAGE, 0).await {
                                                                    changes_more.set(page.len() as u32 == CHANGES_PAGE);
                                                                    changes_offset.set(page.len() as u32);
                                                                    changes.set(page);
                                                                }
                                                                if let Ok(ls) = api::list_snapshots().await {
                                                                    snapshots.set(ls);
                                                                }
                                                                let cur = sync_trigger.read().0;
                                                                sync_trigger.set(SyncTrigger(cur + 1));
                                                            }
                                                            Err(e) => error_msg.set(Some(format!("Restore failed: {e}"))),
                                                        }
                                                        changes_busy.set(false);
                                                    });
                                                }
                                            },
                                            { if is_confirm { "Tap again to roll back to before this" } else { "↺ Restore to before this" } }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if *changes_more.read() {
                    button {
                        r#type: "button",
                        class: "w-full bg-cyber-dark border border-cyber-border text-cyber-dim rounded-md px-3 py-2 text-[10px] font-bold tracking-wider uppercase press-scale disabled:opacity-50",
                        disabled: *changes_busy.read(),
                        onclick: load_more_changes,
                        { if *changes_busy.read() { "LOADING…" } else { "Load more" } }
                    }
                }
            }
        }
    }
}

/// `2026-05-23 03:14` from a unix-millis timestamp.
fn format_ts(ms: f64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms as i64)
        .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "—".to_string())
}

/// Human-friendly file size.
fn format_size(b: u64) -> String {
    if b >= 1_048_576 {
        format!("{:.1}M", b as f64 / 1_048_576.0)
    } else if b >= 1024 {
        format!("{}K", b / 1024)
    } else {
        format!("{b}B")
    }
}
