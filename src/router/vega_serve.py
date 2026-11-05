"""kwant's Laya sidecar for the vega engine (`laya_engine = "vega"`).

laya-serve has no switch for vegaml: it can only be turned on from Python with
`laya.Agent(vegaml=True)`. kwant writes this file to `<data>/laya/vega_serve.py` and runs it
with the Python in `<data>/laya/venv-vega` instead of `laya-serve`.

It answers the two routes kwant uses, with the shape kwant already parses:

- `GET /health`: 200 once the agent is built and warmed (the port opens only then).
- `POST /v1/systemone`: `{"state": ..., "questions": {...}}` -> `{"model", "answers", "usage"}`.
  Each answer is laya's shape plus vega's own fields (`route`, `route_reason`,
  `prediction_set`, `abstain`, `unbound`), passed through untouched.

It binds loopback only, builds the agent once, and runs every decision on one worker thread so
`/health` keeps answering while torch is busy. Everything above `create_app` is plain Python,
so `scripts/test_vega_serve.py` checks the shape with a fake agent and no model.
"""
from __future__ import annotations

import json
import logging
import os
import sys
import time
from typing import Any, Dict, Mapping, Tuple

ENGINE = "vega"
MAX_BODY_BYTES = 2 * 1024 * 1024
MAX_QUESTIONS = 64
MAX_STATE_CHARS = 50_000
MAX_CHOICE_OPTIONS = 100
MAX_SCORE_LEVELS = 32
QUESTION_TYPES = ("choice", "score", "noul")
LOOPBACK = ("127.0.0.1", "::1", "localhost")
DEFAULT_THREADS = 4
MAX_THREADS = 64

WARMUP_STATE = {"prompt": "hello, can you help me plan a trip?"}
WARMUP_QUESTIONS = {
    "warmup": {
        "type": "choice",
        "instructions": "Who should answer?",
        "criteria": {"a": "travel and planning", "b": "software engineering"},
    }
}

log = logging.getLogger("kwnt.vega")


class RequestError(Exception):
    """A request kwant (or anyone else on loopback) got wrong: answered with `status`."""

    def __init__(self, status: int, detail: str):
        super().__init__(detail)
        self.status = status
        self.detail = detail


def parse_body(raw: bytes) -> Tuple[Any, Dict[str, Any]]:
    """`(state, questions)` from a request body, or a `RequestError` saying what is wrong."""
    if len(raw) > MAX_BODY_BYTES:
        raise RequestError(413, "request body is larger than %d bytes" % MAX_BODY_BYTES)
    try:
        body = json.loads(raw)
    except (ValueError, UnicodeDecodeError):
        raise RequestError(400, "request body is not valid JSON")
    if not isinstance(body, dict) or "questions" not in body:
        raise RequestError(400, "request body must be an object with a 'questions' field")
    state, questions = body.get("state"), body["questions"]
    if state is None:
        raise RequestError(400, "'state' is required")
    if not isinstance(state, (str, dict, list)):
        raise RequestError(400, "'state' must be a string, an object or a list")
    text = state if isinstance(state, str) else json.dumps(state, ensure_ascii=False)
    if len(text) > MAX_STATE_CHARS:
        raise RequestError(413, "state is longer than %d characters" % MAX_STATE_CHARS)
    if not isinstance(questions, dict) or not questions:
        raise RequestError(400, "'questions' must be a non-empty object")
    if len(questions) > MAX_QUESTIONS:
        raise RequestError(413, "too many questions (%d > %d)" % (len(questions), MAX_QUESTIONS))
    for qid, q in questions.items():
        if not isinstance(q, dict):
            raise RequestError(400, "question %r must be an object" % qid)
        if q.get("type") not in QUESTION_TYPES:
            raise RequestError(422, "question %r has type %r (choice, score or noul)" % (qid, q.get("type")))
        if not isinstance(q.get("instructions"), str) or not q["instructions"].strip():
            raise RequestError(422, "question %r needs 'instructions'" % qid)
        crit = q.get("criteria")
        if q["type"] == "choice":
            if not isinstance(crit, (dict, list)) or not crit:
                raise RequestError(422, "choice question %r needs criteria" % qid)
            if len(crit) > MAX_CHOICE_OPTIONS:
                raise RequestError(413, "too many choice options for %r (%d > %d)" % (qid, len(crit), MAX_CHOICE_OPTIONS))
        elif q["type"] == "score":
            if not isinstance(crit, list) or not crit:
                raise RequestError(422, "score question %r needs a list of levels" % qid)
            if len(crit) > MAX_SCORE_LEVELS:
                raise RequestError(413, "too many score levels for %r (%d > %d)" % (qid, len(crit), MAX_SCORE_LEVELS))
    return state, questions


def shape(result: Mapping[str, Any], model: str) -> Dict[str, Any]:
    """vega's predict result as laya-serve's `/v1/systemone` answer, vega's fields kept."""
    usage = result.get("usage") or {}
    out = {
        "model": model,
        "engine": ENGINE,
        "answers": dict(result.get("answers") or {}),
        "usage": {
            "input_tokens": int(usage.get("input_tokens") or 0),
            "output_tokens": int(usage.get("output_tokens") or 0),
        },
    }
    if result.get("latency_ms") is not None:
        out["latency_ms"] = result["latency_ms"]
    return out


def decide(agent: Any, raw: bytes, model: str) -> Tuple[int, Dict[str, Any]]:
    """One `/v1/systemone` request, start to finish: `(status, json body)`. Never raises."""
    try:
        state, questions = parse_body(raw)
    except RequestError as e:
        return e.status, {"detail": e.detail}
    try:
        result = agent.predict(state, questions)
    except ValueError as e:
        # laya's question validation names the question and what to fix
        return 422, {"detail": str(e)}
    except Exception:  # noqa: BLE001 -- never echo paths, weights or OOM text to a caller
        log.exception("vega inference failed")
        return 500, {"detail": "inference failed"}
    missing = [qid for qid in questions if qid not in (result.get("answers") or {})]
    if missing:
        log.error("vega answered without %s", ", ".join(missing))
        return 500, {"detail": "inference failed"}
    return 200, shape(result, model)


def health(model: str, started: float) -> Dict[str, Any]:
    return {"status": "ok", "engine": ENGINE, "model": model, "uptime_s": round(time.time() - started, 1)}


def check_host(host: str) -> str:
    """The bind address, which must be loopback: this server has no authentication."""
    host = (host or "127.0.0.1").strip()
    if host not in LOOPBACK:
        raise SystemExit("vega_serve binds loopback only; LAYA_HOST=%r is not" % host)
    return host


def create_app(agent: Any, model: str = ENGINE):
    """The FastAPI app over a built agent. Decisions run one at a time on one worker thread."""
    import asyncio
    from concurrent.futures import ThreadPoolExecutor

    from fastapi import FastAPI, Request
    from fastapi.responses import JSONResponse

    pool = ThreadPoolExecutor(max_workers=1, thread_name_prefix="vega-infer")
    started = time.time()
    app = FastAPI(title="kwant vega sidecar", docs_url=None, redoc_url=None, openapi_url=None)

    @app.get("/health")
    def _health() -> Dict[str, Any]:
        return health(model, started)

    @app.post("/v1/systemone")
    async def _systemone(request: Request):
        length = request.headers.get("content-length")
        if length and length.isdigit() and int(length) > MAX_BODY_BYTES:
            return JSONResponse({"detail": "request body is larger than %d bytes" % MAX_BODY_BYTES}, status_code=413)
        raw = await request.body()
        t0 = time.perf_counter()
        status, body = await asyncio.get_running_loop().run_in_executor(pool, decide, agent, raw, model)
        ms = (time.perf_counter() - t0) * 1000.0
        return JSONResponse(body, status_code=status, headers={"X-Inference-Time-Ms": "%.2f" % ms})

    return app


def thread_count(env: Mapping[str, str]) -> int:
    """Torch threads for vega: `KWNT_VEGA_THREADS` (kwant sets 4), so it never takes the whole machine."""
    raw = (env.get("KWNT_VEGA_THREADS") or "").strip()
    try:
        n = int(raw) if raw else DEFAULT_THREADS
    except ValueError:
        n = DEFAULT_THREADS
    return max(1, min(n, MAX_THREADS))


def cap_threads(n: int) -> None:
    """Holds torch (and the OpenMP/MKL pools under it) to `n` threads. Call before torch does work."""
    for var in ("OMP_NUM_THREADS", "MKL_NUM_THREADS"):
        os.environ[var] = str(n)
    os.environ.setdefault("TOKENIZERS_PARALLELISM", "false")
    import torch

    torch.set_num_threads(n)
    try:
        torch.set_num_interop_threads(1)
    except RuntimeError:  # already set once torch ran something; the intra-op cap above still holds
        pass


def build_agent():
    """laya's agent on the vega engine, warmed with one decision so the first real one is quick."""
    cap_threads(thread_count(os.environ))
    import laya

    which = os.environ.get("KWNT_VEGA_MODEL", "").strip() or True
    t0 = time.perf_counter()
    agent = laya.Agent(vegaml=which)
    loaded = time.perf_counter()
    agent.predict(WARMUP_STATE, WARMUP_QUESTIONS)
    import torch

    log.warning("vega ready: laya %s, load %.1fs, warm-up %.1fs, %d torch threads",
                getattr(laya, "__version__", "?"), loaded - t0, time.perf_counter() - loaded, torch.get_num_threads())
    return agent


def main() -> None:
    logging.basicConfig(level=os.environ.get("LAYA_LOG_LEVEL", "warning").upper(),
                        format="%(levelname)s %(name)s: %(message)s")
    if sys.version_info < (3, 10):
        raise SystemExit("ERROR: the vega engine needs Python 3.10 or newer, this is %d.%d" % sys.version_info[:2])
    host = check_host(os.environ.get("LAYA_HOST", "127.0.0.1"))
    port = int(os.environ.get("LAYA_PORT", "8000"))
    agent = build_agent()
    import uvicorn

    uvicorn.run(create_app(agent), host=host, port=port, workers=1, access_log=False,
                log_level=os.environ.get("LAYA_LOG_LEVEL", "warning"))


if __name__ == "__main__":
    main()
