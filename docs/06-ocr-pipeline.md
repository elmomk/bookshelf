# 6. OCR Pipeline

> *"The best machine learning systems are the ones where you don't need machine learning."* — Chip Huyen
>
> Life Manager's OCR pipeline uses Tesseract (a traditional OCR engine) with rule-based extraction — no ML models, no API calls, no GPU required.

## The Problem

Shopee (a Southeast Asian e-commerce platform) sends pickup notifications as screenshots with structured information:
- **Product name**: 【義美生醫】乳清蛋白飲-奶茶（10包...
- **Pickup store**: 蝦皮店到店 南港重陽 - 智取店
- **Verification code**: 取件驗證碼：472960
- **Deadline**: 請於 2026-03-27 前

The user photographs this screen and uploads it. The app must extract the structured data automatically.

## Pipeline Architecture

```
Phone Camera / Screenshot
         │
         ▼
    File Picker (JS)
         │ base64-encoded image
         ▼
    dioxus.send() → Rust WASM
         │
         ▼
    Server Function (HTTP POST)
         │
    ┌────▼─────────────────────────┐
    │  1. Base64 decode            │
    │  2. Write to temp file       │
    │  3. Tesseract OCR            │
    │     -l chi_tra+eng           │
    │     --psm 3                  │
    │  4. Text extraction          │
    │     ├─ extract_all_codes()   │
    │     ├─ extract_all_stores()  │
    │     └─ extract_all_titles()  │
    │  5. Package assembly         │
    └────┬─────────────────────────┘
         │ Vec<OcrResult>
         ▼
    Client receives results
         │
    ┌────▼─────────────────────────┐
    │  1 result → fill form fields │
    │  N results → auto-add all    │
    └──────────────────────────────┘
```

## Tesseract Configuration

```bash
tesseract <image> stdout -l chi_tra+eng --psm 3
```

- **`-l chi_tra+eng`**: Traditional Chinese + English. Required because Shopee Taiwan uses both scripts.
- **`--psm 3`**: Fully automatic page segmentation. Better than PSM 6 (uniform block) for mixed-layout screenshots with images, buttons, and text.

## Multi-Package Extraction

A single screenshot might contain multiple packages. The extraction engine identifies all packages by finding repeated patterns.

### Code Extraction (`extract_all_codes`)

Scans the entire OCR text for ALL occurrences of pickup code patterns:

```rust
let code_patterns = &["取件驗證碼", "验证码", "驗證碼", "取件码"];

// Iterate through all matches, not just the first
let mut search_from = 0;
while let Some(pos) = text[search_from..].find(pattern) {
    // Extract digits after the pattern
    let code: String = after.chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if code.len() >= 4 {
        results.push((abs_pos, code));
    }
    search_from = abs_pos + pattern.len();
}
```

Fallback: if no pattern-based codes are found, look for standalone 6–10 digit numbers.

### Store Extraction (`extract_all_stores`)

Two patterns for finding pickup locations:

1. **至...取件**: `"至 蝦皮店到店 南港重陽 - 智取店 取件"` → `"南港重陽 - 智取店"`
2. **店到店 LOCATION**: `"店到店 南港重陽"` → `"南港重陽"`

The `蝦皮店到店` prefix is stripped — the user cares about the location, not the logistics provider.

### Title Extraction (`extract_all_titles`)

1. **Bracket pattern**: Text in `【】` brackets is almost always a product name
2. **Heuristic fallback**: Lines longer than 8 characters that don't match UI patterns (button labels, navigation, prices)

UI patterns are filtered out:

```rust
let skip_patterns = &[
    "待收貨", "待付款", "待出貨", "訂單", "退貨", "退款", "追蹤",
    "取件", "驗證", "請於", "猜你", "購買", "蝦皮", "已售出",
];
```

### Package Assembly

Once all codes, stores, and titles are extracted with their byte positions, they're assembled into packages:

- **Multiple codes**: Each code becomes a package. The nearest preceding store and title are associated with it.
- **Multiple titles, one code**: Titles are joined with ` + ` into a single package.
- **Single everything**: One package with whatever was found.

```rust
if codes.len() > 1 {
    // Each code = separate package
    codes.iter().map(|(pos, code)| {
        let store = stores.iter()
            .filter(|(sp, _)| *sp < *pos)  // Store must appear before code
            .last()
            .map(|(_, s)| s.clone());
        OcrResult { title, store, code: Some(code.clone()) }
    }).collect()
}
```

## Client-Side Handling

The Shopee page handles single vs. multiple results differently:

```rust
on_results: move |results: Vec<OcrResult>| {
    if results.len() == 1 {
        // Fill form for review
        input_code.set(result.code);
        input_store.set(result.store);
        input_title.set(result.title);
    } else {
        // Auto-add all packages
        for r in results {
            shopee_api::add_shopee(title, store, code).await;
        }
        reload();
    }
}
```

Single results let the user verify and edit before adding. Multiple results skip the review step because manually editing N packages is tedious.

## Limitations

1. **OCR accuracy**: Tesseract is imperfect, especially with stylized fonts, low contrast, or compressed screenshots
2. **Layout sensitivity**: PSM 3 works well for standard Shopee layouts but may struggle with unusual formatting
3. **No image preprocessing**: No rotation correction, contrast enhancement, or region detection — the raw screenshot goes directly to Tesseract
4. **CPU-intensive**: Each OCR call spawns a Tesseract process that consumes significant CPU and RAM
