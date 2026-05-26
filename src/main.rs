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

    loop {
        // Sleep until the next due tick (immediate if none in last 24h).
        let now_ms = chrono::Utc::now().timestamp_millis() as i64;
        let due_in_ms: i64 = match server::snapshots::ts_of_newest() {
            Some(ts) => (DAY_MS as i64) - (now_ms - ts as i64),
            None => 0,
        };
        let due_in_ms = due_in_ms.clamp(0, DAY_MS as i64) as u64;
        if due_in_ms > 0 {
            tokio::time::sleep(Duration::from_millis(due_in_ms)).await;
        }

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
