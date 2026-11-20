# WireGuard to SOCKS5 in user space: no NET_ADMIN, no host network, no root. kwnt writes the
# config (docs/wireguard-helper.md); this image only runs it. Used by the `wireguard` compose profile.
FROM debian:bookworm-slim
ARG WIREPROXY_VERSION=v1.1.3
ARG TARGETARCH
RUN set -eu; \
    apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/*; \
    case "$TARGETARCH" in \
      amd64) sum=e88c1d090740373fc606c1bafd81d9a5eadc642cce5667616e20e9d7a444f51c ;; \
      arm64) sum=370e00bd2167960d1ecd1c3c1439715bbaa94a0a110a2040468670c9af6021b6 ;; \
      *) echo "unsupported architecture: $TARGETARCH" >&2; exit 1 ;; \
    esac; \
    curl -fsSL -o /tmp/wp.tgz "https://github.com/windtf/wireproxy/releases/download/${WIREPROXY_VERSION}/wireproxy_linux_${TARGETARCH}.tar.gz"; \
    echo "$sum  /tmp/wp.tgz" | sha256sum -c -; \
    tar -xzf /tmp/wp.tgz -C /usr/local/bin wireproxy; rm /tmp/wp.tgz; \
    apt-get purge -y curl && apt-get autoremove -y; \
    useradd --uid 10002 --no-create-home --shell /usr/sbin/nologin wireproxy
USER wireproxy
# Waits for the config (saved with `kwnt wireguard import` or Settings), so the service can be
# started before it exists. Never prints the config.
ENTRYPOINT ["/bin/sh", "-c", "until [ -s /data/wireguard/wireproxy.conf ]; do echo 'wireproxy: waiting for a WireGuard config (kwnt wireguard import)'; sleep 5; done; exec wireproxy -c /data/wireguard/wireproxy.conf"]
