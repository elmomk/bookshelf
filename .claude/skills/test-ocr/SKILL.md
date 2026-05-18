---
name: test-ocr
description: Run Tesseract OCR on an image and test the Shopee package parser
allowed-tools: Bash, Read, Write, Glob, Grep
---

Test the Shopee OCR parsing pipeline on a local image.

## Usage

Provide a path to a Shopee screenshot image (PNG/JPG).

## Steps

1. Run Tesseract on the image:
   ```
   tesseract <image_path> stdout -l chi_tra+eng --psm 3
   ```

2. Save the raw OCR output to `/tmp/ocr_raw.txt`

3. Run the normalization + parser test by compiling and executing a temporary Rust program that:
   - Reads `/tmp/ocr_raw.txt`
   - Applies `normalize_ocr_text()` (copy the function from `src/api/shopee.rs`)
   - Applies `find_store_headers()`, `extract_title_from_section()`, `extract_store_from_section()`, `extract_code_from_section()`, `extract_due_date()`
   - Prints each extracted package with: title, store, code, due_date

4. Report:
   - Raw OCR text
   - Normalized text
   - Number of packages detected
   - Each package's fields
   - Any issues or patterns not being caught

Use this to iterate on OCR parsing improvements. The parser source is in `src/api/shopee.rs` (functions after line 66).
