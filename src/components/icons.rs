use dioxus::prelude::*;

#[component]
pub fn CheckSquareIcon(class: Option<String>) -> Element {
    let class = class.unwrap_or_default();
    rsx! {
        svg {
            class: "{class}",
            xmlns: "http://www.w3.org/2000/svg",
            width: "24", height: "24",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            path { d: "m9 11 3 3L22 4" }
            path { d: "M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11" }
        }
    }
}

#[component]
pub fn ShoppingCartIcon(class: Option<String>) -> Element {
    let class = class.unwrap_or_default();
    rsx! {
        svg {
            class: "{class}",
            xmlns: "http://www.w3.org/2000/svg",
            width: "24", height: "24",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            circle { cx: "8", cy: "21", r: "1" }
            circle { cx: "19", cy: "21", r: "1" }
            path { d: "M2.05 2.05h2l2.66 12.42a2 2 0 0 0 2 1.58h9.78a2 2 0 0 0 1.95-1.57l1.65-7.43H5.12" }
        }
    }
}

#[component]
pub fn PackageIcon(class: Option<String>) -> Element {
    let class = class.unwrap_or_default();
    rsx! {
        svg {
            class: "{class}",
            xmlns: "http://www.w3.org/2000/svg",
            width: "24", height: "24",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            path { d: "m7.5 4.27 9 5.15" }
            path { d: "M21 8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16Z" }
            path { d: "m3.3 7 8.7 5 8.7-5" }
            path { d: "M12 22V12" }
        }
    }
}

#[component]
pub fn TvIcon(class: Option<String>) -> Element {
    let class = class.unwrap_or_default();
    rsx! {
        svg {
            class: "{class}",
            xmlns: "http://www.w3.org/2000/svg",
            width: "24", height: "24",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            rect { x: "2", y: "7", width: "20", height: "15", rx: "2", ry: "2" }
            polyline { points: "17 2 12 7 7 2" }
        }
    }
}

#[component]
pub fn HeartIcon(class: Option<String>) -> Element {
    let class = class.unwrap_or_default();
    // Cute flower icon — a little bloom with round petals and a center dot
    rsx! {
        svg {
            class: "{class}",
            xmlns: "http://www.w3.org/2000/svg",
            width: "24", height: "24",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "1.8",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            // Five petals around center
            circle { cx: "12", cy: "7.5", r: "3" }
            circle { cx: "16.3", cy: "10.5", r: "3" }
            circle { cx: "14.7", cy: "15.2", r: "3" }
            circle { cx: "9.3", cy: "15.2", r: "3" }
            circle { cx: "7.7", cy: "10.5", r: "3" }
            // Center
            circle { cx: "12", cy: "11.5", r: "2", fill: "currentColor", stroke: "none" }
            // Stem
            path { d: "M12 17v5" }
            // Little leaf
            path { d: "M12 19c-1.5-0.5-2.5-1.5-2.5-2.5" }
        }
    }
}

#[component]
pub fn BellIcon(class: Option<String>) -> Element {
    let class = class.unwrap_or_default();
    rsx! {
        svg {
            class: "{class}",
            xmlns: "http://www.w3.org/2000/svg",
            width: "24", height: "24",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            path { d: "M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9" }
            path { d: "M10.3 21a1.94 1.94 0 0 0 3.4 0" }
        }
    }
}

#[component]
pub fn PlusIcon(class: Option<String>) -> Element {
    let class = class.unwrap_or_default();
    rsx! {
        svg {
            class: "{class}",
            xmlns: "http://www.w3.org/2000/svg",
            width: "24", height: "24",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            path { d: "M5 12h14" }
            path { d: "M12 5v14" }
        }
    }
}

#[component]
pub fn TrashIcon(class: Option<String>) -> Element {
    let class = class.unwrap_or_default();
    rsx! {
        svg {
            class: "{class}",
            xmlns: "http://www.w3.org/2000/svg",
            width: "24", height: "24",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            path { d: "M3 6h18" }
            path { d: "M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6" }
            path { d: "M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2" }
        }
    }
}

#[component]
pub fn PoopIcon(class: Option<String>) -> Element {
    let class = class.unwrap_or_default();
    // Cute poop swirl with a face
    rsx! {
        svg {
            class: "{class}",
            xmlns: "http://www.w3.org/2000/svg",
            width: "24", height: "24",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "1.8",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            // Swirl top
            path { d: "M11.5 3c1.5 0 2.5 1.2 2.5 2.5 0 .8-.3 1.2-.5 1.5h1c1.7 0 3 1.3 3 3 0 .7-.2 1.3-.5 1.8" }
            // Body
            path { d: "M7 12c-1.1.5-2 1.7-2 3.2C5 17.9 7.5 20 12 20s7-2.1 7-4.8c0-1.5-.9-2.7-2-3.2" }
            // Middle bulge
            path { d: "M7 12c0-1.5 1-2.8 2.5-2.8h5c1.5 0 2.5 1.3 2.5 2.8" }
            // Eyes
            circle { cx: "10", cy: "15", r: "0.8", fill: "currentColor", stroke: "none" }
            circle { cx: "14", cy: "15", r: "0.8", fill: "currentColor", stroke: "none" }
            // Smile
            path { d: "M10.5 17c.5.4 1 .5 1.5.5s1-.1 1.5-.5" }
        }
    }
}

#[component]
pub fn MapPinIcon(class: Option<String>) -> Element {
    let class = class.unwrap_or_default();
    rsx! {
        svg {
            class: "{class}",
            xmlns: "http://www.w3.org/2000/svg",
            width: "24", height: "24",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            path { d: "M20 10c0 6-8 12-8 12s-8-6-8-12a8 8 0 0 1 16 0Z" }
            circle { cx: "12", cy: "10", r: "3" }
        }
    }
}

#[component]
pub fn BookIcon(class: Option<String>) -> Element {
    let class = class.unwrap_or_default();
    rsx! {
        svg {
            class: "{class}",
            xmlns: "http://www.w3.org/2000/svg",
            width: "24", height: "24",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            path { d: "M4 19.5A2.5 2.5 0 0 1 6.5 17H20" }
            path { d: "M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z" }
        }
    }
}

#[component]
pub fn ActivityIcon(class: Option<String>) -> Element {
    let class = class.unwrap_or_default();
    rsx! {
        svg {
            class: "{class}",
            xmlns: "http://www.w3.org/2000/svg",
            width: "24", height: "24",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            path { d: "M22 12h-4l-3 9L9 3l-3 9H2" }
        }
    }
}

#[component]
pub fn CheckIcon(class: Option<String>) -> Element {
    let class = class.unwrap_or_default();
    rsx! {
        svg {
            class: "{class}",
            xmlns: "http://www.w3.org/2000/svg",
            width: "24", height: "24",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            path { d: "M20 6 9 17l-5-5" }
        }
    }
}

#[component]
pub fn TrophyIcon(class: Option<String>) -> Element {
    let class = class.unwrap_or_default();
    rsx! {
        svg {
            class: "{class}",
            xmlns: "http://www.w3.org/2000/svg",
            width: "24", height: "24",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            // Handles
            path { d: "M6 9H4.5a2.5 2.5 0 0 1 0-5H6" }
            path { d: "M18 9h1.5a2.5 2.5 0 0 0 0-5H18" }
            // Base
            path { d: "M4 22h16" }
            path { d: "M10 14.66V17c0 .55-.47.98-.97 1.21C7.85 18.75 7 20.24 7 22" }
            path { d: "M14 14.66V17c0 .55.47.98.97 1.21C16.15 18.75 17 20.24 17 22" }
            // Cup
            path { d: "M18 2H6v7a6 6 0 0 0 12 0V2Z" }
        }
    }
}

#[component]
pub fn SettingsIcon(class: Option<String>) -> Element {
    let class = class.unwrap_or_default();
    rsx! {
        svg {
            class: "{class}",
            xmlns: "http://www.w3.org/2000/svg",
            width: "24", height: "24",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            circle { cx: "12", cy: "12", r: "3" }
            path { d: "M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" }
        }
    }
}
