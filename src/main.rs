mod cache;
mod components;
mod models;
mod pages;
mod route;
mod api;
mod util;
#[cfg(not(target_arch = "wasm32"))]
mod server;

use dioxus::prelude::*;

use route::Route;

static CSS: Asset = asset!("/assets/main.css");

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        server::db::init();

        dioxus_server::serve(|| async {
            use dioxus_server::{DioxusRouterExt, ServeConfig};
            use tower_http::compression::CompressionLayer;

            // One automatic snapshot per day. Catches up on startup if the
            // newest snapshot is older than 24h.
            tokio::spawn(run_daily_snapshots());

            Ok(axum::Router::new()
                .serve_dioxus_application(ServeConfig::new(), App)
                .layer(CompressionLayer::new()))
        });
    }

    #[cfg(target_arch = "wasm32")]
    dioxus::launch(App);
}

#[cfg(not(target_arch = "wasm32"))]
async fn run_daily_snapshots() {
    use std::time::Duration;
    const DAY_MS: u64 = 24 * 60 * 60 * 1000;
    // Defense-in-depth: never create auto snapshots more often than this,
    // no matter what the math says. If a regression ever makes the sleep
    // calculation return 0, this caps damage at ~24 snapshots/day instead
    // of saturating the disk.
    const MIN_INTERVAL_MS: u64 = 60 * 60 * 1000; // 1 hour

    // Sanity log — confirms exactly one task instance is running.
    eprintln!("daily-snapshots: task started");

    loop {
        // Sleep until the next due tick (immediate if none in last 24h).
        let now_ms = chrono::Utc::now().timestamp_millis() as i64;
        let newest = server::snapshots::ts_of_newest();
        let due_in_ms: u64 = match newest {
            Some(ts) => {
                let elapsed_ms = (now_ms - ts as i64).max(0) as u64;
                DAY_MS.saturating_sub(elapsed_ms).max(MIN_INTERVAL_MS)
            }
            None => MIN_INTERVAL_MS,
        };
        eprintln!(
            "daily-snapshots: sleeping due_in_ms={} (newest_ts={:?}, now_ms={})",
            due_in_ms, newest, now_ms
        );
        tokio::time::sleep(Duration::from_millis(due_in_ms)).await;

        // Best-effort: never panic the task if the pool is briefly unhappy.
        match server::db::pool().get() {
            Ok(conn) => match server::snapshots::create_auto(&conn) {
                Ok(id) => eprintln!("auto-snapshot: {id}"),
                Err(e) => eprintln!("auto-snapshot failed: {e}"),
            },
            Err(e) => eprintln!("auto-snapshot pool error: {e}"),
        }
        // Run retention right after creating so we never miss a sweep.
        server::snapshots::prune();
    }
}

#[component]
fn App() -> Element {
    rsx! {
        document::Stylesheet { href: CSS }
        document::Link { rel: "manifest", href: "/manifest.json" }
        document::Link { rel: "apple-touch-icon", href: "/icons/icon-192.png" }
        document::Meta { name: "viewport", content: "width=device-width, initial-scale=1, viewport-fit=cover" }
        document::Meta { name: "theme-color", content: "#08080f" }
        document::Meta { name: "apple-mobile-web-app-capable", content: "yes" }
        document::Meta { name: "apple-mobile-web-app-status-bar-style", content: "black-translucent" }

        Router::<Route> {}
    }
}
