FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends chromium socat ca-certificates fonts-liberation fonts-noto-color-emoji \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 1000 --create-home browser \
    && chromium --version && socat -V
COPY browser-entrypoint.sh /usr/local/bin/browser-entrypoint
RUN test -s /usr/local/bin/browser-entrypoint && chmod +x /usr/local/bin/browser-entrypoint
USER browser
ENV HOME=/tmp XDG_CONFIG_HOME=/tmp/.config XDG_CACHE_HOME=/tmp/.cache
EXPOSE 9222
ENTRYPOINT ["/usr/local/bin/browser-entrypoint"]
