FROM debian:trixie-slim

RUN apt-get update && apt-get install -y ca-certificates tesseract-ocr tesseract-ocr-eng && rm -rf /var/lib/apt/lists/*

RUN useradd -r -m -s /bin/false appuser

WORKDIR /app

# Copy locally-built binary and public assets
COPY target/dx/bookclub/release/web/bookclub ./bookclub
COPY target/dx/bookclub/release/web/public ./public

# Copy PWA files that Dioxus doesn't include in the build output
COPY assets/sw.js ./public/sw.js
COPY assets/sw-register.js ./public/sw-register.js
COPY assets/manifest.json ./public/manifest.json
COPY assets/icons ./public/icons
COPY assets/fonts ./public/fonts

RUN mkdir -p /app/data && chown -R appuser:appuser /app

VOLUME /app/data

ENV DATABASE_PATH=/app/data/bookclub.db
ENV REQUIRE_AUTH=true
ENV IP=0.0.0.0
ENV PORT=8080

EXPOSE 8080

CMD ["./bookclub"]
