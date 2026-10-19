FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends chromium socat ca-certificates fonts-liberation fonts-noto-color-emoji \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 1000 --create-home browser \
    && chromium --version && socat -V
# The build context may be written under a strict umask (e.g. 077), so set the full mode:
# adding only the execute bit leaves the script unreadable to the unprivileged browser user.
COPY browser-entrypoint.sh /usr/local/bin/browser-entrypoint
RUN test -s /usr/local/bin/browser-entrypoint && chmod 755 /usr/local/bin/browser-entrypoint
USER browser
ENV HOME=/tmp XDG_CONFIG_HOME=/tmp/.config XDG_CACHE_HOME=/tmp/.cache
EXPOSE 9222
ENTRYPOINT ["/usr/local/bin/browser-entrypoint"]
