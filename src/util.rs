//! Tiny client-side helpers shared across pages.

/// Await `ms` milliseconds via the existing eval bridge (no timer crate
/// needed). No-op on the server.
#[cfg(target_arch = "wasm32")]
pub async fn anim_sleep(ms: u32) {
    use dioxus::prelude::*;
    let mut e = document::eval(&format!("setTimeout(() => dioxus.send(0), {ms})"));
    let _ = e.recv::<i32>().await;
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn anim_sleep(_ms: u32) {
    // Server-side: noop.
}
