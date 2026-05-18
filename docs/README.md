# Life Manager Documentation

## Table of Contents

1. **[Architecture Overview](./01-architecture.md)** — System design, technology choices, and how the pieces fit together. *Inspired by "Designing Data-Intensive Applications" (Kleppmann).*

2. **[The Dioxus Fullstack Model](./02-dioxus-fullstack.md)** — How Dioxus 0.7 bridges client and server in a single Rust codebase. Server functions, signals, and the reactive rendering pipeline. *Inspired by "Programming Rust" (Blandy & Orendorff).*

3. **[Data Model & Storage](./03-data-model.md)** — SQLite schema design, migration strategy, connection pooling, and the shared model layer. *Inspired by "Database Internals" (Petrov).*

4. **[Components & UI Patterns](./04-components.md)** — The component hierarchy, gesture system, reactive state management, and the cyberpunk design language. *Inspired by "Refactoring UI" (Wathan & Schoger).*

5. **[Authentication & Security](./05-security.md)** — Tailscale-based auth, input validation, the threat model, and defense-in-depth approach. *Inspired by "The Web Application Hacker's Handbook" (Stuttard & Pinto).*

6. **[OCR Pipeline](./06-ocr-pipeline.md)** — How Shopee screenshots become structured data: image processing, Tesseract integration, multi-package extraction, and Chinese text handling. *Inspired by "Designing Machine Learning Systems" (Huyen).*

7. **[Deployment & Operations](./07-deployment.md)** — Docker packaging, Tailscale networking, PWA configuration, and the build pipeline. *Inspired by "Site Reliability Engineering" (Beyer et al.).*

8. **[Developer Guide](./08-developer-guide.md)** — How to add modules, extend the API, modify the theme, and use the CLI tools. *Inspired by "The Pragmatic Programmer" (Hunt & Thomas).*
