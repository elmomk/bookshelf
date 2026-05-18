# Tesseract OCR Integration in Life Manager

A comprehensive guide to how Life Manager uses Tesseract OCR to parse Shopee
pickup screenshots, extract structured package data from Traditional Chinese
text, and handle the many edge cases that arise from real-world OCR output.

---

## Table of Contents

1. [Introduction to Tesseract OCR](#1-introduction-to-tesseract-ocr)
2. [Tesseract Configuration](#2-tesseract-configuration)
3. [The OCR Pipeline](#3-the-ocr-pipeline)
4. [CJK Text Normalization](#4-cjk-text-normalization)
5. [Multi-Package Parsing Strategy](#5-multi-package-parsing-strategy)
6. [Date Extraction](#6-date-extraction)
7. [Update-Matching on Re-scan](#7-update-matching-on-re-scan)
8. [Testing & Debugging](#8-testing--debugging)
9. [Performance & Reliability](#9-performance--reliability)

---

## 1. Introduction to Tesseract OCR

[Tesseract](https://github.com/tesseract-ocr/tesseract) is an open-source
optical character recognition engine originally developed by Hewlett-Packard in
the 1980s and now maintained by Google. Since version 4.0, Tesseract uses an
LSTM (Long Short-Term Memory) neural network as its primary recognition engine,
a significant upgrade from the older pattern-matching approach. The LSTM
architecture allows Tesseract to recognize characters in context rather than in
isolation, which dramatically improves accuracy for scripts like Traditional
Chinese where individual strokes can be ambiguous (see *OCR with Tesseract,
OpenCV and Python* by the Tesseract community documentation, and the [official
Tesseract docs](https://tesseract-ocr.github.io/tessdoc/)).

### Why Tesseract for this project

Three properties make Tesseract the right choice for Life Manager:

1. **Open source and self-hostable.** The app runs on a private Tailscale
   network. There is no reason to send personal package screenshots to a
   third-party cloud OCR service. Tesseract runs as a local binary inside the
   Docker container.

2. **Traditional Chinese support.** Tesseract ships trained data files for
   `chi_tra` (Traditional Chinese) and `eng` (English). Shopee Taiwan
   screenshots contain a mix of both scripts -- store names are often in
   English or mixed CJK/Latin, while status labels and pickup instructions are
   in Traditional Chinese.

3. **Server-side execution.** Because Tesseract is a CLI tool invoked
   server-side, the mobile client only needs to capture and upload an image.
   No WASM OCR library is needed, keeping the client bundle small.

### How it is installed

The Dockerfile installs Tesseract and the required language packs in a single
`apt-get` layer:

```dockerfile
# Dockerfile, line 3
RUN apt-get update && apt-get install -y \
    ca-certificates \
    tesseract-ocr \
    tesseract-ocr-chi-tra \
    tesseract-ocr-eng \
    && rm -rf /var/lib/apt/lists/*
```

The `tesseract-ocr` package provides the engine binary. The two `-ocr-*`
packages install the LSTM-trained data files for Traditional Chinese and
English respectively. On Debian Trixie these are Tesseract 5.x data files.

---

## 2. Tesseract Configuration

The server invokes Tesseract with the following flags:

```rust
// src/api/shopee.rs, lines 37-43
tokio::process::Command::new("tesseract")
    .arg(&tmp_path)       // input image
    .arg("stdout")        // output to stdout (not a file)
    .arg("-l")
    .arg("chi_tra+eng")   // language packs
    .arg("--psm")
    .arg("3")             // page segmentation mode
    .output()
```

### Language selection: `-l chi_tra+eng`

The `-l` flag specifies which trained data models to load. The `+` syntax tells
Tesseract to use both models simultaneously and pick the best recognition
result per character. This is essential because Shopee screenshots contain:

- Traditional Chinese for UI labels (`待收貨`, `取件驗證碼`, `請於...前`)
- English for store/brand names (`QMAT OUTLET`, `DENPA GINGA`)
- Digits for pickup codes and dates

Without `eng`, Tesseract would try to interpret English brand names as Chinese
characters, producing garbage. Without `chi_tra`, all the Chinese status text
and pickup instructions would be unreadable.

### Page segmentation mode: `--psm 3`

The `--psm` (page segmentation mode) flag tells Tesseract how to interpret the
layout of the input image. Tesseract supports 14 PSM modes (0-13). The most
relevant ones:

| PSM | Name                          | Use case                          |
|-----|-------------------------------|-----------------------------------|
| 0   | OSD only                      | Orientation/script detection only |
| 1   | Auto with OSD                 | Auto + orientation detection      |
| 3   | **Fully automatic**           | **Default. Best for full pages.** |
| 4   | Single column                 | Text in one column                |
| 6   | Single block                  | One uniform block of text         |
| 7   | Single line                   | A single text line                |
| 8   | Single word                   | A single word                     |
| 11  | Sparse text                   | No particular order               |
| 13  | Raw line                      | Single line, no Tesseract hacks   |

PSM 3 ("fully automatic page segmentation, but no OSD") is chosen because
Shopee screenshots are full-page captures with multiple text regions: a tab
bar at the top, multiple package cards stacked vertically, each containing
store names, product titles, status labels, pickup instructions, and codes.
PSM 3 lets Tesseract figure out the block structure on its own.

For reference, *Tesseract OCR Best Practices* (from the official tessdoc)
recommends PSM 3 as the starting point for multi-block documents and suggests
switching to PSM 6 or 11 only when the layout is known to be a single block or
when text appears scattered without clear line structure. Screenshots from a
mobile app have a predictable top-to-bottom layout, so PSM 3 works well.

### Why no `--oem` flag

Tesseract 5.x defaults to `--oem 1` (LSTM only), which is the best mode for
CJK recognition. The legacy engine (`--oem 0`) does not support `chi_tra` well.
Omitting the flag accepts the default, which is correct.

---

## 3. The OCR Pipeline

The complete flow from camera tap to structured package data:

### Step 1: Image capture (client-side)

The `ShopeeOcr` component (`src/components/shopee_ocr.rs`) renders a camera
button. On tap, it creates a hidden `<input type="file" accept="image/*">`
element via JavaScript:

```rust
// src/components/shopee_ocr.rs, lines 13-27
let js = r#"
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = 'image/*';
    input.onchange = () => {
        const file = input.files[0];
        if (!file) { dioxus.send(''); return; }
        const reader = new FileReader();
        reader.onload = () => dioxus.send(reader.result);
        reader.onerror = () => dioxus.send('');
        reader.readAsDataURL(file);
    };
    input.oncancel = () => dioxus.send('');
    input.click();
"#;
```

On mobile browsers, `accept="image/*"` triggers the OS camera/gallery picker.
The selected image is read as a base64 data URL via `FileReader` and sent back
to Rust through `dioxus.send()` -- the Dioxus 0.7 JS-to-Rust bridge. The Rust
side receives it with `eval.recv::<String>().await` (line 30).

### Step 2: Upload to server

The base64 string is passed directly to the `ocr_shopee` server function:

```rust
// src/components/shopee_ocr.rs, lines 46-49
match ocr_shopee(base64_data).await {
    Ok(results) => {
        loading.set(false);
        on_results.call(results);
    }
    // ...
}
```

Dioxus fullstack serializes this as a POST request to the server. The image
travels as a base64 string in the request body -- not ideal for bandwidth, but
simple and avoids multipart form handling.

### Step 3: Server-side processing

The `ocr_shopee` function in `src/api/shopee.rs` performs:

1. **Auth check** (line 11): verifies the Tailscale header.
2. **Data URL stripping** (lines 13-17): removes the `data:image/...;base64,`
   prefix if present.
3. **Size check** (lines 19-22): rejects images over 10 MB (base64 inflates
   size by ~33%, so the limit is `10 * 1024 * 1024 * 4 / 3` bytes in base64).
4. **Base64 decode** (lines 24-26): decodes to raw image bytes.
5. **Temp file** (lines 28-33): writes bytes to a `NamedTempFile`. Tesseract
   requires a file path, not stdin.
6. **Tesseract invocation** (lines 35-48): runs the command with a 30-second
   timeout via `tokio::time::timeout`.
7. **Normalization** (line 60): the raw stdout text is passed through
   `normalize_ocr_text()`.
8. **Parsing** (line 63): `extract_packages()` splits the normalized text into
   structured `OcrResult` values.
9. **Empty check** (lines 65-67): returns an error if no packages were found,
   so the user gets feedback rather than silent success.

### Step 4: Results displayed

The `on_results` callback in the Shopee page receives a `Vec<OcrResult>` and
either adds new packages or updates existing ones (see section 7).

---

## 4. CJK Text Normalization

This is the most critical piece of the pipeline. Without normalization, almost
none of the downstream parsing patterns would match.

### The problem: Tesseract spaces between CJK characters

Tesseract was originally designed for Latin scripts where words are separated by
spaces. When processing CJK text (Chinese, Japanese, Korean), the engine often
inserts spaces between individual characters because its layout analysis treats
each character as a separate "word." A line that should read:

```
待收貨
```

comes out of Tesseract as:

```
待 收 貨
```

or worse, with inconsistent spacing:

```
取 件 驗證 碼
```

This is documented in the Tesseract issue tracker and is a known limitation
when using the default page segmentation modes on CJK text. The `chi_tra`
trained data improves but does not eliminate this behavior.

### The `normalize_ocr_text()` function

Located at `src/api/shopee.rs`, lines 440-484. It performs two passes:

**Pass 1: Fullwidth punctuation normalization (lines 444-456)**

Tesseract sometimes outputs fullwidth (Unicode) variants of ASCII punctuation.
These must be mapped to their ASCII equivalents so that downstream parsing
(which searches for `-`, `:`, `,`, etc.) works consistently:

```rust
// src/api/shopee.rs, lines 444-456
for ch in text.chars() {
    match ch {
        '﹣' | '－' => result.push('-'),
        '﹕' | '：' => result.push(':'),
        '﹐' | '，' => result.push(','),
        '﹒' => result.push('.'),
        '﹩' => result.push('$'),
        '﹔' | '；' => result.push(';'),
        '\u{FF3B}' => result.push('['), // ［
        '\u{FF3D}' => result.push(']'), // ］
        _ => result.push(ch),
    }
}
```

Note the inclusion of both fullwidth (`﹣` U+FE63) and wide (`－` U+FF0D)
variants -- Tesseract is inconsistent about which it produces.

**Pass 2: CJK space collapsing (lines 460-478)**

The algorithm walks the character array and, whenever it finds a CJK character
followed by one or more spaces followed by another CJK character, it skips the
spaces:

```rust
// src/api/shopee.rs, lines 463-478
while i < chars.len() {
    collapsed.push(chars[i]);
    if is_cjk_or_punct(chars[i]) && i + 1 < chars.len() && chars[i + 1] == ' ' {
        let mut j = i + 1;
        while j < chars.len() && chars[j] == ' ' {
            j += 1;
        }
        if j < chars.len() && is_cjk_or_punct(chars[j]) {
            i = j;  // skip all spaces between the two CJK chars
            continue;
        }
    }
    i += 1;
}
```

This is conservative: it only removes spaces when *both* the character before
and after the space run are CJK. A space between a CJK character and a Latin
character is preserved (e.g., `DENPA GINGA 電波銀河` stays as-is). This
follows text normalization principles described in *Natural Language Processing
with Python* (Bird, Klein & Loper) -- normalize aggressively within a script
class but preserve boundaries between scripts.

The `is_cjk_or_punct()` helper (lines 487-495) defines what counts as "CJK":

```rust
// src/api/shopee.rs, lines 487-495
fn is_cjk_or_punct(c: char) -> bool {
    (c >= '\u{4e00}' && c <= '\u{9fff}')   // CJK Unified Ideographs
    || c == '【' || c == '】' || c == '。' || c == '，' || c == '：'
    || c == '、' || c == '（' || c == '）' || c == '「' || c == '」'
    || c == '﹣' || c == '﹕'
}
```

The range `U+4E00..U+9FFF` covers the CJK Unified Ideographs block (over
20,000 characters). CJK punctuation marks like `【】。，` are included because
they also appear without spaces in natural Chinese text.

**Post-processing: bracket normalization (line 481)**

After space collapsing, ASCII brackets `[` and `]` are replaced with their
fullwidth CJK counterparts `【` and `】`:

```rust
// src/api/shopee.rs, line 481
collapsed = collapsed.replace('[', "【").replace(']', "】");
```

This is because Shopee product titles use `【】` brackets, but Tesseract
sometimes outputs the ASCII equivalents (especially after the fullwidth
normalization pass converted `［` and `］` to `[` and `]`). Unifying to `【】`
means the title extraction logic only needs one pattern.

### Common OCR errors in Traditional Chinese

Tesseract makes character substitution errors specific to Traditional Chinese.
The most impactful one observed in production:

- **`貨` (goods) misread as `吉` (lucky)**: The characters share structural
  similarities. This turns `待收貨` (awaiting pickup) into `待收吉`. The
  `find_store_headers()` function accounts for this by including `"待收吉"` in
  its header markers (line 136).

Other known substitutions do not affect parsing because they typically occur in
product titles or descriptions, which are extracted as-is rather than matched
against fixed patterns.

---

## 5. Multi-Package Parsing Strategy

A single Shopee screenshot can show multiple packages. The `extract_packages()`
function (`src/api/shopee.rs`, lines 74-123) must split the OCR text into
per-package sections and extract structured data from each.

### Why splitting by pickup codes fails

An earlier approach tried to split the text by `驗證碼` (verification code)
patterns. This fails because:

- Not all packages have pickup codes yet (packages in `待出貨` / shipping
  status have no code).
- Packages without codes would get merged with the previous package's section.

### The store header strategy

The current approach uses **store header lines** as section delimiters. In
Shopee's "待收貨" (awaiting pickup) list, each package card starts with a
line showing the store name and a status marker:

```
QMAT OUTLET 運動/瑜珈墊 巧拼地墊 按... 待收貨
DENPA GINGA 電波銀河 待收貨
```

The `find_store_headers()` function (`src/api/shopee.rs`, lines 131-177)
identifies these delimiter lines using two strategies:

**Primary strategy: status marker detection (lines 148-158)**

```rust
// src/api/shopee.rs, lines 136, 148-157
let header_markers = &["待收貨", "待出貨", "待收吉"];

let has_marker = header_markers.iter().any(|p| trimmed.contains(p));
if has_marker {
    let char_count = trimmed.chars().count();
    if char_count > 4 {
        headers.push(idx);
        continue;
    }
}
```

Lines containing `待收貨` or `待出貨` that are longer than 4 characters are
classified as store headers. The length check (`> 4`) filters out the tab bar
label (which is just `待收貨` on its own -- 3 characters) from actual store
header lines (which include the store name alongside the marker).

**Fallback strategy: store indicator + product title confirmation (lines 161-173)**

```rust
// src/api/shopee.rs, lines 139-142
let store_indicators = &[
    "旗艦", "官方", "專賣", "OUTLET", "outlet", "Shop", "shop", "SHOP",
    "Store", "store", "STORE", "旗艦店", "官方店", "GINGA", "DENPA",
];
```

If a line contains one of these brand-like tokens, is 3-50 characters long,
and is followed within 5 lines by a `【` bracket (indicating a product title),
it is treated as a store header. This handles cases where the OCR failed to
recognize the status marker but correctly read the store name.

### Per-section extraction

Once section boundaries are established, each section is processed
independently:

```rust
// src/api/shopee.rs, lines 91-109
for (i, &start_idx) in section_starts.iter().enumerate() {
    let end_idx = section_starts.get(i + 1).copied().unwrap_or(lines.len());
    let section_lines: Vec<&str> = lines[start_idx..end_idx]
        .iter().map(|(_, l)| *l).collect();
    let section_text = section_lines.join("\n");

    let title = extract_title_from_section(&section_lines);
    let store = extract_store_from_section(&section_lines);
    let code = extract_code_from_section(&section_text);
    let (due_date, date_is_estimate) = extract_due_date(&section_lines);

    if title.is_some() || store.is_some() || code.is_some() {
        results.push(OcrResult {
            title, store, code, due_date, date_is_estimate,
        });
    }
}
```

A section only produces an `OcrResult` if at least one of title, store, or code
was extracted. This filters out sections that are just UI chrome.

Adjacent results with identical titles are deduplicated (lines 113-115) to
handle cases where the OCR produces duplicate sections.

### Title extraction: `extract_title_from_section()` (lines 181-211)

Two strategies, tried in order:

1. **Bracket titles**: look for `【】` brackets (the standard Shopee product
   title format). Everything from `【` onward is the title:
   ```
   【QMAT】瑜珈墊 6mm TPE環保 雙面止滑
   ```

2. **Heuristic fallback**: skip the first line (store header) and find the
   first "long enough" line (8+ characters) that does not contain status
   keywords. A skip list of 20+ Chinese keywords (lines 196-201) filters out
   status labels, tracking info, and UI text.

### Store extraction: `extract_store_from_section()` (lines 219-253)

Pickup location parsing must handle multi-line text because Tesseract often
breaks a single pickup instruction across two lines:

```
請於 2026-03-28 前 , 至蝦皮店到店南港重
陽 - 智取店取件。取件驗證碼 ; 782399。
```

The function joins all section lines into a single string (line 221) and
searches for the `至...取件` (to...pickup) pattern. The store name is
extracted between `至` and `取件`, with the `蝦皮店到店` prefix stripped
(line 229).

This approach -- joining then searching -- follows principles from
*Introduction to Information Retrieval* (Manning, Raghavan & Schutze) on
structured information extraction: when field boundaries do not align with
line boundaries, operate on the concatenated text.

### Code extraction: `extract_code_from_section()` (lines 257-275)

Searches for pickup code patterns (`取件驗證碼`, `验证码`, `驗證碼`, `取件码`)
and extracts the first run of 4+ consecutive digits after the pattern. The
prefix stripping (line 263-266) handles various separators that Tesseract might
produce between the label and the digits:

```rust
// src/api/shopee.rs, lines 263-266
let after = after.trim_start_matches(|c: char| {
    c == '：' || c == ':' || c == ' ' || c == '\t' || c == ',' || c == '，'
    || c == '﹔' || c == ';' || c == '；'
});
```

The minimum length of 4 digits prevents false positives from quantities or
prices in the OCR text.

---

## 6. Date Extraction

The `extract_due_date()` function (`src/api/shopee.rs`, lines 281-314)
distinguishes between two semantically different date types:

### Pickup deadlines: `請於 DATE 前`

```
請於 2026-03-28 前 , 至蝦皮店到店...取件
```

This means "please pick up by 2026-03-28." The date is firm; missing it means
the package is returned. The function returns `date_is_estimate = false`.

```rust
// src/api/shopee.rs, lines 286-290
if let Some(pos) = line.find("請於") {
    let after = &line[pos + "請於".len()..];
    if let Some(date) = parse_chinese_date(after, &current_year) {
        return (Some(date), false);
    }
}
```

### Delivery estimates: `預計於 DATE - DATE 配達`

```
預計於 2026-03-20 - 2026-03-22 配達
```

This is a delivery window estimate. The function extracts the *later* date
(the end of the range) as the due date and marks it as an estimate:

```rust
// src/api/shopee.rs, lines 294-299
if let Some(pos) = line.find("預計於") {
    let after = &line[pos + "預計於".len()..];
    let dates = extract_dates_from_segment(after, &current_year);
    if let Some(last) = dates.last() {
        return (Some(last.clone()), true);
    }
}
```

Using the later date is a UX choice: the user should not be notified that a
package is "late" when it is still within the delivery window.

### The `parse_chinese_date()` function (lines 319-371)

Handles three date formats that appear in Shopee screenshots:

1. **Chinese format**: `3月25日` or `03月25日` -- month/day with Chinese
   suffixes. The parser extracts digits before `月` for month and between `月`
   and `日` for day.

2. **ISO/slash format with year**: `2026-03-25` or `2026/03/25` -- split on
   `-` or `/`, expect 3 segments where the first is a 4-digit year.

3. **Short slash format**: `3/25` or `03/25` -- split on `/`, expect 2
   segments. The current year is prepended.

### The leading-digit extraction trick (line 347)

A subtle but important detail. After normalization, a date segment might look
like:

```
2026-03-28 前 , 至蝦皮...
```

When split on `-`, the third segment is `28 前 , 至蝦皮...`. Rather than
trying to strip the trailing text, the parser uses `take_while(|c|
c.is_ascii_digit())` on each segment:

```rust
// src/api/shopee.rs, lines 345-348
let slash_parts: Vec<String> = text
    .split(|c: char| c == '/' || c == '-')
    .map(|s| s.trim().chars()
        .take_while(|c| c.is_ascii_digit()).collect::<String>())
    .filter(|s| !s.is_empty())
    .collect();
```

This extracts just `28` from `28 前 , 至蝦皮...` without needing to know what
comes after the digits. It is a robust pattern for extracting numbers from
noisy OCR output.

### Date range extraction: `extract_dates_from_segment()` (lines 377-417)

For delivery estimates with date ranges, this function scans the text for
all parseable dates. It first tries to find `YYYY-MM-DD` patterns by looking
for runs of 4 digits (potential years), then falls back to splitting on
range delimiters (`~`, `至`, `到`), and finally tries parsing the entire
segment as a single date.

---

## 7. Update-Matching on Re-scan

When the user scans a new screenshot, some packages may already exist in the
database (e.g., scanned earlier before a pickup code was assigned). The
`on_results` handler in `src/pages/shopee.rs` (lines 117-175) implements
smart matching to avoid duplicates and update existing records.

### Matching logic (lines 129-148)

For each OCR result, the code searches existing active (not picked up) packages
for a match using two strategies:

**Strategy 1: Title substring matching**

```rust
// src/pages/shopee.rs, lines 132-137
if let Some(ref ocr_title) = r.title {
    let ocr_clean = ocr_title.replace("【", "").replace("】", "");
    let pkg_clean = pkg.title.replace("【", "").replace("】", "");
    if !ocr_clean.is_empty()
        && (pkg_clean.contains(&ocr_clean) || ocr_clean.contains(&pkg_clean))
    {
        return true;
    }
}
```

Brackets are stripped before comparison because OCR may capture partial titles
(the screenshot might cut off part of the title). The bidirectional `contains`
check handles both cases: the OCR title being a substring of the stored title,
or vice versa.

**Strategy 2: Store + code matching**

```rust
// src/pages/shopee.rs, lines 140-147
if let (Some(ref pkg_store), Some(ref ocr_store)) = (&pkg.store, &r.store) {
    if pkg_store == ocr_store {
        if let (Some(ref pkg_code), Some(ref ocr_code)) = (&pkg.code, &r.code) {
            return pkg_code == ocr_code;
        }
    }
}
```

If both the store and code match exactly, it is the same package.

### Update behavior (lines 150-161)

When a match is found, the only update performed is adding a pickup code to a
package that previously had none:

```rust
// src/pages/shopee.rs, lines 150-161
if let Some(existing) = matching {
    if existing.code.is_none() {
        if let Some(ref new_code) = code {
            let id = existing.id.clone();
            let code_val = new_code.clone();
            spawn(async move {
                let _ = shopee_api::update_shopee_code(id, code_val).await;
                reload();
            });
        }
    }
}
```

This is the primary re-scan use case: a package was initially scanned while
still in `待出貨` (shipping) status with no code, and is now in `待收貨`
(awaiting pickup) status with a code assigned.

If no match is found, the OCR result is added as a new package.

---

## 8. Testing & Debugging

### Debugging OCR output

The normalized OCR text is logged at `debug` level:

```rust
// src/api/shopee.rs, line 61
tracing::debug!("OCR normalized text:\n{text}");
```

To see this output, run the server with `RUST_LOG=debug` (or
`RUST_LOG=life_manager=debug` to filter). This shows exactly what text the
parser is working with after normalization, which is essential for diagnosing
extraction failures.

### The `/test-ocr` skill and `ocr-dev` agent

The project includes a `/test-ocr` skill for automated testing with real
screenshots and an `ocr-dev` agent configuration for iterating on the parser.
These tools let you:

- Feed a screenshot through the OCR pipeline without the UI
- Compare expected vs. actual extracted fields
- Iterate on normalization and parsing logic

### Testing with real screenshots

The most reliable way to test is with actual Shopee screenshots from a phone:

1. Take a screenshot of the Shopee app's "待收貨" tab
2. Upload it through the ShopeeOcr component (or via the test skill)
3. Check the server logs for the normalized text
4. Verify the extracted packages match expectations

### Common failure modes

| Symptom | Likely cause | Fix |
|---------|-------------|-----|
| No packages extracted | Store headers not detected | Add the missing status marker variant to `header_markers` |
| Title is a status label | Skip list incomplete | Add the keyword to `extract_title_from_section`'s skip list |
| Store name truncated | Multi-line split | Already handled by `join(" ")` in `extract_store_from_section`; check if a new separator pattern appeared |
| Code not found | New separator between label and digits | Add the character to `trim_start_matches` in `extract_code_from_section` |
| Date wrong or missing | New date format in Shopee UI | Add format handling in `parse_chinese_date` |
| Characters garbled | Poor image quality or resolution | Advise user to crop tighter or use better lighting |
| `待收貨` not matched | OCR produced `待收吉` | Already handled; if a new substitution appears, add it to `header_markers` |

---

## 9. Performance & Reliability

Following principles from *Release It!* by Michael Nygard on designing for
production, the OCR pipeline includes several safeguards:

### 30-second timeout

```rust
// src/api/shopee.rs, lines 35-47
let output = tokio::time::timeout(
    std::time::Duration::from_secs(30),
    tokio::process::Command::new("tesseract")
        // ...
        .output(),
)
.await
.map_err(|_| ServerFnError::new("OCR timed out (30s limit)"))?
```

Tesseract can hang on pathological inputs (very large images, corrupt files).
The 30-second timeout prevents a single bad request from blocking the async
runtime. This is an instance of Nygard's "Timeouts" stability pattern.

In practice, Tesseract processes a typical Shopee screenshot (1080x2400, ~500KB)
in 2-5 seconds on the deployment hardware.

### 10 MB image size limit

```rust
// src/api/shopee.rs, lines 19-22
const MAX_BASE64_SIZE: usize = 10 * 1024 * 1024 * 4 / 3;
if raw_b64.len() > MAX_BASE64_SIZE {
    return Err(ServerFnError::new("Image too large (max 10MB)"));
}
```

The constant accounts for base64 encoding overhead (every 3 bytes of binary
become 4 bytes of base64, so 10 MB of image data is ~13.3 MB of base64). This
prevents denial-of-service through enormous uploads and keeps Tesseract's
memory usage bounded.

### Error handling and user feedback

Every failure mode produces a user-visible error:

- **Base64 decode failure**: `"Base64 decode error: ..."`
- **Temp file creation failure**: `"Temp file error: ..."`
- **Tesseract process failure**: `"OCR processing failed"` (with stderr logged
  at `error` level for server-side diagnosis)
- **Timeout**: `"OCR timed out (30s limit)"`
- **No packages found**: `"Could not extract any packages from image"`

On the client side, these errors surface through the `ShopeeOcr` component's
inline error display (a small magenta text below the camera button, line 97 of
`shopee_ocr.rs`) and through the page-level `ErrorBanner` component for
broader failures.

### Temp file cleanup

The use of `tempfile::NamedTempFile` ensures automatic cleanup. The temp file
is deleted when `tmp` goes out of scope at the end of `ocr_shopee()`, even if
Tesseract fails. This prevents disk space leaks from accumulated image files.

---

## File Reference

| File | Role |
|------|------|
| `src/api/shopee.rs` | Server-side OCR invocation, normalization, and parsing |
| `src/components/shopee_ocr.rs` | Client-side camera/file picker component |
| `src/models/shopee.rs` | `OcrResult` and `ShopeePackage` data models |
| `src/pages/shopee.rs` | Page component with re-scan matching logic |
| `Dockerfile` | Tesseract installation in the container |

---

## Further Reading

- [Tesseract Official Documentation](https://tesseract-ocr.github.io/tessdoc/) -- configuration flags, PSM modes, training data
- *OCR with Tesseract, OpenCV and Python* -- practical Tesseract usage patterns
- *Natural Language Processing with Python* (Bird, Klein & Loper) -- text normalization techniques
- *Introduction to Information Retrieval* (Manning, Raghavan & Schutze) -- structured information extraction from text
- *Release It!* (Michael Nygard) -- stability patterns: timeouts, fail fast, bulkheads
