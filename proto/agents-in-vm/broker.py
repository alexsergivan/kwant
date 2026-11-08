#!/usr/bin/env python3
"""Throwaway credential broker for the agents-in-vm spike (Lin, 2026-11-08).

Listens on a Unix socket that is bind-mounted into a `--network none` container.
The CLI inside talks plain HTTP to 127.0.0.1:8080 (socat) -> this socket.

Per request:
  * route by prefix (/anthropic, /openai, /chatgpt, /xai, /mcp); anything else -> 403
  * per route, an allow-list of (method, path) -> anything else -> 403
  * drop whatever credential the container sent (it is a dummy) and add the real one,
    read from a host-only file on every request (so a host-side refresh is picked up)
  * OAuth: refresh on the host when near expiry, or once on a 401, then retry
Tokens are never logged; the log carries sha256 prefixes only.

The real design builds this in Rust inside kwnt; see design.md.
"""
import hashlib, http.client, json, os, re, socketserver, ssl, sys, threading, time
from http.server import BaseHTTPRequestHandler

SOCK, CREDS, LOG, UPSTREAM = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
# A port number is the local mock; "https://api.anthropic.com" is Gate 0 (the real API, the
# /anthropic route only, API-key mode only: no refresh is ever attempted against it).
REAL = UPSTREAM.startswith("https://")
UPSTREAM_PORT = 443 if REAL else int(UPSTREAM)
lock, refresh_lock = threading.Lock(), threading.Lock()
HOP = {"connection", "keep-alive", "proxy-connection", "transfer-encoding", "upgrade", "te",
       "trailer", "host", "content-length", "authorization", "x-api-key", "chatgpt-account-id"}
# route prefix -> (upstream host, port, path prefix upstream, allowed (method, regex))
ROUTES = {
    "/anthropic": ("127.0.0.1", UPSTREAM_PORT, "", [
        ("POST", r"/v1/messages(\?.*)?"), ("POST", r"/v1/messages/count_tokens(\?.*)?")]),
    "/openai": ("127.0.0.1", UPSTREAM_PORT, "/openai", [
        ("POST", r"/v1/responses(\?.*)?"), ("GET", r"/v1/models(\?.*)?")]),
    "/chatgpt": ("127.0.0.1", UPSTREAM_PORT, "/chatgpt", [
        ("POST", r"/backend-api/codex/responses(\?.*)?"), ("GET", r"/backend-api/codex/models(\?.*)?")]),
    "/xai": ("127.0.0.1", UPSTREAM_PORT, "/xai", [
        ("POST", r"/v1/(chat/completions|responses|messages)(\?.*)?"), ("GET", r"/v1/models(\?.*)?")]),
    # kwant's MCP gateway: the bot's own per-turn gateway token passes through untouched
    "/mcp": ("127.0.0.1", UPSTREAM_PORT, "/mcp", [("POST", r"(\?.*)?"), ("GET", r"(\?.*)?"),
                                                  ("DELETE", r"(\?.*)?")]),
}
if REAL:
    ROUTES = {"/anthropic": (UPSTREAM.removeprefix("https://").rstrip("/"), 443, "", ROUTES["/anthropic"][3])}

def h8(s):
    return hashlib.sha256(s.encode()).hexdigest()[:8] if s else "-"

def scrub(path):
    """Path for the log: long or id-like segments become sha8 so nothing secret is written."""
    p, _, q = path.partition("?")
    segs = [("h:" + h8(x)) if len(x) > 24 or re.search(r"[0-9a-f]{8}-|[A-Za-z0-9_-]{20,}", x) else x
            for x in p.split("/")]
    return "/".join(segs) + (("?" + "&".join(k.split("=")[0] + ("=" + v if len(v) <= 12 else "=h:" + h8(v))
                                              for k, _, v in (kv.partition("=") for kv in q.split("&"))))
                             if q else "")

def log(rec):
    rec["t"] = round(time.time(), 3)
    rec["path"] = scrub(rec.get("path", ""))
    with lock, open(LOG, "a") as f:
        f.write(json.dumps(rec) + "\n")

def creds():
    with open(CREDS) as f:
        return json.load(f)

def save_creds(c):
    tmp = CREDS + ".tmp"
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        json.dump(c, f)
    os.replace(tmp, CREDS)

def refresh(seen_access):
    """Host-side refresh. Single flight: a second caller that sees a newer token skips."""
    with refresh_lock:
        c = creds()
        o = c["anthropic"]["oauth"]
        if o["accessToken"] != seen_access:
            return "already_refreshed"
        conn = http.client.HTTPConnection("127.0.0.1", UPSTREAM_PORT, timeout=30)
        conn.request("POST", "/v1/oauth/token", json.dumps(
            {"grant_type": "refresh_token", "refresh_token": o["refreshToken"], "client_id": "spike"}),
            {"content-type": "application/json"})
        r = conn.getresponse()
        body = json.loads(r.read() or b"{}")
        if r.status != 200:
            return f"refresh_failed_{r.status}"
        o["accessToken"], o["refreshToken"] = body["access_token"], body["refresh_token"]
        o["expiresAt"] = int((time.time() + body["expires_in"]) * 1000)
        save_creds(c)
        return "refreshed"

def inject(route, headers, rec):
    """Returns the real credential headers for this route, or an error string."""
    c = creds()
    if route == "/anthropic":
        a = c["anthropic"]
        if REAL and a["mode"] != "api_key":
            return "gate 0 runs in api-key mode only"
        sent_bearer = headers.get("authorization", "").startswith("Bearer ")
        sent_key = bool(headers.get("x-api-key"))
        if a["mode"] == "api_key":
            if not sent_key:
                return "container must use the api-key dummy for this profile"
            return {"x-api-key": a["api_key"]}
        if not sent_bearer:
            return "container must use the oauth dummy for this profile"
        o = a["oauth"]
        if o["expiresAt"] / 1000 - time.time() < 60:
            rec["pre_refresh"] = refresh(o["accessToken"])
            o = creds()["anthropic"]["oauth"]
        return {"authorization": "Bearer " + o["accessToken"]}
    if route == "/openai":
        return {"authorization": "Bearer " + c["openai"]["api_key"]}
    if route == "/chatgpt":
        g = c["chatgpt"]
        return {"authorization": "Bearer " + g["access_token"], "chatgpt-account-id": g["account_id"]}
    if route == "/xai":
        return {"authorization": "Bearer " + c["xai"]["api_key"]}
    return {}

class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def log_message(self, *a):
        pass
    def address_string(self):
        return "unix"

    def handle_any(self):
        if "chunked" in (self.headers.get("transfer-encoding") or "").lower():
            body = b""  # codex streams its request body chunked
            while True:
                size = int(self.rfile.readline().split(b";")[0].strip() or b"0", 16)
                if size == 0:
                    while self.rfile.readline() not in (b"\r\n", b"\n", b""):
                        pass
                    break
                body += self.rfile.read(size)
                self.rfile.readline()
        else:
            n = int(self.headers.get("content-length") or 0)
            body = self.rfile.read(n) if n else None
        route = next((p for p in ROUTES if self.path == p or self.path.startswith(p + "/")
                      or self.path.startswith(p + "?")), None)
        sent_auth = self.headers.get("authorization", "") or self.headers.get("x-api-key", "")
        rec = {"method": self.command, "path": self.path, "route": route, "sent_cred_sha": h8(sent_auth)}
        if route is None:
            rec["decision"], rec["status"] = "deny_route", 403
            log(rec)
            return self.reply(403, {"error": "kwnt broker: host not allowed"})
        host, port, up_prefix, allowed = ROUTES[route]
        rest = self.path[len(route):] or ""
        if not any(m == self.command and re.fullmatch(rx, rest) for m, rx in allowed):
            rec["decision"], rec["status"] = "deny_path", 403
            log(rec)
            return self.reply(403, {"error": "kwnt broker: path not allowed"})
        for attempt in (1, 2):
            hdrs = {k: v for k, v in self.headers.items() if k.lower() not in HOP}
            if route == "/mcp":
                if self.headers.get("authorization"):
                    hdrs["authorization"] = self.headers["authorization"]
            else:
                real = inject(route, self.headers, rec)
                if isinstance(real, str):
                    rec["decision"] = "deny_cred_kind"
                    log(rec)
                    return self.reply(409, {"error": real})
                hdrs.update(real)
                rec["real_cred_sha"] = h8(real.get("authorization") or real.get("x-api-key") or "")
            if body is not None:
                hdrs["content-length"] = str(len(body))
            conn = (http.client.HTTPSConnection(host, port, timeout=600, context=ssl.create_default_context())
                    if REAL else http.client.HTTPConnection(host, port, timeout=600))
            conn.request(self.command, up_prefix + rest, body=body, headers=hdrs)
            r = conn.getresponse()
            if (r.status == 401 and route == "/anthropic" and attempt == 1 and not REAL
                    and creds()["anthropic"]["mode"] == "oauth"):
                r.read()
                rec["on_401"] = refresh(hdrs["authorization"].removeprefix("Bearer "))
                continue
            break
        rec["decision"] = "forward"
        rec["status"] = r.status
        log(rec)
        self.send_response(r.status)
        for k, v in r.getheaders():
            if k.lower() not in HOP:
                self.send_header(k, v)
        self.send_header("transfer-encoding", "chunked")
        self.end_headers()
        while True:
            chunk = r.read1(65536)
            if not chunk:
                break
            self.wfile.write(b"%x\r\n%s\r\n" % (len(chunk), chunk))
            self.wfile.flush()
        self.wfile.write(b"0\r\n\r\n")

    def reply(self, code, obj):
        data = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    do_GET = do_POST = do_DELETE = do_PUT = do_PATCH = do_HEAD = do_OPTIONS = handle_any

    def do_CONNECT(self):
        # Gate 0 points HTTPS_PROXY here, so a CLI call to any other host shows up (and is denied).
        log({"method": "CONNECT", "path": self.path, "route": None, "decision": "deny_route",
             "status": 403, "sent_cred_sha": "-"})
        self.reply(403, {"error": "kwnt broker: host not allowed"})

class S(socketserver.ThreadingMixIn, socketserver.UnixStreamServer):
    daemon_threads = True

if os.path.exists(SOCK):
    os.unlink(SOCK)
old = os.umask(0o177)
srv = S(SOCK, H)
os.umask(old)
srv.serve_forever()
