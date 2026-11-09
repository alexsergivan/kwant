#!/usr/bin/env python3
"""Throwaway mock upstream for the agents-in-vm spike (Lin, 2026-11-08).

One HTTP server on 127.0.0.1 standing in for three things:
  /v1/messages[...]      a scripted Anthropic Messages API (streaming SSE or JSON)
  /v1/oauth/token        an OAuth refresh endpoint that rotates the access token
  /mcp                   kwant's MCP gateway with the `kwnt.approve` permission tool
Every request is logged as one JSON line (method, path, ALL headers, notes) so the
spike can show which credential actually arrived upstream.

Scripted model: if the newest user turn carries a tool_result, log it and answer
"done". If the prompt contains `RUN: <cmd>`, answer with a Bash tool_use for <cmd>.
Otherwise answer "mock reply #<n>, saw <k> messages".
"""
import json, os, sys, threading, time, uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

STATE = sys.argv[2]            # upstream_state.json: which credentials are valid
LOG = sys.argv[3]
PORT = int(sys.argv[1])
lock = threading.Lock()
counter = {"n": 0}

def state():
    with open(STATE) as f:
        return json.load(f)

def save_state(s):
    tmp = STATE + ".tmp"
    with open(tmp, "w") as f:
        json.dump(s, f)
    os.replace(tmp, STATE)

def log(rec):
    with lock, open(LOG, "a") as f:
        f.write(json.dumps(rec) + "\n")

def sse(events):
    out = b""
    for e in events:
        out += f"event: {e['type']}\ndata: {json.dumps(e)}\n\n".encode()
    return out

def text_of(block):
    if isinstance(block, str):
        return block
    if block.get("type") == "text":
        return block.get("text", "")
    if block.get("type") == "tool_result":
        c = block.get("content")
        return c if isinstance(c, str) else "".join(text_of(x) for x in (c or []))
    return ""

class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def log_message(self, *a):
        pass

    def body(self):
        n = int(self.headers.get("content-length") or 0)
        return self.rfile.read(n) if n else b""

    def send(self, code, payload, ctype="application/json"):
        data = payload if isinstance(payload, bytes) else json.dumps(payload).encode()
        self.send_response(code)
        self.send_header("content-type", ctype)
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        log({"method": "GET", "path": self.path, "headers": dict(self.headers)})
        self.send(405 if self.path.startswith("/mcp") else 404, {"error": "no"})

    def do_DELETE(self):
        log({"method": "DELETE", "path": self.path, "headers": dict(self.headers)})
        self.send(200, {})

    def do_POST(self):
        raw = self.body()
        rec = {"t": time.time(), "method": "POST", "path": self.path, "headers": dict(self.headers)}
        try:
            if self.path.startswith("/v1/oauth/token"):
                return self.oauth(raw, rec)
            if self.path.startswith("/mcp"):
                return self.mcp(raw, rec)
            if self.path.startswith("/v1/messages/count_tokens"):
                rec["auth"] = self.check_auth()
                log(rec)
                return self.send(200, {"input_tokens": 12})
            if self.path.startswith("/v1/messages"):
                return self.messages(raw, rec)
            if self.path.endswith("/responses") and (self.path.startswith("/openai/") or self.path.startswith("/chatgpt/")):
                return self.openai_responses(raw, rec)
            log(rec)
            self.send(404, {"error": "unknown"})
        except Exception as e:  # keep the mock alive
            rec["error"] = repr(e)
            log(rec)
            self.send(500, {"error": repr(e)})

    def check_auth(self):
        s = state()
        key = self.headers.get("x-api-key")
        bearer = (self.headers.get("authorization") or "").removeprefix("Bearer ").strip()
        if key and key == s["api_key"]:
            return "api_key_ok"
        if bearer and bearer == s["access_token"]:
            return "oauth_ok"
        if bearer and bearer in s.get("expired_access_tokens", []):
            return "oauth_expired"
        return "rejected"

    def oauth(self, raw, rec):
        s = state()
        req = json.loads(raw or b"{}")
        rec["body"] = {k: ("<present>" if "token" in k else v) for k, v in req.items()}
        if req.get("grant_type") == "refresh_token" and req.get("refresh_token") == s["refresh_token"]:
            new_access = "sk-ant-oat01-REALCANARY-" + uuid.uuid4().hex
            new_refresh = "sk-ant-ort01-REALCANARY-" + uuid.uuid4().hex
            s.setdefault("expired_access_tokens", []).append(s["access_token"])
            s["access_token"], s["refresh_token"] = new_access, new_refresh
            save_state(s)
            rec["result"] = "rotated"
            log(rec)
            return self.send(200, {"access_token": new_access, "refresh_token": new_refresh,
                                   "expires_in": 3600, "token_type": "Bearer"})
        rec["result"] = "bad_refresh"
        log(rec)
        self.send(400, {"error": "invalid_grant"})

    def messages(self, raw, rec):
        req = json.loads(raw)
        auth = self.check_auth()
        rec["auth"] = auth
        rec["model"] = req.get("model")
        msgs = req.get("messages", [])
        rec["n_messages"] = len(msgs)
        rec["tools"] = [t.get("name") for t in req.get("tools", [])][:60]
        if auth != "api_key_ok" and auth != "oauth_ok":
            log(rec)
            msg = "OAuth token has expired" if auth == "oauth_expired" else "invalid x-api-key"
            return self.send(401, {"type": "error", "error": {"type": "authentication_error", "message": msg}})
        last = msgs[-1] if msgs else {"content": ""}
        content = last.get("content")
        blocks = content if isinstance(content, list) else [content]
        tool_results = [text_of(b) for b in blocks if isinstance(b, dict) and b.get("type") == "tool_result"]
        prompt = "".join(text_of(b) for b in blocks if not (isinstance(b, dict) and b.get("type") == "tool_result"))
        with lock:
            counter["n"] += 1
            n = counter["n"]
        tool = None
        if tool_results:
            rec["tool_result"] = tool_results
            reply = "done: tool ran"
        elif "RUN:" in prompt and "Bash" in rec["tools"]:
            cmd = prompt.split("RUN:", 1)[1].strip().splitlines()[0]
            tool = {"type": "tool_use", "id": "toolu_" + uuid.uuid4().hex[:20], "name": "Bash",
                    "input": {"command": cmd, "description": "spike probe"}}
            rec["asked_tool"] = cmd
            reply = None
        else:
            reply = f"mock reply #{n}, saw {len(msgs)} messages"
        log(rec)
        model = req.get("model", "mock")
        mid = "msg_" + uuid.uuid4().hex[:20]
        usage = {"input_tokens": 10, "output_tokens": 5}
        if not req.get("stream"):
            c = [tool] if tool else [{"type": "text", "text": reply}]
            return self.send(200, {"id": mid, "type": "message", "role": "assistant", "model": model,
                                   "content": c, "stop_reason": "tool_use" if tool else "end_turn",
                                   "stop_sequence": None, "usage": usage})
        ev = [{"type": "message_start", "message": {"id": mid, "type": "message", "role": "assistant",
               "model": model, "content": [], "stop_reason": None, "stop_sequence": None, "usage": usage}}]
        if tool:
            ev += [{"type": "content_block_start", "index": 0,
                    "content_block": {"type": "tool_use", "id": tool["id"], "name": "Bash", "input": {}}},
                   {"type": "content_block_delta", "index": 0,
                    "delta": {"type": "input_json_delta", "partial_json": json.dumps(tool["input"])}},
                   {"type": "content_block_stop", "index": 0},
                   {"type": "message_delta", "delta": {"stop_reason": "tool_use", "stop_sequence": None},
                    "usage": {"output_tokens": 5}}]
        else:
            ev += [{"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}},
                   {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": reply}},
                   {"type": "content_block_stop", "index": 0},
                   {"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": None},
                    "usage": {"output_tokens": 5}}]
        ev.append({"type": "message_stop"})
        self.send(200, sse(ev), "text/event-stream")

    # --- OpenAI Responses API (codex): just enough SSE for one text reply ---
    def openai_responses(self, raw, rec):
        s = state()
        bearer = (self.headers.get("authorization") or "").removeprefix("Bearer ").strip()
        ok = bearer in (s.get("openai_key"), s.get("chatgpt_access"))
        rec["auth"] = ("openai_key_ok" if bearer == s.get("openai_key") else "chatgpt_ok") if ok else "rejected"
        rec["account"] = self.headers.get("chatgpt-account-id")
        log(rec)
        if not ok:
            return self.send(401, {"error": {"message": "bad key", "type": "invalid_request_error"}})
        rid = "resp_" + uuid.uuid4().hex[:12]
        ev = [{"type": "response.created", "response": {"id": rid}},
              {"type": "response.output_item.done", "item": {"type": "message", "role": "assistant", "id": "msg_1",
               "content": [{"type": "output_text", "text": "mock codex reply via " + rec["auth"]}]}},
              {"type": "response.completed", "response": {"id": rid, "usage": {"input_tokens": 10,
               "input_tokens_details": {"cached_tokens": 0}, "output_tokens": 5,
               "output_tokens_details": {"reasoning_tokens": 0}, "total_tokens": 15}}}]
        self.send(200, sse(ev), "text/event-stream")

    # --- kwant MCP gateway stand-in: streamable HTTP, JSON responses only ---
    def mcp(self, raw, rec):
        s = state()
        req = json.loads(raw)
        rec["rpc"] = req.get("method")
        rec["gateway_auth"] = (self.headers.get("authorization") or "") == "Bearer " + s["gateway_token"]
        if not rec["gateway_auth"]:
            log(rec)
            return self.send(401, {"error": "gateway token"})
        rid = req.get("id")
        m = req.get("method")
        if rid is None:
            log(rec)
            self.send_response(202); self.send_header("content-length", "0"); self.end_headers()
            return
        if m == "initialize":
            res = {"protocolVersion": req["params"].get("protocolVersion", "2025-06-18"),
                   "capabilities": {"tools": {}}, "serverInfo": {"name": "kwnt", "version": "spike"}}
        elif m == "tools/list":
            res = {"tools": [{"name": "approve", "description": "kwant approval",
                              "inputSchema": {"type": "object", "properties": {
                                  "tool_name": {"type": "string"}, "input": {"type": "object"},
                                  "tool_use_id": {"type": "string"}}, "required": ["tool_name", "input"]}}]}
        elif m == "tools/call":
            args = req["params"].get("arguments", {})
            rec["approval_request"] = {"tool_name": args.get("tool_name"), "input": args.get("input")}
            res = {"content": [{"type": "text", "text": json.dumps(
                {"behavior": "allow", "updatedInput": args.get("input", {})})}]}
        else:
            res = {}
        log(rec)
        self.send(200, {"jsonrpc": "2.0", "id": rid, "result": res})

ThreadingHTTPServer.daemon_threads = True
ThreadingHTTPServer(("127.0.0.1", PORT), H).serve_forever()
