# Kwant server: React UI embedded in a single Rust binary.
FROM node:20-bookworm-slim AS web
WORKDIR /app/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ ./
RUN npm run build

FROM rust:1-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src src
COPY docker docker
COPY --from=web /app/frontend/dist frontend/dist
RUN cargo build --release --locked --bin kwnt

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /app/target/release/kwnt /usr/local/bin/kwnt
ENV KWNT_HOST=0.0.0.0 \
    KWNT_PORT=8799 \
    KWNT_DATA_DIR=/data \
    KWNT_LAYA_MANAGED=0 \
    LAYA_URL=http://laya:8000
VOLUME /data
EXPOSE 8799
CMD ["kwnt", "serve"]
