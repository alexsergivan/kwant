# Kwant server: React UI embedded in a single Rust binary.
# AI CLIs are optional: --build-arg KWNT_CLIS=claude,codex,grok (any subset; default none).
# With Compose: KWNT_CLIS=claude,codex docker compose up -d --build
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

# The CLIs asked for in KWNT_CLIS, with Node only if one of them needs it. Empty when none.
FROM node:24-bookworm-slim AS clis
ARG KWNT_CLIS=""
RUN set -eu; mkdir -p /opt/clis/bin; \
    want() { case ",$(echo "$KWNT_CLIS" | tr -d ' ')," in *",$1,"*) return 0;; *) return 1;; esac; }; \
    for c in $(echo "$KWNT_CLIS" | tr ',' ' '); do \
      case "$c" in claude|codex|grok) ;; *) echo "KWNT_CLIS: unknown CLI '$c' (use claude, codex, grok)" >&2; exit 1;; esac; \
    done; \
    pkgs=""; \
    if want claude; then pkgs="$pkgs @anthropic-ai/claude-code"; fi; \
    if want codex; then pkgs="$pkgs @openai/codex"; fi; \
    if [ -n "$pkgs" ]; then \
      npm install -g --prefix /opt/clis --no-fund --no-audit $pkgs && npm cache clean --force \
      && cp /usr/local/bin/node /opt/clis/bin/node; \
    fi; \
    if want grok; then \
      apt-get update && apt-get install -y --no-install-recommends curl ca-certificates; \
      export HOME=/tmp/grok; mkdir -p "$HOME"; \
      curl -fsSL https://x.ai/cli/install.sh | bash \
      && cp -L "$HOME/.grok/bin/grok" /opt/clis/bin/grok; \
    fi; \
    ls /opt/clis/bin

FROM debian:bookworm-slim
ARG KWNT_CLIS=""
# The CLIs' own tools want git and a few basics; kwnt alone needs only CA certificates.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
      $(if [ -n "$KWNT_CLIS" ]; then echo git curl procps ripgrep less; fi) \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --uid 10001 --create-home --home-dir /home/kwnt --shell /bin/bash kwnt \
 && mkdir -p /data && chown kwnt:kwnt /data
COPY --from=clis /opt/clis /opt/clis
COPY --from=build /app/target/release/kwnt /usr/local/bin/kwnt
COPY docker/kwnt-entrypoint.sh /usr/local/bin/kwnt-entrypoint
RUN chmod 755 /usr/local/bin/kwnt-entrypoint
ENV PATH=/opt/clis/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    HOME=/home/kwnt \
    KWNT_IN_DOCKER=1 \
    KWNT_HOST=0.0.0.0 \
    KWNT_PORT=8799 \
    KWNT_DATA_DIR=/data \
    KWNT_LAYA_MANAGED=0 \
    LAYA_URL=http://laya:8000
# Not root: CLI sign-ins (`docker compose exec kwnt claude auth login`) land in /home/kwnt.
USER kwnt
WORKDIR /home/kwnt
VOLUME /data
EXPOSE 8799
ENTRYPOINT ["/usr/local/bin/kwnt-entrypoint"]
CMD ["kwnt", "serve"]
