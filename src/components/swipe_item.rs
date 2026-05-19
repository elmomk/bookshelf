use dioxus::prelude::*;

use crate::components::icons::*;

const THRESHOLD: f64 = 72.0;

/// Trigger a short haptic vibration when crossing the swipe threshold.
#[cfg(target_arch = "wasm32")]
fn haptic_tick() {
    if let Some(window) = web_sys::window() {
        let _ = window.navigator().vibrate_with_duration(10);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn haptic_tick() {}

#[cfg(target_arch = "wasm32")]
fn now_ms() -> f64 {
    js_sys::Date::now()
}

#[cfg(not(target_arch = "wasm32"))]
fn now_ms() -> f64 {
    0.0
}

/// Flick velocity (px/ms) that dismisses regardless of distance, and the
/// minimum travel before a flick counts (ignores tiny accidental moves).
const FLICK_VELOCITY: f64 = 0.11;
const FLICK_MIN_PX: f64 = 8.0;

#[component]
pub fn SwipeItem(
    children: Element,
    on_swipe_right: Option<EventHandler<()>>,
    on_swipe_left: EventHandler<()>,
    completed: bool,
) -> Element {
    let mut translate_x = use_signal(|| 0.0_f64);
    let mut start_x = use_signal(|| 0.0_f64);
    let mut start_y = use_signal(|| 0.0_f64);
    let mut swiping = use_signal(|| false);
    let mut direction_locked = use_signal(|| false);
    let mut is_horizontal = use_signal(|| false);
    let mut animating = use_signal(|| false);
    let mut threshold_crossed = use_signal(|| false);
    let mut start_time = use_signal(|| 0.0_f64);

    let opacity = if completed { "opacity-40" } else { "" };
    let line_through = if completed { "line-through decoration-cyber-dim/50" } else { "" };
    let tx = *translate_x.read();

    let bg_color = if tx > 0.0 {
        "bg-neon-green/80"
    } else if tx < 0.0 {
        "bg-neon-magenta/80"
    } else {
        "bg-transparent"
    };

    let transition = if *animating.read() {
        "transition-transform duration-200 [transition-timing-function:cubic-bezier(0.23,1,0.32,1)]"
    } else {
        ""
    };

    rsx! {
        div { class: "relative overflow-hidden rounded-lg mb-2 item-enter",
            // Background action indicator
            div { class: "absolute inset-0 flex items-center justify-between px-6 {bg_color}",
                if tx > 0.0 {
                    CheckIcon { class: "w-6 h-6 text-cyber-black".to_string() }
                }
                if tx < 0.0 {
                    div { class: "ml-auto",
                        TrashIcon { class: "w-6 h-6 text-white".to_string() }
                    }
                }
            }

            // Swipeable content
            div {
                class: "relative bg-cyber-card border border-cyber-border rounded-lg p-4 {opacity} {line_through} {transition}",
                style: "transform: translateX({tx}px)",
                ontouchstart: move |e| {
                    if let Some(touch) = e.data().touches().first() {
                        start_x.set(touch.client_coordinates().x);
                        start_y.set(touch.client_coordinates().y);
                        swiping.set(true);
                        direction_locked.set(false);
                        is_horizontal.set(false);
                        animating.set(false);
                        threshold_crossed.set(false);
                        start_time.set(now_ms());
                    }
                },
                ontouchmove: move |e| {
                    if !*swiping.read() {
                        return;
                    }
                    if let Some(touch) = e.data().touches().first() {
                        let dx = touch.client_coordinates().x - *start_x.read();
                        let dy = touch.client_coordinates().y - *start_y.read();

                        if !*direction_locked.read() {
                            if dx.abs() > 10.0 || dy.abs() > 10.0 {
                                direction_locked.set(true);
                                is_horizontal.set(dx.abs() > dy.abs());
                            }
                            return;
                        }

                        if !*is_horizontal.read() {
                            return;
                        }

                        if dx > 0.0 && on_swipe_right.is_none() {
                            return;
                        }

                        e.prevent_default();

                        // Haptic feedback when crossing threshold
                        let was_crossed = *threshold_crossed.read();
                        let now_crossed = dx.abs() > THRESHOLD;
                        if now_crossed && !was_crossed {
                            haptic_tick();
                        }
                        threshold_crossed.set(now_crossed);

                        translate_x.set(dx);
                    }
                },
                ontouchend: move |_| {
                    swiping.set(false);
                    animating.set(true);
                    let tx = *translate_x.read();
                    let dt = (now_ms() - *start_time.read()).max(1.0);
                    let velocity = tx.abs() / dt;
                    let flick = tx.abs() > FLICK_MIN_PX && velocity > FLICK_VELOCITY;

                    if (tx > THRESHOLD || (tx > 0.0 && flick)) && on_swipe_right.is_some() {
                        // Fly out the way it was swiped (spatial consistency),
                        // then the parent's reload removes the row.
                        translate_x.set(800.0);
                        if let Some(ref handler) = on_swipe_right {
                            handler.call(());
                        }
                    } else if tx < -THRESHOLD || (tx < 0.0 && flick) {
                        translate_x.set(-800.0);
                        on_swipe_left.call(());
                    } else {
                        translate_x.set(0.0);
                    }
                },
                {children}
            }
        }
    }
}
