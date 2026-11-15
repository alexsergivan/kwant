# Browser proxy

Sites treat a datacenter address (a VPS) differently from a home one: more CAPTCHAs, blocked sign-ins. Kwant can send the bots' browser, and only the browser, out through a proxy, so sites see the proxy's connection.

## Turn it on

Set `KWNT_BROWSER_PROXY` where kwant runs:

| Value | Meaning |
|---|---|
| `socks5://host:port` | SOCKS5. Names are looked up by the proxy, so the local resolver never sees the sites. |
| `socks5h://host:port` | Same as `socks5`. |
| `http://host:port`, `https://host:port` | HTTP(S) proxy. |

`KWNT_BROWSER_PROXY_BYPASS` adds hosts that skip the proxy (`;` or `,` separated, Chromium's syntax: `example.com`, `*.corp`, `10.9.0.0/16`).

Kwant's own services always skip the proxy: `laya`, `kwnt`, `browser`, `localhost`, `127.0.0.0/8`, `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `169.254.0.0/16`. Private networks therefore are not reachable through the proxy; list a host in the bypass only if it should go direct.

**Docker compose.** Put the value in `.env` next to `docker-compose.yml` (or in the shell) and run `docker compose up -d browser kwnt`. The browser container and `kwnt doctor` both read it:

```
KWNT_BROWSER_PROXY=socks5://172.17.0.1:1080
```

From a container, the Docker host is `172.17.0.1` on the default bridge. A proxy that listens only on the host's `127.0.0.1` is not reachable from the container; run a relay on `172.17.0.1` or use a proxy that listens on the bridge address.

**Kwant starting its own browser container** (Docker VM mode without compose). The setting is put into the container; changing or removing it replaces the container the next time a bot opens the browser (open pages close once).

**Local mode** (Chrome on your own computer). Kwant starts its Chrome with the proxy flags. If Chrome is already running from an earlier start (it is reused), close the kwant Chrome once so the next start picks the setting up.

## No credentials

A value with a user name or password (`socks5://user:pass@host:port`) is refused. Chromium cannot sign in to a SOCKS proxy at all, and a password in a command line or container environment would leak. Use a proxy that trusts the network it is reached from: a relay on the Docker bridge, or a WireGuard-to-SOCKS helper next to kwant. The refusal never repeats what you typed.

## Check it

```
docker compose exec kwnt kwnt doctor
```

The **Browser proxy** line is OK when the proxy answers and a request through it leaves from another address than a direct one, and shows both addresses. It warns when the address is the same (the proxy does not go anywhere else) or the test request fails, and fails when the proxy does not answer or the value is unusable.

## Notes

- Only the browser uses the proxy. The AI accounts' network traffic and kwnt's own downloads go direct.
- A reverse proxy hop, WebRTC and the sites' own fingerprinting still exist. A home address helps; it does not make the browser invisible.
