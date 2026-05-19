use std::collections::HashSet;

use dioxus::prelude::*;

use crate::api::books as api;
use crate::cache::{self, SyncStatus};
use crate::components::error_banner::ErrorBanner;
use crate::components::layout::SyncTrigger;
use crate::components::progress_bar::ProgressBar;
use crate::models::{Book, BookComment, ReadingProgress, ReadingStatus, TocEntry, REACTION_EMOJIS};
use crate::route::Route;

/// Multi-select file picker that returns a JSON array of image data URLs via
/// `dioxus.send` (`"[]"` on cancel/error). No `capture` attribute, so it lets
/// the user pick existing photos (gallery / files) rather than forcing the
/// camera on mobile, and `multiple` lets a multi-page ToC be scanned in one go.
///
/// OCR fidelity is everything here: a re-encode to low-quality JPEG turns dot
/// leaders into letter-soup that no parser can recover. So small files are
/// sent **untouched** (camera-native quality). Only oversized images are
/// downscaled, and then at high quality (0.92), just enough to stay under the
/// server's ~10 MB decode cap.
const OCR_PICK_JS: &str = r#"
const input = document.createElement('input');
input.type = 'file';
input.accept = 'image/*';
input.multiple = true;
const RAW_MAX = 6 * 1024 * 1024; // send as-is if the original is under this
const toDataUrl = (file) => new Promise((resolve) => {
  const reader = new FileReader();
  reader.onload = () => {
    const orig = reader.result;
    // Small enough: keep the original encoding (best for OCR).
    if (file.size <= RAW_MAX) { resolve(orig); return; }
    // Too big: downscale at high quality so base64 fits the server cap.
    const img = new Image();
    img.onload = () => {
      try {
        const MAX = 3000;
        let w = img.width, h = img.height;
        if (w > MAX || h > MAX) {
          const s = MAX / Math.max(w, h);
          w = Math.round(w * s); h = Math.round(h * s);
        }
        const c = document.createElement('canvas');
        c.width = w; c.height = h;
        c.getContext('2d').drawImage(img, 0, 0, w, h);
        resolve(c.toDataURL('image/jpeg', 0.92));
      } catch (e) { resolve(orig); }
    };
    img.onerror = () => resolve(orig);
    img.src = orig;
  };
  reader.onerror = () => resolve('');
  reader.readAsDataURL(file);
});
input.onchange = async () => {
  const files = input.files ? Array.from(input.files) : [];
  if (!files.length) { dioxus.send('[]'); return; }
  const out = [];
  for (const f of files) {
    const u = await toDataUrl(f);
    if (u) out.push(u);
  }
  dioxus.send(JSON.stringify(out));
};
input.oncancel = () => dioxus.send('[]');
input.click();
"#;

/// Mirror a (possibly cached) book's own-progress fields into the editor
/// signals. Signals are `Copy`, so this takes them by value like `toc_selector`.
fn apply_progress(
    b: &Book,
    mut edit_page: Signal<String>,
    mut edit_chapter: Signal<String>,
    mut edit_status: Signal<ReadingStatus>,
) {
    if let Some(p) = &b.my_progress {
        edit_page.set(p.current_page.map(|v| v.to_string()).unwrap_or_default());
        edit_chapter.set(p.current_chapter.map(|v| v.to_string()).unwrap_or_default());
        edit_status.set(p.status.clone());
    }
}

#[component]
pub fn BookDetail(id: String) -> Element {
    let book_id = id.clone();

    let mut book = use_signal(|| None::<Book>);
    let mut comments = use_signal(Vec::<BookComment>::new);
    let mut me = use_signal(String::new);
    let mut error_msg = use_signal(|| None::<String>);

    let mut sync_status = use_context::<Signal<SyncStatus>>();
    let sync_trigger = use_context::<Signal<SyncTrigger>>();

    let mut edit_page = use_signal(String::new);
    let edit_chapter = use_signal(String::new);
    let mut edit_status = use_signal(|| ReadingStatus::ToRead);

    let mut toc_editing = use_signal(|| false);
    let mut manual_open = use_signal(|| false);
    let mut toc_text = use_signal(String::new);
    let mut ocr_loading = use_signal(|| false);
    let mut ocr_status = use_signal(String::new);

    let mut comment_body = use_signal(String::new);
    let mut comment_filter = use_signal(String::new);
    // Which comment's emoji picker is open, and its input buffer.
    let react_open = use_signal(|| None::<String>);
    let react_buf = use_signal(String::new);
    // Which top-level comment a reply is being composed for, and its buffer.
    let reply_to = use_signal(|| None::<String>);
    let reply_body = use_signal(String::new);
    // Threads default to collapsed: this is the set of roots the reader has
    // explicitly expanded. Persisted per book so it survives reloads.
    let expanded = {
        let key = format!("expanded_{book_id}");
        use_signal(move || {
            cache::read::<HashSet<String>>(&key).unwrap_or_default()
        })
    };

    let reload = {
        let book_id = book_id.clone();
        move || {
            let book_id = book_id.clone();
            spawn(async move {
                sync_status.set(SyncStatus::Syncing);
                let book_key = format!("book_{book_id}");
                let comments_key = format!("comments_{book_id}");
                let mut ok = true;
                match api::get_book(book_id.clone()).await {
                    Ok(Some(b)) => {
                        apply_progress(&b, edit_page, edit_chapter, edit_status);
                        cache::write(&book_key, &b);
                        book.set(Some(b));
                    }
                    Ok(None) => book.set(None),
                    Err(_) => {
                        ok = false;
                        // Offline / server unreachable: fall back to last-known
                        // copy so the page isn't blank.
                        if book.read().is_none() {
                            if let Some(b) = cache::read::<Book>(&book_key) {
                                apply_progress(&b, edit_page, edit_chapter, edit_status);
                                book.set(Some(b));
                            }
                        }
                    }
                }
                match api::list_comments(book_id).await {
                    Ok(c) => {
                        cache::write(&comments_key, &c);
                        comments.set(c);
                    }
                    Err(_) => {
                        ok = false;
                        if comments.read().is_empty() {
                            if let Some(c) =
                                cache::read::<Vec<BookComment>>(&comments_key)
                            {
                                comments.set(c);
                            }
                        }
                    }
                }
                if ok {
                    cache::write_sync_time();
                }
                sync_status.set(if ok {
                    SyncStatus::Synced
                } else {
                    SyncStatus::CachedOnly
                });
            });
        }
    };

    use_effect(move || {
        if let Some(name) = cache::read::<String>("me") {
            me.set(name);
        }
        spawn(async move {
            if let Ok(name) = api::whoami().await {
                cache::write("me", &name);
                me.set(name);
            }
        });
    });

    {
        let reload = reload.clone();
        use_effect(move || {
            let _t = sync_trigger.read().0;
            reload();
        });
    }
    {
        let reload = reload.clone();
        let book_id = book_id.clone();
        use_effect(move || {
            // Instant paint from last-known data; reload() then refreshes (or
            // keeps this if the network is down). NOTE: do not read `book`
            // reactively here — reload() writes it, which would re-fire this
            // effect in a tight loop (sync flicker + clobbered ToC edits).
            if let Some(b) = cache::read::<Book>(&format!("book_{book_id}")) {
                apply_progress(&b, edit_page, edit_chapter, edit_status);
                book.set(Some(b));
            }
            if let Some(c) =
                cache::read::<Vec<BookComment>>(&format!("comments_{book_id}"))
            {
                comments.set(c);
            }
            reload();
        });
    }

    // Persist status immediately when a status button is tapped.
    let save_with = {
        let book_id = book_id.clone();
        let reload = reload.clone();
        move |status: ReadingStatus| {
            let book_id = book_id.clone();
            let reload = reload.clone();
            let page = edit_page.read().trim().parse::<i32>().ok();
            let chapter = edit_chapter.read().trim().parse::<i32>().ok();
            edit_status.set(status.clone());
            spawn(async move {
                if let Err(e) =
                    api::set_reading_progress(book_id, page, chapter, status).await
                {
                    error_msg.set(Some(format!("Failed: {e}")));
                }
                reload();
            });
        }
    };

    let save_progress = {
        let save_with = save_with.clone();
        move |_| {
            let s = edit_status.read().clone();
            save_with.clone()(s);
        }
    };

    // Jumping to a ToC section persists progress immediately (no SAVE press).
    let on_jump = {
        let book_id = book_id.clone();
        let reload = reload.clone();
        EventHandler::new(move |(page, chapter): (Option<i32>, Option<i32>)| {
            let book_id = book_id.clone();
            let reload = reload.clone();
            // Picking a section means you've started reading.
            let mut status = edit_status.read().clone();
            if status == ReadingStatus::ToRead {
                status = ReadingStatus::Reading;
                edit_status.set(ReadingStatus::Reading);
            }
            spawn(async move {
                if let Err(e) =
                    api::set_reading_progress(book_id, page, chapter, status).await
                {
                    error_msg.set(Some(format!("Failed: {e}")));
                }
                reload();
            });
        })
    };

    let post_comment = {
        let book_id = book_id.clone();
        let reload = reload.clone();
        move |_| {
            let body = comment_body.read().trim().to_string();
            if body.is_empty() {
                return;
            }
            let book_id = book_id.clone();
            let reload = reload.clone();
            comment_body.set(String::new());
            spawn(async move {
                if let Err(e) = api::add_comment(book_id, body, None).await {
                    error_msg.set(Some(format!("Failed to post: {e}")));
                }
                reload();
            });
        }
    };

    let b = book.read().clone();

    rsx! {
        div { class: "px-4 py-4 space-y-4",
            Link {
                to: Route::Books {},
                class: "inline-flex items-center gap-1 text-[11px] text-cyber-dim tracking-wider uppercase press-scale",
                "← Shelf"
            }

            ErrorBanner { message: error_msg }

            if let Some(book) = b {
                // Header
                div { class: "flex gap-4",
                    if let Some(cover) = &book.cover_url {
                        img { class: "w-24 h-36 object-cover rounded-lg shrink-0 border border-cyber-border", src: "{cover}" }
                    } else {
                        div { class: "w-24 h-36 rounded-lg bg-cyber-dark shrink-0 flex items-center justify-center text-3xl", "📖" }
                    }
                    div { class: "min-w-0 flex-1",
                        h2 { class: "text-base font-bold text-neon-cyan leading-snug", "{book.title}" }
                        if let Some(a) = &book.author {
                            p { class: "text-xs text-cyber-dim mt-1", "{a}" }
                        }
                        div { class: "flex gap-3 mt-2 text-[10px] text-cyber-dim",
                            if let Some(p) = book.total_pages {
                                span { "{p} pages" }
                            }
                            if let Some(by) = &book.added_by {
                                span { "added by {by}" }
                            }
                        }
                    }
                }

                if let Some(desc) = &book.description {
                    p { class: "text-xs text-cyber-dim/90 leading-relaxed line-clamp-5", "{desc}" }
                }

                // My progress
                div { class: "bg-cyber-card/80 border border-cyber-border rounded-xl p-4 space-y-3",
                    p { class: "text-[10px] text-neon-cyan tracking-[0.2em] uppercase font-bold", "My Progress" }
                    div { class: "flex gap-2",
                        {status_button(ReadingStatus::ToRead, edit_status, save_with.clone())}
                        {status_button(ReadingStatus::Reading, edit_status, save_with.clone())}
                        {status_button(ReadingStatus::Finished, edit_status, save_with.clone())}
                    }
                    {
                        let toc = book.toc();
                        let has_isbn = book.isbn.as_deref().map(|s| !s.trim().is_empty()).unwrap_or(false);
                        let on_refresh = {
                            let book_id = book_id.clone();
                            let reload = reload.clone();
                            EventHandler::new(move |_| {
                                let book_id = book_id.clone();
                                let reload = reload.clone();
                                spawn(async move {
                                    match api::refresh_toc(book_id).await {
                                        Ok(true) => {}
                                        Ok(false) => error_msg.set(Some(
                                            "No chapter list found on Open Library for this book.".to_string(),
                                        )),
                                        Err(e) => error_msg.set(Some(format!("Lookup failed: {e}"))),
                                    }
                                    reload();
                                });
                            })
                        };
                        let toc_for_prefill = toc.clone();
                        let has_toc = !toc.is_empty();
                        let save_book_id = book_id.clone();
                        let save_reload = reload.clone();
                        let clear_book_id = book_id.clone();
                        let clear_reload = reload.clone();
                        rsx! {
                            { toc_selector(toc.clone(), has_isbn, edit_page, edit_chapter, on_refresh, on_jump) }
                            button {
                                r#type: "button",
                                class: "w-full bg-cyber-dark border border-cyber-border text-neon-purple/80 rounded-lg px-3 py-2 text-[10px] font-bold tracking-wider uppercase press-scale",
                                onclick: move |_| {
                                    let open = !*toc_editing.read();
                                    if open {
                                        toc_text.set(toc_to_text(&toc_for_prefill));
                                    }
                                    toc_editing.set(open);
                                },
                                { if has_toc { "✎ Edit chapter list" } else { "✎ Create chapter list manually" } }
                            }
                            if *toc_editing.read() {
                                div { class: "space-y-2 rounded-lg border border-cyber-border bg-cyber-dark/60 p-3",
                                    p { class: "text-[9px] text-cyber-dim leading-relaxed",
                                        "One entry per line. If a paste merged several entries onto one line, split them. Skip front matter (Cover / Title Page / Contents). Indent 2 spaces to nest — repeat to go deeper (part → chapter → section), or start a line with \"- \". Page optional: end with \" | 12\"."
                                    }
                                    pre { class: "text-[9px] text-cyber-dim/60 whitespace-pre overflow-x-auto",
                                        "Phase 1: Ditch the Negative\n  Break the Cycle\n  NeuroToolkit 1\nPhase 2: Shift Your Narrative\n  Rewire Your Subconscious\n    1. Leave Your Phone Alone\n    2. Visualization & Attention\nPhase 3: Boost the Positive  |  142"
                                    }
                                    div { class: "flex flex-col gap-1",
                                        button {
                                            r#type: "button",
                                            class: "w-full bg-neon-orange/10 border border-neon-orange/30 text-neon-orange rounded-lg px-3 py-2 text-[10px] font-bold tracking-wider uppercase press-scale disabled:opacity-50",
                                            disabled: *ocr_loading.read(),
                                            onclick: move |_| {
                                                if *ocr_loading.read() { return; }
                                                spawn(async move {
                                                    ocr_loading.set(true);
                                                    ocr_status.set("OPENING…".to_string());
                                                    error_msg.set(None);
                                                    // If the editor already has a list, ask whether to
                                                    // replace it or add to it (prevents silent dupes when
                                                    // re-scanning over an already-saved ToC).
                                                    let had_existing =
                                                        !toc_text.read().trim().is_empty();
                                                    let mut replace = false;
                                                    if had_existing {
                                                        let mut cev = document::eval(
                                                            r#"dioxus.send(window.confirm('This chapter list already has entries.\n\nOK  =  REPLACE them with this scan\nCancel  =  ADD this scan to the end'))"#,
                                                        );
                                                        replace = matches!(
                                                            cev.recv::<bool>().await,
                                                            Ok(true)
                                                        );
                                                    }
                                                    let mut eval = document::eval(OCR_PICK_JS);
                                                    let payload = match eval.recv::<String>().await {
                                                        Ok(s) => s,
                                                        Err(_) => {
                                                            ocr_loading.set(false);
                                                            ocr_status.set(String::new());
                                                            return;
                                                        }
                                                    };
                                                    let urls: Vec<String> =
                                                        serde_json::from_str(&payload).unwrap_or_default();
                                                    if urls.is_empty() {
                                                        ocr_loading.set(false);
                                                        ocr_status.set(String::new());
                                                        return;
                                                    }
                                                    let total = urls.len();
                                                    let mut chunks: Vec<String> = Vec::new();
                                                    let mut failed = 0usize;
                                                    for (i, url) in urls.into_iter().enumerate() {
                                                        ocr_status.set(format!("SCANNING {}/{}…", i + 1, total));
                                                        match api::ocr_toc(url).await {
                                                            Ok(text) => {
                                                                let t = text.trim().to_string();
                                                                if !t.is_empty() { chunks.push(t); }
                                                            }
                                                            Err(_) => failed += 1,
                                                        }
                                                    }
                                                    if chunks.is_empty() {
                                                        error_msg.set(Some(if failed > 0 {
                                                            format!("OCR failed for all {total} image(s).")
                                                        } else {
                                                            "No text recognized in the image(s).".to_string()
                                                        }));
                                                    } else {
                                                        let add = chunks.join("\n");
                                                        let cur = toc_text.read().to_string();
                                                        if replace || cur.trim().is_empty() {
                                                            toc_text.set(add);
                                                        } else {
                                                            toc_text.set(format!("{}\n{}", cur.trim_end(), add));
                                                        }
                                                        if failed > 0 {
                                                            error_msg.set(Some(format!(
                                                                "{failed} of {total} image(s) could not be read; the rest were added."
                                                            )));
                                                        }
                                                    }
                                                    ocr_loading.set(false);
                                                    ocr_status.set(String::new());
                                                });
                                            },
                                            {
                                                if *ocr_loading.read() {
                                                    let s = ocr_status.read().clone();
                                                    if s.is_empty() { "SCANNING…".to_string() } else { s }
                                                } else {
                                                    "📷 Upload ToC photo(s)".to_string()
                                                }
                                            }
                                        }
                                        p { class: "text-[8px] text-cyber-dim/60",
                                            "Pick one or more photos — recognized text from each is appended below; review & fix indentation, then SAVE LIST."
                                        }
                                    }
                                    textarea {
                                        class: "w-full bg-cyber-dark border border-cyber-border rounded-lg px-3 py-2 text-xs text-cyber-text outline-none focus:border-neon-purple/60 font-mono resize-y",
                                        rows: "8",
                                        placeholder: "Phase 1: Ditch the Negative\n  Break the Cycle\n    1. Leave Your Phone Alone",
                                        value: "{toc_text}",
                                        oninput: move |e| toc_text.set(e.value()),
                                    }
                                    div { class: "flex gap-2",
                                        button {
                                            r#type: "button",
                                            class: "flex-1 bg-neon-cyan/15 border border-neon-cyan/40 text-neon-cyan rounded-lg px-3 py-2 text-[10px] font-bold tracking-wider uppercase press-scale",
                                            onclick: move |_| {
                                                let txt = toc_text.read().to_string();
                                                let entries = parse_toc_text(&txt);
                                                let book_id = save_book_id.clone();
                                                let reload = save_reload.clone();
                                                spawn(async move {
                                                    if let Err(e) = api::set_toc(book_id, entries).await {
                                                        error_msg.set(Some(format!("Failed to save: {e}")));
                                                    }
                                                    toc_editing.set(false);
                                                    reload();
                                                });
                                            },
                                            "SAVE LIST"
                                        }
                                        button {
                                            r#type: "button",
                                            class: "bg-cyber-dark border border-neon-magenta/40 text-neon-magenta rounded-lg px-3 py-2 text-[10px] font-bold tracking-wider uppercase press-scale",
                                            onclick: move |_| {
                                                let book_id = clear_book_id.clone();
                                                let reload = clear_reload.clone();
                                                spawn(async move {
                                                    if let Err(e) = api::set_toc(book_id, Vec::new()).await {
                                                        error_msg.set(Some(format!("Failed to clear: {e}")));
                                                    }
                                                    toc_text.set(String::new());
                                                    toc_editing.set(false);
                                                    reload();
                                                });
                                            },
                                            "CLEAR"
                                        }
                                        button {
                                            r#type: "button",
                                            class: "bg-cyber-dark border border-cyber-border text-cyber-dim rounded-lg px-3 py-2 text-[10px] font-bold tracking-wider uppercase press-scale",
                                            onclick: move |_| toc_editing.set(false),
                                            "CANCEL"
                                        }
                                    }
                                }
                            }
                        }
                    }
                    button {
                        r#type: "button",
                        class: "w-full bg-cyber-dark border border-cyber-border text-cyber-dim rounded-lg px-3 py-2 text-[10px] font-bold tracking-wider uppercase press-scale",
                        onclick: move |_| {
                            let o = !*manual_open.read();
                            manual_open.set(o);
                        },
                        { if *manual_open.read() { "× Hide exact page / chapter" } else { "⊕ Set exact page / chapter" } }
                    }
                    if *manual_open.read() {
                        p { class: "text-[9px] text-cyber-dim leading-relaxed",
                            "Only needed when the section dropdown above isn't precise enough."
                        }
                        {
                            let page_max = book.total_pages.filter(|t| *t > 0);
                            let chap_max = {
                                let n = book.toc().len() as i32;
                                if n > 0 { n } else { 99 }
                            };
                            rsx! {
                                div { class: "flex items-center gap-2",
                                    if let Some(t) = page_max {
                                        { num_select("Page", t, edit_page) }
                                    } else {
                                        div { class: "flex-1",
                                            label { class: "text-[9px] text-cyber-dim uppercase tracking-wider", "Page" }
                                            input {
                                                class: "w-full bg-cyber-dark border border-cyber-border rounded-lg px-3 py-2 text-sm text-cyber-text outline-none focus:border-neon-cyan/60 font-mono",
                                                r#type: "number",
                                                inputmode: "numeric",
                                                placeholder: "—",
                                                value: "{edit_page}",
                                                oninput: move |e| edit_page.set(e.value()),
                                            }
                                        }
                                    }
                                    { num_select("Chapter", chap_max, edit_chapter) }
                                }
                            }
                        }
                        button {
                            class: "w-full bg-neon-cyan/15 border border-neon-cyan/40 text-neon-cyan rounded-lg px-4 py-2 text-xs font-bold tracking-wider uppercase press-scale",
                            onclick: save_progress,
                            "SAVE PROGRESS"
                        }
                    }
                }

                // Club progress
                div { class: "bg-cyber-card/80 border border-cyber-border rounded-xl p-4 space-y-3",
                    p { class: "text-[10px] text-neon-purple tracking-[0.2em] uppercase font-bold", "Club Progress" }
                    if book.progress.is_empty() {
                        p { class: "text-xs text-cyber-dim", "No one has started yet." }
                    }
                    for p in book.progress.iter() {
                        {render_club_row(p, &book)}
                    }
                }

                // Discussion
                div { class: "bg-cyber-card/80 border border-cyber-border rounded-xl p-4 space-y-3",
                    p { class: "text-[10px] text-neon-orange tracking-[0.2em] uppercase font-bold", "Discussion" }

                    div { class: "space-y-2",
                        textarea {
                            class: "w-full bg-cyber-dark border border-cyber-border rounded-lg px-3 py-2 text-sm text-cyber-text outline-none focus:border-neon-orange/60 font-mono resize-none",
                            rows: "2",
                            placeholder: "Share a thought...",
                            value: "{comment_body}",
                            oninput: move |e| comment_body.set(e.value()),
                        }
                        div { class: "flex items-center gap-2 flex-wrap",
                            span { class: "text-[9px] text-cyber-dim",
                                "🔖 Auto-anchored to your current section — hidden from readers behind you."
                            }
                            button {
                                r#type: "button",
                                class: "ml-auto bg-neon-orange/15 border border-neon-orange/40 text-neon-orange rounded-md px-3 py-1 text-[10px] font-bold tracking-wider uppercase press-scale",
                                onclick: post_comment,
                                "POST"
                            }
                        }
                    }

                    div { class: "space-y-2 pt-1",
                        {
                            // Snapshot the signals into owned values BEFORE rendering
                            // children. Holding a `comments.read()` guard across
                            // `render_comment` construction/diff is the Dioxus 0.7
                            // "already borrowed" trap that surfaces as a Wasm
                            // `unreachable executed` panic on teardown/re-render
                            // (e.g. the ToC editor CANCEL path).
                            let comments_snapshot = comments.read().clone();
                            let me_now = me.read().clone();
                            // Distinct authors in first-seen order (newest first,
                            // since the list is now DESC by created_at).
                            let mut authors: Vec<String> = Vec::new();
                            for c in comments_snapshot.iter() {
                                if !authors.contains(&c.author) {
                                    authors.push(c.author.clone());
                                }
                            }
                            // Normalize: a stale filter (e.g. carried over from
                            // another book) falls back to "everyone".
                            let raw = comment_filter.read().clone();
                            let filter = if authors.iter().any(|a| *a == raw) {
                                raw
                            } else {
                                String::new()
                            };
                            let visible: Vec<BookComment> = comments_snapshot
                                .iter()
                                .filter(|c| filter.is_empty() || c.author == filter)
                                .cloned()
                                .collect();
                            // Thread only when not filtering (a filter would
                            // orphan replies). roots are newest-first; each
                            // root's replies are oldest-first.
                            let threaded = filter.is_empty();
                            let threads: Vec<(BookComment, Vec<BookComment>)> = if threaded {
                                visible
                                    .iter()
                                    .filter(|c| c.parent_id.is_none())
                                    .map(|root| {
                                        let mut kids: Vec<BookComment> = visible
                                            .iter()
                                            .filter(|c| {
                                                c.parent_id.as_deref()
                                                    == Some(root.id.as_str())
                                            })
                                            .cloned()
                                            .collect();
                                        kids.reverse();
                                        (root.clone(), kids)
                                    })
                                    .collect()
                            } else {
                                Vec::new()
                            };
                            let bid = book_id.clone();
                            rsx! {
                                if authors.len() > 1 {
                                    div { class: "flex items-center gap-2",
                                        label { class: "text-[9px] text-cyber-dim uppercase tracking-wider shrink-0", "Filter" }
                                        select {
                                            class: "flex-1 bg-cyber-dark border border-cyber-border rounded-lg px-2 py-1.5 text-xs text-cyber-text outline-none focus:border-neon-orange/60 font-mono",
                                            value: "{filter}",
                                            onchange: move |e| comment_filter.set(e.value()),
                                            option { value: "", selected: filter.is_empty(), "Everyone" }
                                            for a in authors.iter() {
                                                option { value: "{a}", selected: filter == *a, "{a}" }
                                            }
                                        }
                                    }
                                }
                                if visible.is_empty() {
                                    p { class: "text-xs text-cyber-dim text-center py-3",
                                        { if comments_snapshot.is_empty() { "No comments yet — start the conversation." } else { "No comments from this person." } }
                                    }
                                }
                                if threaded {
                                    for (root, kids) in threads.iter() {
                                        {render_comment(root.clone(), me_now.clone(), reload.clone(), error_msg, react_open, react_buf, bid.clone(), reply_to, reply_body, expanded, kids.clone(), true)}
                                    }
                                } else {
                                    for c in visible.iter() {
                                        {render_comment(c.clone(), me_now.clone(), reload.clone(), error_msg, react_open, react_buf, bid.clone(), reply_to, reply_body, expanded, Vec::new(), false)}
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                div { class: "text-center py-20",
                    p { class: "text-xs tracking-[0.3em] uppercase text-cyber-dim", "Book not found" }
                }
            }
        }
    }
}

fn status_button(
    status: ReadingStatus,
    edit_status: Signal<ReadingStatus>,
    mut on_pick: impl FnMut(ReadingStatus) + Clone + 'static,
) -> Element {
    let active = *edit_status.read() == status;
    let s2 = status.clone();
    let cls = if active {
        match status {
            ReadingStatus::Reading => "bg-neon-cyan/20 border-neon-cyan/60 text-neon-cyan",
            ReadingStatus::Finished => "bg-neon-green/20 border-neon-green/60 text-neon-green",
            ReadingStatus::ToRead => "bg-neon-purple/20 border-neon-purple/60 text-neon-purple",
        }
    } else {
        "bg-cyber-dark border-cyber-border text-cyber-dim"
    };
    rsx! {
        button {
            r#type: "button",
            class: "flex-1 rounded-lg border px-2 py-2 text-[10px] font-bold tracking-wider uppercase press-scale {cls}",
            onclick: move |_| on_pick(s2.clone()),
            "{status.label()}"
        }
    }
}

fn toc_label(e: &TocEntry) -> String {
    let indent = "— ".repeat(e.level.max(0) as usize);
    let lbl = e
        .label
        .as_deref()
        .map(|l| format!("{l}  "))
        .unwrap_or_default();
    let pg = e.page.map(|p| format!("   · p.{p}")).unwrap_or_default();
    format!("{indent}{lbl}{}{pg}", e.title)
}

/// Render a stored ToC back into the editable text format.
fn toc_to_text(toc: &[TocEntry]) -> String {
    toc.iter()
        .map(|e| {
            let indent = "  ".repeat(e.level.max(0) as usize);
            let label = e
                .label
                .as_deref()
                .map(|l| format!("{l} "))
                .unwrap_or_default();
            let page = e.page.map(|p| format!("  |  {p}")).unwrap_or_default();
            format!("{indent}{label}{}{page}", e.title)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Parse the manual editor text into ToC entries.
/// Indentation (2 spaces) or a leading bullet marks a subchapter level;
/// an optional ` | <n>` (or ` @ <n>`) trailing token sets the page.
fn parse_toc_text(s: &str) -> Vec<TocEntry> {
    let mut out: Vec<TocEntry> = Vec::new();
    for raw in s.lines() {
        if out.len() >= 1000 {
            break;
        }
        if raw.trim().is_empty() {
            continue;
        }
        let mut indent = 0usize;
        for ch in raw.chars() {
            match ch {
                ' ' => indent += 1,
                '\t' => indent += 4,
                _ => break,
            }
        }
        let mut level = (indent / 2).min(3) as i32;
        let mut rest = raw.trim().to_string();
        for m in ["- ", "* ", "> ", "• ", "– "] {
            if let Some(stripped) = rest.strip_prefix(m) {
                rest = stripped.trim_start().to_string();
                level = level.max(1);
                break;
            }
        }

        let mut page: Option<i32> = None;
        for sep in [" | ", " @ ", "|", "@"] {
            if let Some(idx) = rest.rfind(sep) {
                let (head, tail) = rest.split_at(idx);
                let tail = &tail[sep.len()..];
                if !head.trim().is_empty() {
                    if let Ok(p) = tail.trim().parse::<i32>() {
                        page = Some(p);
                        rest = head.trim_end().to_string();
                        break;
                    }
                }
            }
        }

        let title: String = rest.trim().chars().take(200).collect();
        if title.is_empty() {
            continue;
        }
        out.push(TocEntry {
            title,
            label: None,
            page,
            level,
        });
    }
    out
}

/// Compact `1..=max` numeric dropdown bound to a `String` signal
/// (`""` = unset). Used for the rarely-touched manual Page / Chapter fields.
fn num_select(label_text: &str, max: i32, mut value: Signal<String>) -> Element {
    let cur = value.read().trim().to_string();
    let cur_n = cur.parse::<i32>().ok();
    rsx! {
        div { class: "flex-1",
            label { class: "text-[9px] text-cyber-dim uppercase tracking-wider", "{label_text}" }
            select {
                class: "w-full bg-cyber-dark border border-cyber-border rounded-lg px-3 py-2 text-sm text-cyber-text outline-none focus:border-neon-cyan/60 font-mono",
                value: "{cur}",
                onchange: move |e| value.set(e.value()),
                option { value: "", selected: cur_n.is_none(), "—" }
                for n in 1..=max {
                    option { value: "{n}", selected: cur_n == Some(n), "{n}" }
                }
            }
        }
    }
}

fn toc_selector(
    toc: Vec<TocEntry>,
    has_isbn: bool,
    mut edit_page: Signal<String>,
    mut edit_chapter: Signal<String>,
    on_refresh: EventHandler<()>,
    on_jump: EventHandler<(Option<i32>, Option<i32>)>,
) -> Element {
    if !toc.is_empty() {
        let toc_for_change = toc.clone();
        // Saved chapter is the 1-based ToC index, so the reader's current
        // position pre-selects its section — the dropdown doubles as a
        // "where am I" indicator, not just a jump control.
        let sel_idx = edit_chapter
            .read()
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|c| c.checked_sub(1))
            .filter(|i| *i < toc.len());
        let sel_value = sel_idx.map(|i| i.to_string()).unwrap_or_default();
        rsx! {
            div { class: "space-y-1",
                label { class: "text-[9px] text-cyber-dim uppercase tracking-wider", "Current section — changes save instantly" }
                select {
                    class: "w-full bg-cyber-dark border border-cyber-border rounded-lg px-3 py-2 text-sm text-cyber-text outline-none focus:border-neon-cyan/60 font-mono",
                    value: "{sel_value}",
                    onchange: move |e| {
                        if let Ok(i) = e.value().parse::<usize>() {
                            if let Some(en) = toc_for_change.get(i) {
                                let chapter = Some((i + 1) as i32);
                                // Keep the existing page if this entry has none.
                                let page = en
                                    .page
                                    .or_else(|| edit_page.read().trim().parse::<i32>().ok());
                                if let Some(pg) = en.page {
                                    edit_page.set(pg.to_string());
                                }
                                edit_chapter.set((i + 1).to_string());
                                on_jump.call((page, chapter));
                            }
                        }
                    },
                    option { value: "", selected: sel_idx.is_none(), "— not started · pick a section —" }
                    for (i, en) in toc.iter().enumerate() {
                        option { value: "{i}", selected: sel_idx == Some(i), {toc_label(en)} }
                    }
                }
            }
        }
    } else if has_isbn {
        rsx! {
            button {
                r#type: "button",
                class: "w-full bg-cyber-dark border border-cyber-border text-cyber-dim rounded-lg px-3 py-2 text-[10px] font-bold tracking-wider uppercase press-scale",
                onclick: move |_| on_refresh.call(()),
                "↻ Fetch chapter list (Open Library)"
            }
        }
    } else {
        rsx! {}
    }
}

fn render_club_row(p: &ReadingProgress, book: &Book) -> Element {
    let sections = book.toc().len() as i32;
    let label = match p.status {
        ReadingStatus::Finished => "FINISHED".to_string(),
        ReadingStatus::ToRead => "TO READ".to_string(),
        ReadingStatus::Reading => {
            if sections > 0 {
                match p.current_chapter.filter(|c| *c > 0) {
                    Some(ch) => format!("§ {} / {sections}", ch.min(sections)),
                    None => "reading".to_string(),
                }
            } else {
                match (p.current_page, book.total_pages) {
                    (Some(c), Some(t)) if t > 0 => format!("p.{c} / {t}"),
                    (Some(c), _) => format!("p.{c}"),
                    _ => "reading".to_string(),
                }
            }
        }
    };
    let frac = book.reading_fraction(p);
    rsx! {
        div { class: "space-y-1",
            div { class: "flex items-center justify-between text-[11px]",
                span { class: "text-cyber-text font-medium", "{p.reader}" }
                span { class: "text-cyber-dim", "{label}" }
            }
            if let Some((cur, tot)) = frac {
                ProgressBar { watched: cur, total: tot }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_comment(
    c: BookComment,
    me: String,
    reload: impl FnMut() + Clone + 'static,
    mut error_msg: Signal<Option<String>>,
    react_open: Signal<Option<String>>,
    react_buf: Signal<String>,
    book_id: String,
    mut reply_to: Signal<Option<String>>,
    mut reply_body: Signal<String>,
    mut expanded: Signal<HashSet<String>>,
    replies: Vec<BookComment>,
    allow_reply: bool,
) -> Element {
    let anchor = match (c.page, c.chapter) {
        (Some(p), Some(ch)) => Some(format!("p.{p} · ch.{ch}")),
        (Some(p), None) => Some(format!("p.{p}")),
        (None, Some(ch)) => Some(format!("ch.{ch}")),
        (None, None) => None,
    };
    let mine = c.author == me;
    let del_id = c.id.clone();
    let cid = c.id.clone();
    let replying = reply_to.read().as_ref() == Some(&cid);
    let reply_count = replies.len();
    let foldable = allow_reply && reply_count > 0;
    let is_folded = foldable && !expanded.read().contains(&cid);

    let self_box = if c.hidden {
        rsx! {
            div { class: "rounded-lg border border-cyber-border bg-cyber-dark/60 px-3 py-2",
                div { class: "flex items-center gap-2",
                    span { class: "text-[11px] text-cyber-dim", "🔒 {c.author}" }
                    if let Some(a) = &anchor {
                        span { class: "text-[9px] text-neon-orange/70 border border-neon-orange/30 rounded px-1", "{a}" }
                    }
                }
                p { class: "text-[11px] text-cyber-dim/70 italic mt-1",
                    "Hidden — past your reading progress. Read further to unlock."
                }
            }
        }
    } else {
        let reactions_el = reaction_bar(&c, reload.clone(), error_msg, react_open, react_buf);
        let del_reload = reload.clone();
        rsx! {
            div { class: "rounded-lg border border-cyber-border bg-cyber-dark/40 px-3 py-2",
                div { class: "flex items-center gap-2",
                    span { class: "text-[11px] font-semibold text-neon-green", "{c.author}" }
                    if let Some(a) = &anchor {
                        span { class: "text-[9px] text-neon-orange/80 border border-neon-orange/30 rounded px-1", "{a}" }
                    }
                    span { class: "text-[9px] text-cyber-dim ml-auto", {format_ago(c.created_at)} }
                    if mine {
                        button {
                            class: "text-neon-magenta text-xs font-bold press-scale",
                            onclick: move |_| {
                                let did = del_id.clone();
                                let mut rl = del_reload.clone();
                                spawn(async move {
                                    if let Err(e) = crate::api::books::delete_comment(did).await {
                                        error_msg.set(Some(format!("Failed: {e}")));
                                    }
                                    rl();
                                });
                            },
                            "×"
                        }
                    }
                }
                p { class: "text-xs text-cyber-text leading-relaxed mt-1 whitespace-pre-wrap", "{c.body}" }
                {reactions_el}
                if allow_reply {
                    button {
                        r#type: "button",
                        class: "mt-2 text-[10px] font-bold tracking-wider uppercase text-cyber-dim press-scale",
                        onclick: {
                            let cid = cid.clone();
                            move |_| {
                                if reply_to.read().as_ref() == Some(&cid) {
                                    reply_to.set(None);
                                } else {
                                    reply_body.set(String::new());
                                    reply_to.set(Some(cid.clone()));
                                }
                            }
                        },
                        if replying { "↳ Cancel" } else { "↳ Reply" }
                    }
                }
                if replying {
                    div { class: "mt-2 space-y-2",
                        textarea {
                            class: "w-full bg-cyber-dark border border-cyber-border rounded-lg px-3 py-2 text-sm text-cyber-text outline-none focus:border-neon-orange/60 font-mono resize-none",
                            rows: "2",
                            placeholder: "Reply...",
                            value: "{reply_body}",
                            oninput: move |e| reply_body.set(e.value()),
                        }
                        button {
                            r#type: "button",
                            class: "bg-neon-orange/15 border border-neon-orange/40 text-neon-orange rounded-md px-3 py-1 text-[10px] font-bold tracking-wider uppercase press-scale",
                            onclick: {
                                let cid = cid.clone();
                                let book_id = book_id.clone();
                                let reload = reload.clone();
                                move |_| {
                                    let body = reply_body.read().trim().to_string();
                                    if body.is_empty() {
                                        return;
                                    }
                                    let cid = cid.clone();
                                    let book_id = book_id.clone();
                                    let mut rl = reload.clone();
                                    reply_body.set(String::new());
                                    reply_to.set(None);
                                    spawn(async move {
                                        if let Err(e) = crate::api::books::add_comment(
                                            book_id,
                                            body,
                                            Some(cid),
                                        )
                                        .await
                                        {
                                            error_msg.set(Some(format!("Failed to post: {e}")));
                                        }
                                        rl();
                                    });
                                }
                            },
                            "POST REPLY"
                        }
                    }
                }
                if foldable {
                    button {
                        r#type: "button",
                        class: "mt-2 ml-3 text-[10px] font-bold tracking-wider uppercase text-neon-cyan/70 press-scale",
                        onclick: {
                            let cid = cid.clone();
                            let book_id = book_id.clone();
                            move |_| {
                                expanded.with_mut(|s| {
                                    if !s.remove(&cid) {
                                        s.insert(cid.clone());
                                    }
                                });
                                cache::write(
                                    &format!("expanded_{book_id}"),
                                    &*expanded.read(),
                                );
                            }
                        },
                        if is_folded { "▸ Show {reply_count} replies" } else { "▾ Hide replies" }
                    }
                }
            }
        }
    };

    rsx! {
        {self_box}
        if !replies.is_empty() && !is_folded {
            div { class: "ml-4 pl-3 border-l border-cyber-border/60 space-y-2 mt-2",
                for r in replies.iter() {
                    {render_comment(
                        r.clone(), me.clone(), reload.clone(), error_msg,
                        react_open, react_buf, book_id.clone(),
                        reply_to, reply_body, expanded, Vec::new(), false,
                    )}
                }
            }
        }
    }
}

/// Reaction bar under a comment: existing reactions as toggle chips, plus a
/// `＋` that opens a native text input so any emoji from the phone keyboard
/// can be used (with a few quick picks for convenience).
fn reaction_bar(
    c: &BookComment,
    reload: impl FnMut() + Clone + 'static,
    mut error_msg: Signal<Option<String>>,
    mut react_open: Signal<Option<String>>,
    mut react_buf: Signal<String>,
) -> Element {
    let cid = c.id.clone();
    let reactions = c.reactions.clone();
    let open = react_open.read().as_ref() == Some(&cid);
    rsx! {
        div { class: "flex flex-wrap items-center gap-1 mt-2",
            for r in reactions.iter() {
                {
                    let cls = if r.mine {
                        "border-neon-cyan/60 bg-neon-cyan/15 text-neon-cyan"
                    } else {
                        "border-cyber-border bg-cyber-dark text-cyber-text"
                    };
                    let emoji = r.emoji.clone();
                    let cid = cid.clone();
                    let reload = reload.clone();
                    rsx! {
                        button {
                            r#type: "button",
                            class: "rounded-full border px-2 py-0.5 text-[11px] leading-none press-scale {cls}",
                            onclick: move |_| {
                                let cid = cid.clone();
                                let emoji = emoji.clone();
                                let mut rl = reload.clone();
                                spawn(async move {
                                    if let Err(e) =
                                        crate::api::books::react_to_comment(cid, emoji).await
                                    {
                                        error_msg.set(Some(format!("Failed: {e}")));
                                    }
                                    rl();
                                });
                            },
                            "{r.emoji} {r.count}"
                        }
                    }
                }
            }
            button {
                r#type: "button",
                class: "rounded-full border border-cyber-border text-cyber-dim px-2 py-0.5 text-[11px] leading-none press-scale",
                onclick: {
                    let cid = cid.clone();
                    move |_| {
                        if react_open.read().as_ref() == Some(&cid) {
                            react_open.set(None);
                        } else {
                            react_buf.set(String::new());
                            react_open.set(Some(cid.clone()));
                        }
                    }
                },
                "＋"
            }
            if open {
                input {
                    class: "w-14 bg-cyber-dark border border-neon-cyan/40 rounded-full px-2 py-0.5 text-[13px] text-cyber-text outline-none text-center",
                    r#type: "text",
                    inputmode: "text",
                    autocomplete: "off",
                    // `autofocus` only fires on initial page load, not for a
                    // dynamically-inserted input, so the soft keyboard never
                    // opened. Focus it explicitly the moment it mounts.
                    onmounted: move |e| {
                        spawn(async move {
                            let _ = e.set_focus(true).await;
                        });
                    },
                    value: "{react_buf}",
                    placeholder: "🙂",
                    oninput: {
                        let cid = cid.clone();
                        let reload = reload.clone();
                        move |e| {
                            let v = e.value();
                            // Submit on the first emoji; ignore plain typing.
                            let picked: String =
                                v.chars().filter(|ch| !ch.is_ascii()).collect();
                            if picked.is_empty() {
                                react_buf.set(v);
                                return;
                            }
                            react_open.set(None);
                            react_buf.set(String::new());
                            let cid = cid.clone();
                            let mut rl = reload.clone();
                            spawn(async move {
                                if let Err(e) =
                                    crate::api::books::react_to_comment(cid, picked).await
                                {
                                    error_msg.set(Some(format!("Failed: {e}")));
                                }
                                rl();
                            });
                        }
                    },
                }
                for &q in REACTION_EMOJIS.iter() {
                    {
                        let cid = cid.clone();
                        let reload = reload.clone();
                        rsx! {
                            button {
                                r#type: "button",
                                class: "rounded-full border border-cyber-border px-2 py-0.5 text-[13px] leading-none press-scale",
                                onclick: move |_| {
                                    react_open.set(None);
                                    let cid = cid.clone();
                                    let mut rl = reload.clone();
                                    spawn(async move {
                                        if let Err(e) = crate::api::books::react_to_comment(
                                            cid,
                                            q.to_string(),
                                        )
                                        .await
                                        {
                                            error_msg.set(Some(format!("Failed: {e}")));
                                        }
                                        rl();
                                    });
                                },
                                "{q}"
                            }
                        }
                    }
                }
            }
        }
    }
}

fn format_ago(created_at: f64) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let now = js_sys::Date::now();
        let diff = ((now - created_at) / 1000.0).max(0.0) as u64;
        if diff < 60 {
            "just now".to_string()
        } else if diff < 3600 {
            format!("{}m ago", diff / 60)
        } else if diff < 86400 {
            format!("{}h ago", diff / 3600)
        } else {
            format!("{}d ago", diff / 86400)
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = created_at;
        String::new()
    }
}
