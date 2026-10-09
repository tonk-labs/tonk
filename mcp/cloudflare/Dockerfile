FROM node:24-bookworm-slim AS ui
WORKDIR /src/mcp
COPY mcp/package.json mcp/package-lock.json ./
RUN npm ci --ignore-scripts
COPY mcp/scripts/build-ui.mjs ./scripts/build-ui.mjs
COPY mcp/ui ./ui
COPY rust/tonk-prose/assets/tonk-prose-editor.js /src/rust/tonk-prose/assets/tonk-prose-editor.js
RUN npm run build:ui

FROM rust:1.93.0-bookworm AS native
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev clang cmake && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY rust ./rust
RUN cargo build --locked --release -p tonk-cli --bin tonk-mcp-runtime

FROM node:24-bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY mcp/package.json mcp/package-lock.json ./
RUN npm ci --omit=dev --ignore-scripts
COPY mcp/*.mjs mcp/tools.json ./
COPY mcp/ui ./ui
COPY --from=ui /src/mcp/ui/notebook-renderer.js ./ui/notebook-renderer.js
COPY --from=native /src/target/release/tonk-mcp-runtime /usr/local/bin/tonk-mcp-runtime
COPY mcp/cloudflare/entrypoint.sh /entrypoint.sh
EXPOSE 8080
ENTRYPOINT ["sh", "/entrypoint.sh"]
