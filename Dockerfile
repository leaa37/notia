# Notia como servidor sin ventana (`notia --headless`), servido por HTTPS en
# el puerto 52480 con la interfaz web.
#
# Todo lo que Notia guarda queda en /app/data (`--data-root`):
#   /app/data/app          datos de Notia: catálogo de bibliotecas, contraseña
#                          del servidor, certificado TLS, sesiones, Telegram,
#                          WhatsApp, modelos y paquetes que se descargan.
#   /app/data/bibliotecas  una subcarpeta por biblioteca: notas, .notia/notia.db
#                          (SQLite), .agent/, chats y tableros.
# Montá /app/data como volumen y no se pierde nada al recrear el contenedor.

# --- 1. Interfaz web (dist/) -------------------------------------------------
FROM node:22-trixie-slim AS web
WORKDIR /src
COPY package.json package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY index.html public-task-manager.html public-space.html public-space-xgraph.html ./
COPY vite.config.ts tsconfig.json tsconfig.app.json tsconfig.node.json ./
COPY public ./public
COPY src ./src
RUN npm run build

# --- 2. Binario solo servidor (sin ventana, sin WebKit) ----------------------
FROM rust:1.97-trixie AS server
WORKDIR /src/src-tauri
COPY src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/build.rs ./
COPY src-tauri/src ./src
COPY src-tauri/app ./app
COPY src-tauri/backend-core ./backend-core
COPY src-tauri/resources/speech/model-manifest.json ./resources/speech/model-manifest.json
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/src-tauri/target \
    cargo build --release --locked --no-default-features \
    && cp target/release/notia /notia

# --- 3. Imagen final ----------------------------------------------------------
# Node.js queda disponible para WhatsApp (Baileys) y Python para los scripts
# del asistente; los dos instalan o guardan lo suyo dentro de /app/data.
FROM node:22-trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates python3 \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=server /notia /app/notia
COPY --from=web /src/dist /app/dist
VOLUME ["/app/data"]
EXPOSE 52480
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD ["node", "-e", "require('https').get({host:'127.0.0.1',port:52480,path:'/api/health',rejectUnauthorized:false},r=>process.exit(r.statusCode===200?0:1)).on('error',()=>process.exit(1))"]
ENTRYPOINT ["/app/notia", "--headless", "--data-root", "/app/data"]
