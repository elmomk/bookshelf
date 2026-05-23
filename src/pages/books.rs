use dioxus::prelude::*;

use crate::api::books as api;
use crate::cache::{self, SyncStatus};
use crate::components::error_banner::ErrorBanner;
use crate::components::layout::SyncTrigger;
use crate::components::swipe_item::SwipeItem;
use crate::components::undo_toast::UndoToast;
use crate::models::{Book, BookSearchResult, ReadingStatus};
use crate::route::Route;
use crate::util::anim_sleep;

#[component]
pub fn Books() -> Element {
    let mut books = use_signal(Vec::<Book>::new);
    let mut input_text = use_signal(String::new);
    let mut search_results = use_signal(Vec::<BookSearchResult>::new);
    let mut searching = use_signal(|| false);
    let mut error_msg = use_signal(|| None::<String>);
    // Undo state for the last book removal (token, id, label).
    let undo_target = use_signal(|| None::<(u64, String, String)>);
    let undo_seq = use_signal(|| 0_u64);

    let mut sync_status = use_context::<Signal<SyncStatus>>();
    let sync_trigger = use_context::<Signal<SyncTrigger>>();

    let reload = move || {
        spawn(async move {
            sync_status.set(SyncStatus::Syncing);
            match api::list_books().await {
                Ok(loaded) => {
                    cache::write("books", &loaded);
                    cache::write_sync_time();
                    books.set(loaded);
                    sync_status.set(SyncStatus::Synced);
                }
                Err(e) => {
                    if books.read().is_empty() {
                        error_msg.set(Some(format!("Failed to load: {e}")));
                    }
                    sync_status.set(SyncStatus::CachedOnly);
                }
            }
        });
    };

    use_effect(move || {
        if let Some(cached) = cache::read::<Vec<Book>>("books") {
            books.set(cached);
        }
        reload();
    });

    use_effect(move || {
        let _t = sync_trigger.read().0;
        reload();
    });

    let do_search = move |q: String| {
        spawn(async move {
            if q.trim().is_empty() {
                search_results.set(vec![]);
                searching.set(false);
                return;
            }
            searching.set(true);
            match api::search_books(q).await {
                Ok(r) => search_results.set(r),
                Err(_) => search_results.set(vec![]),
            }
            searching.set(false);
        });
    };

    let add_from_search = move |r: BookSearchResult| {
        input_text.set(String::new());
        search_results.set(vec![]);
        spawn(async move {
            if let Err(e) = api::add_book(r).await {
                error_msg.set(Some(format!("Failed to add: {e}")));
            }
            reload();
        });
    };

    let mut add_manual = move |title: String| {
        if title.trim().is_empty() {
            return;
        }
        input_text.set(String::new());
        search_results.set(vec![]);
        spawn(async move {
            if let Err(e) = api::add_book_manual(title, None, None, None).await {
                error_msg.set(Some(format!("Failed to add: {e}")));
            }
            reload();
        });
    };

    rsx! {
        div { class: "px-4 py-4 space-y-4",
            ErrorBanner { message: error_msg }

            // Search / add
            div { class: "bg-cyber-card/80 border border-cyber-border rounded-xl p-4",
                form {
                    class: "space-y-3",
                    onsubmit: move |e| {
                        e.prevent_default();
                        let t = input_text.read().clone();
                        add_manual(t);
                    },
                    input {
                        class: "w-full bg-cyber-dark border border-cyber-border rounded-lg px-4 py-2.5 text-sm text-cyber-text outline-none focus:border-neon-cyan/60 font-mono",
                        r#type: "text",
                        placeholder: "Search a book to add to the shelf...",
                        value: "{input_text}",
                        oninput: move |e| {
                            let v = e.value();
                            input_text.set(v.clone());
                            do_search(v);
                        },
                    }

                    if !search_results.read().is_empty() {
                        div { class: "bg-cyber-dark border border-cyber-border rounded-lg overflow-hidden max-h-72 overflow-y-auto",
                            for r in search_results.read().iter() {
                                {render_search_hit(r.clone(), add_from_search)}
                            }
                        }
                    }

                    if *searching.read() {
                        div { class: "text-center py-2",
                            span { class: "text-[10px] text-neon-cyan tracking-wider animate-pulse", "SEARCHING..." }
                        }
                    }

                    if search_results.read().is_empty() && !input_text.read().trim().is_empty() && !*searching.read() {
                        button {
                            class: "w-full bg-neon-cyan/10 text-neon-cyan/80 border border-neon-cyan/20 rounded-lg px-4 py-2 text-xs font-bold tracking-wider uppercase transition-colors press-scale",
                            r#type: "submit",
                            "ADD “{input_text}” MANUALLY"
                        }
                    }
                }
            }

            // Shelf
            div { class: "space-y-0",
                for book in books.read().iter() {
                    {render_book_card(book.clone(), reload, error_msg, undo_target, undo_seq)}
                }
                if books.read().is_empty() {
                    div { class: "text-center py-16",
                        p { class: "text-2xl mb-3 opacity-30", "📚" }
                        p { class: "text-xs tracking-[0.3em] uppercase text-cyber-dim",
                            "The shelf is empty — search a book above"
                        }
                    }
                }
                if !books.read().is_empty() {
                    p { class: "text-center text-[9px] text-cyber-dim/30 tracking-widest mt-3 pb-1",
                        "← REMOVE • TAP TO OPEN • SWIPE → ADVANCE"
                    }
                }
            }
        }
        UndoToast {
            target: undo_target,
            on_undo: {
                let reload = reload.clone();
                move |id: String| {
                    let rl = reload.clone();
                    spawn(async move {
                        if let Err(e) = api::undo_delete_book(id).await {
                            error_msg.set(Some(format!("Undo failed: {e}")));
                        }
                        rl();
                    });
                }
            },
        }
    }
}

fn render_search_hit(
    r: BookSearchResult,
    mut on_add: impl FnMut(BookSearchResult) + Clone + 'static,
) -> Element {
    let r2 = r.clone();
    rsx! {
        button {
            class: "flex items-center gap-3 w-full text-left px-3 py-2 border-b border-cyber-border/50 hover:bg-cyber-card/40 transition-colors",
            onclick: move |_| on_add(r2.clone()),
            if let Some(cover) = &r.cover_url {
                img { class: "w-8 h-12 object-cover rounded shrink-0", src: "{cover}" }
            } else {
                div { class: "w-8 h-12 rounded bg-cyber-dark shrink-0 flex items-center justify-center text-cyber-dim text-xs", "?" }
            }
            div { class: "min-w-0 flex-1",
                p { class: "text-xs text-cyber-text truncate", "{r.title}" }
                if let Some(a) = &r.author {
                    p { class: "text-[10px] text-cyber-dim truncate", "{a}" }
                }
            }
            if let Some(p) = r.total_pages {
                span { class: "text-[9px] text-cyber-dim shrink-0", "{p}p" }
            }
        }
    }
}

fn status_chip(status: &ReadingStatus) -> Element {
    let (cls, label) = match status {
        ReadingStatus::Reading => (
            "bg-neon-cyan/15 border-neon-cyan/40 text-neon-cyan",
            "READING",
        ),
        ReadingStatus::Finished => (
            "bg-neon-green/15 border-neon-green/40 text-neon-green",
            "FINISHED",
        ),
        ReadingStatus::ToRead => (
            "bg-neon-purple/15 border-neon-purple/40 text-neon-purple",
            "TO READ",
        ),
    };
    rsx! {
        span { class: "px-2 py-0.5 rounded border text-[9px] font-bold tracking-wider {cls}", "{label}" }
    }
}

fn render_book_card(
    book: Book,
    reload: impl FnMut() + Clone + 'static,
    mut error_msg: Signal<Option<String>>,
    mut undo_target: Signal<Option<(u64, String, String)>>,
    mut undo_seq: Signal<u64>,
) -> Element {
    let nav = navigator();
    let id_for_nav = book.id.clone();
    let card_key = book.id.clone();

    let my_status = book
        .my_progress
        .as_ref()
        .map(|p| p.status.clone())
        .unwrap_or(ReadingStatus::ToRead);

    // swipe right → advance my status
    let reload_r = reload.clone();
    let book_r = book.clone();
    let on_right = move |_| {
        let next = my_status.next();
        let (page, chapter) = book_r
            .my_progress
            .as_ref()
            .map(|p| (p.current_page, p.current_chapter))
            .unwrap_or((None, None));
        let bid = book_r.id.clone();
        let mut rl = reload_r.clone();
        spawn(async move {
            if let Err(e) = api::set_reading_progress(bid, page, chapter, next).await {
                error_msg.set(Some(format!("Failed: {e}")));
            }
            rl();
        });
    };

    // swipe left → soft-remove from shelf with Undo
    let reload_l = reload.clone();
    let del_id = book.id.clone();
    let del_title = book.title.clone();
    let on_left = move |_| {
        let bid = del_id.clone();
        let title = del_title.clone();
        let mut rl = reload_l.clone();
        spawn(async move {
            match api::delete_book(bid.clone()).await {
                Ok(()) => {
                    // Bump the token first, capture it, then schedule a
                    // delayed clear that only fires if no newer delete has
                    // replaced this toast.
                    let tok = {
                        let mut s = undo_seq.write();
                        *s += 1;
                        *s
                    };
                    undo_target.set(Some((
                        tok,
                        bid,
                        format!("📖 “{title}” removed"),
                    )));
                    rl();
                    spawn(async move {
                        anim_sleep(6000).await;
                        let still_mine = undo_target
                            .read()
                            .as_ref()
                            .is_some_and(|(t, _, _)| *t == tok);
                        if still_mine {
                            undo_target.set(None);
                        }
                    });
                }
                Err(e) => {
                    error_msg.set(Some(format!("Failed to delete: {e}")));
                    rl();
                }
            }
        });
    };

    let readers: Vec<_> = book.progress.iter().filter(|p| p.status != ReadingStatus::ToRead).collect();
    let cur_status = book
        .my_progress
        .as_ref()
        .map(|p| p.status.clone())
        .unwrap_or(ReadingStatus::ToRead);

    rsx! {
        SwipeItem {
            key: "{card_key}",
            on_swipe_right: Some(EventHandler::new(on_right)),
            on_swipe_left: EventHandler::new(on_left),
            completed: cur_status == ReadingStatus::Finished,
            div {
                class: "flex gap-3 cursor-pointer",
                onclick: move |_| { nav.push(Route::BookDetail { id: id_for_nav.clone() }); },
                if let Some(cover) = &book.cover_url {
                    img { class: "w-12 h-[72px] object-cover rounded shrink-0", src: "{cover}" }
                } else {
                    div { class: "w-12 h-[72px] rounded bg-cyber-dark shrink-0 flex items-center justify-center text-cyber-dim", "📖" }
                }
                div { class: "min-w-0 flex-1",
                    p { class: "text-sm font-medium text-cyber-text leading-snug line-clamp-2", "{book.title}" }
                    if let Some(a) = &book.author {
                        p { class: "text-[11px] text-cyber-dim truncate mt-0.5", "{a}" }
                    }
                    div { class: "flex items-center gap-2 mt-1.5 flex-wrap",
                        {status_chip(&cur_status)}
                        if book.comment_count > 0 {
                            span { class: "text-[10px] text-neon-orange", "💬 {book.comment_count}" }
                        }
                    }
                    if !readers.is_empty() {
                        div { class: "flex items-center gap-1.5 mt-1.5 flex-wrap",
                            for p in readers.iter() {
                                span { class: "text-[9px] text-cyber-dim bg-cyber-dark border border-cyber-border rounded px-1.5 py-0.5",
                                    {reader_progress_label(p, &book)}
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn reader_progress_label(p: &crate::models::ReadingProgress, book: &Book) -> String {
    let who = p.reader.clone();
    match p.status {
        ReadingStatus::Finished => format!("{who} ✓"),
        ReadingStatus::Reading => match book.reading_fraction(p) {
            Some((cur, tot)) if tot > 0 => {
                format!("{who} {}%", (cur as f64 / tot as f64 * 100.0).round() as i32)
            }
            _ => format!("{who} reading"),
        },
        ReadingStatus::ToRead => format!("{who} …"),
    }
}
