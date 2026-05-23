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

            Ok(axum::Router::new()
                .serve_dioxus_application(ServeConfig::new(), App)
                .layer(CompressionLayer::new()))
        });
    }

    #[cfg(target_arch = "wasm32")]
    dioxus::launch(App);
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
