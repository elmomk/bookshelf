use dioxus::prelude::*;

use crate::components::layout::AppLayout;
use crate::pages::*;

#[derive(Routable, Clone, Debug, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    #[layout(AppLayout)]
        #[route("/books")]
        Books {},
        #[route("/book/:id")]
        BookDetail { id: String },
        #[route("/activity")]
        Activity {},
        #[route("/settings")]
        Settings {},
    #[end_layout]
    #[redirect("/", || Route::Books {})]
    #[route("/:..segments")]
    NotFound { segments: Vec<String> },
}

#[component]
fn NotFound(segments: Vec<String>) -> Element {
    rsx! {
        div { class: "flex items-center justify-center h-full",
            p { class: "text-lg text-gray-500", "Page not found" }
        }
    }
}
