# WireGuard for the browser (built in)

Send the bots' browser out through a WireGuard connection (for example your home router) without a host relay, iptables or root. It builds on [the browser proxy setting](browser-proxy.md): Kwant runs a small `wireproxy` helper (WireGuard in user space, offering a SOCKS5 port) as an optional compose service, and the browser uses it.

## Use a connection made for Kwant

**One WireGuard connection can only be used from one place at a time.** A router hands the tunnel to whoever connected last, so two programs sharing one connection keep knocking each other off. Create a **separate** connection for Kwant. Never paste the config another program already uses (another bot's proxy, your phone or laptop).

On a FRITZ!Box: *Home Network → Network → … WireGuard → Add connection → "Connect a single device"*, name it for Kwant, make it a full tunnel (the config's `AllowedIPs` contains `0.0.0.0/0`), and download the config file.

## Set it up (Docker compose)

1. Save the config. From the folder with `docker-compose.yml`:

   ```sh
   docker compose run --rm -T kwnt kwnt wireguard import < kwant.conf
   ```

   (or paste it into Settings → Browser → WireGuard, or `PUT /api/settings/wireguard` with `{"config": "..."}`). Kwant checks it, prints a summary without any key, and warns if it is not a full tunnel.
2. Add two lines to `.env`:

   ```sh
   COMPOSE_PROFILES=wireguard
   KWNT_BROWSER_PROXY=socks5://wireproxy:1080
   ```
3. `docker compose up -d`, then `docker compose exec kwnt kwnt doctor`. The **Browser proxy** check should say sites see your home address, different from the server's.

`kwnt wireguard status` shows what is saved (no keys); `kwnt wireguard remove` deletes it. To stop using the tunnel, remove the two `.env` lines and `docker compose up -d --remove-orphans`.

## What is stored, and where

- The config is `wireproxy.conf` in its own volume (`kwnt-wireguard`), mode `0600`, folder `0700`. The helper mounts that volume read-only and sees nothing else of Kwant's data. Kwant's settings and API never return the key; the API calls are person-only.
- The private key is kept **as plain text in that file**, because `wireproxy` needs to read it. Treat the volume like any secret: back it up carefully, and don't copy it into compose files or `.env`.
- The key is never printed or logged; errors name the field (`PrivateKey`), not its value.
- Settings that user space has no use for (`PostUp`, `Table`, `ListenPort`…) are dropped; `Address` is made `/32` (`/128`).
- The helper has no published port: only containers on the compose network reach it, and the browser's default bypass keeps Kwant's own services off the tunnel.

## Limits

- Needs the `wireguard` profile; the helper image is built from `docker/wireproxy.Dockerfile` (wireproxy v1.1.3, checksum-pinned, amd64 and arm64).
- DNS names are resolved through the tunnel (SOCKS5), so a config without `DNS` relies on the peer answering.
- Not for the local/Mac browser mode: point `KWNT_BROWSER_PROXY` there at any SOCKS5 proxy you run yourself.
