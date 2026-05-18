# 4. Components & UI Patterns

> *"Good design is obvious. Great design is transparent."* — Joe Sparano
>
> Every component in Life Manager exists because it was extracted from duplication. The component hierarchy reflects real usage patterns, not speculative abstractions.

## Component Hierarchy

```
AppLayout
├── Header (route-aware title, neon-cyan glow)
├── Outlet (page content)
│   ├── ChecklistPage (Todos, Groceries)
│   │   ├── ErrorBanner
│   │   ├── Form (text input + date + ADD button)
│   │   ├── QuickAdd (dynamic chips with delete badges)
│   │   └── SwipeItem[] (per list item)
│   ├── Shopee
│   │   ├── ErrorBanner
│   │   ├── Form (title + store + code + ShopeeOcr)
│   │   ├── Store chips
│   │   └── SwipeItem[] (per package)
│   ├── Watchlist
│   │   ├── ErrorBanner
│   │   ├── Form (text + media type tabs)
│   │   └── SwipeItem[] (per watch item)
│   └── Period
│       ├── ErrorBanner
│       ├── Prediction card
│       ├── Log form (dates + symptom chips)
│       └── SwipeItem[] (per cycle)
└── TabBar (5 icon links)
```

## SwipeItem: The Gesture Engine

`SwipeItem` is the most complex component. It implements horizontal swipe detection with direction locking — a technique borrowed from native mobile development.

### The Problem

On a mobile device, the user might intend to scroll vertically or swipe an item horizontally. The app must decide which gesture the user intended before committing to either action.

### The Solution: Direction Locking

```rust
ontouchmove: move |e| {
    let dx = touch.x - start_x;  // Horizontal distance
    let dy = touch.y - start_y;  // Vertical distance

    if !direction_locked {
        if dx.abs() > 10.0 || dy.abs() > 10.0 {
            direction_locked = true;
            is_horizontal = dx.abs() > dy.abs();
        }
        return;  // Don't act until direction is determined
    }

    if !is_horizontal {
        return;  // Let the browser handle vertical scrolling
    }

    e.prevent_default();  // We own this gesture now
    translate_x.set(dx);  // Move the item with the finger
},
```

The first 10 pixels of movement are "dead zone" — the component watches but doesn't act. Once the user moves past 10px, the direction is locked. If horizontal, the component takes over and moves the item. If vertical, it yields to the browser's scroll handler.

### Threshold and Action

When the user lifts their finger, the component checks if the swipe exceeded the 100px threshold:

```rust
ontouchend: move |_| {
    animating.set(true);  // Enable CSS transition
    let tx = *translate_x.read();

    if tx > 100.0 {
        on_swipe_right.call(());  // Complete / add to defaults
    } else if tx < -100.0 {
        on_swipe_left.call(());   // Delete
    }

    translate_x.set(0.0);  // Snap back
},
```

The snap-back animation uses CSS: `transition-transform duration-200 ease-out`. The transition is only enabled during snap-back (not during drag) to avoid lag.

### Two-Phase Right Swipe

For checklist items, right-swipe has two behaviors:
- **First swipe** (item not done): Mark as complete
- **Second swipe** (item already done): Add text to quick-add defaults

```rust
on_swipe_right: move |_| {
    if done {
        // Already complete — save as default chip
        defaults::add_default(item_text.clone(), category).await;
        reload_chips();
    } else {
        // Mark complete
        checklist::toggle_checklist(id.clone()).await;
        reload();
    }
},
```

## QuickAdd: Dynamic Chip System

Quick-add chips start as hardcoded defaults and evolve as the user interacts:

### Lifecycle

1. **First visit**: `list_defaults()` returns empty → seed from hardcoded list (`["Milk", "Eggs", ...]`)
2. **Tap a chip**: Creates a new checklist item with that text
3. **Swipe a completed item right**: Adds its text to the defaults
4. **Tap the X badge**: Removes the chip from defaults

### Scroll Fade Hint

The chip container scrolls horizontally but hides the scrollbar. A gradient overlay on the right edge hints at more content:

```rust
div { class: "relative",
    // Fade overlay
    div { class: "absolute right-0 top-0 bottom-2 w-8
                  bg-gradient-to-l from-cyber-card to-transparent
                  pointer-events-none z-10" }
    // Scrollable chips
    div { class: "flex gap-2 overflow-x-auto pb-2 scrollbar-hide",
        for chip in chips { ... }
    }
}
```

The `pointer-events-none` ensures the gradient doesn't block tap events on chips beneath it.

## ErrorBanner: Feedback Without Interruption

Every page has an `error_msg` signal. When a server function fails, the error message appears as a dismissible banner at the top of the content area:

```rust
if let Some(text) = msg {
    div { class: "bg-neon-magenta/10 border border-neon-magenta/40
                  text-neon-magenta rounded-lg px-4 py-2 text-xs",
        span { "{text}" }
        button { onclick: move |_| message.set(None), "×" }
    }
}
```

The banner uses magenta — the universal "danger" color in the cyberpunk palette. It's inline with the content rather than a modal overlay, so the user can read the error while seeing the context that caused it.

## The Cyberpunk Design Language

### Color System

Every module has a signature neon color:

| Module | Color | Hex | Usage |
|--------|-------|-----|-------|
| Todos | Cyan | `#00f0ff` | Borders, buttons, active tab |
| Groceries | Green | `#00ff41` | Buttons, completion badge |
| Shopee | Orange | `#ff8c00` | Buttons, store chips |
| Watchlist | Purple | `#bf5af2` | Buttons, type tabs |
| Cycle | Pink | `#ff2d78` | Buttons, prediction card |
| Errors | Magenta | `#ff2d78` | Error banners, delete actions |
| Completed | Green | `#00ff41` | "DONE", "GOT IT", "WATCHED" labels |

### Glow Effects

Neon glow is achieved with `box-shadow`:

```css
.glow-cyan {
    box-shadow:
        0 0 8px rgba(0, 240, 255, 0.3),        /* Outer glow */
        inset 0 0 8px rgba(0, 240, 255, 0.05);  /* Subtle inner glow */
}
```

Text glow uses `text-shadow`:

```css
.text-glow-cyan {
    text-shadow: 0 0 8px rgba(0, 240, 255, 0.5);
}
```

### Scanlines

The CRT scanline effect is a fixed overlay on the entire viewport:

```css
.scanlines::after {
    content: '';
    position: fixed;
    inset: 0;
    pointer-events: none;
    z-index: 9999;
    background: repeating-linear-gradient(
        0deg,
        transparent, transparent 2px,
        rgba(0, 0, 0, 0.03) 2px,
        rgba(0, 0, 0, 0.03) 4px
    );
}
```

The 3% opacity is barely perceptible on static content but creates a subtle shimmer when scrolling — just enough to evoke a retro terminal without being distracting.

### Typography

JetBrains Mono is self-hosted (no Google Fonts dependency) for offline PWA support. The monospace font reinforces the cyberpunk aesthetic and ensures consistent character widths in codes, dates, and labels.

All labels use uppercase with wide letter-spacing:

```rust
class: "text-xs tracking-[0.3em] uppercase text-cyber-dim"
```

## Empty States

When a list has no items, the empty state serves double duty — it tells the user the list is empty AND teaches them how to interact:

```rust
div { class: "text-center py-12",
    p { class: "text-xs tracking-[0.3em] uppercase text-cyber-dim",
        "NO TASKS YET"
    }
    p { class: "text-[10px] text-cyber-dim/50 mt-3 tracking-wider",
        "SWIPE → COMPLETE • SWIPE ← DELETE"
    }
}
```

## Touch Target Sizing

All interactive elements follow the 44px minimum touch target guideline (Apple HIG). Chips use `py-2.5` (10px vertical padding) plus `text-xs` (12px font) plus border, yielding approximately 44px total height.

The date picker and form inputs use `py-2.5` as well, ensuring comfortable tap targets even on small screens.
