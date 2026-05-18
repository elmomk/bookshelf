---
name: tailwind
description: Compile Tailwind CSS from input.css to assets/main.css
allowed-tools: Bash, Read
---

Compile Tailwind CSS for the project.

Run:
```
npx @tailwindcss/cli -i ./input.css -o ./assets/main.css --minify
```

Report whether it succeeded and the output file size.
