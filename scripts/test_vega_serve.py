"""Shape tests for kwant's vega sidecar (src/router/vega_serve.py) with a fake agent.

No laya, torch, fastapi or model download: everything kwant relies on (the request checks, the
answer shape with vega's own fields, the error statuses, loopback only, the thread cap) is plain
Python above `create_app`. Run: python3 -m unittest discover -s scripts -p 'test_*.py'
"""
import importlib.util
import json
import os
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.join(HERE, "..", "src", "router", "vega_serve.py")
spec = importlib.util.spec_from_file_location("vega_serve", SRC)
vs = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vs)

QUESTIONS = {
    "lead_kwant": {
        "type": "choice",
        "instructions": "Which teammate should answer?",
        "criteria": {"Architect": "Software architect", "Writer": "Copywriter"},
    },
    "team_decision": {
        "type": "choice",
        "instructions": "Answer alone or discuss?",
        "criteria": {"single": "one bot", "discuss": "several bots"},
    },
}

VEGA_ANSWER = {
    "type": "choice",
    "probabilities": {"Architect": 0.81, "Writer": 0.19},
    "answer_confidence": 0.81,
    "confidence": 0.7,
    "prediction_set": ["Architect"],
    "abstain": False,
    "unbound": False,
    "readout": "cls",
    "choice": "Architect",
    "route": "auto",
    "route_reason": "singleton conformal set",
}


class FakeAgent:
    def __init__(self, result=None, error=None):
        self.result, self.error, self.calls = result, error, []

    def predict(self, state, questions):
        self.calls.append((state, questions))
        if self.error:
            raise self.error
        if self.result is not None:
            return self.result
        return {
            "answers": {qid: dict(VEGA_ANSWER) for qid in questions},
            "usage": {"input_tokens": 42, "output_tokens": 0},
            "latency_ms": 123.4,
            "modality": "text",
            "mode": "vegaml",
            "views": 1,
            "calibrated": True,
        }


def body(state=None, questions=None):
    return json.dumps({"state": state if state is not None else {"prompt": "fix the build"},
                       "questions": questions if questions is not None else QUESTIONS}).encode()


class ShapeTest(unittest.TestCase):
    def test_answer_has_kwants_shape_and_vegas_fields(self):
        agent = FakeAgent()
        status, out = vs.decide(agent, body(), "vega")
        self.assertEqual(status, 200)
        self.assertEqual(set(out), {"model", "engine", "answers", "usage", "latency_ms"})
        self.assertEqual(out["engine"], "vega")
        self.assertEqual(out["usage"], {"input_tokens": 42, "output_tokens": 0})
        lead = out["answers"]["lead_kwant"]
        self.assertEqual(lead["choice"], "Architect")
        self.assertEqual(lead["probabilities"], {"Architect": 0.81, "Writer": 0.19})
        for field in ("route", "route_reason", "prediction_set", "abstain", "unbound"):
            self.assertIn(field, lead)
        self.assertEqual(lead["route"], "auto")
        # vega's own bookkeeping isn't passed on
        self.assertNotIn("mode", out)
        # the agent got the state and questions as sent
        self.assertEqual(agent.calls[0], ({"prompt": "fix the build"}, QUESTIONS))
        json.dumps(out)  # serialisable

    def test_missing_usage_is_zero(self):
        out = vs.shape({"answers": {"q": {"choice": "a"}}}, "vega")
        self.assertEqual(out["usage"], {"input_tokens": 0, "output_tokens": 0})
        self.assertNotIn("latency_ms", out)

    def test_bad_requests_never_reach_the_agent(self):
        agent = FakeAgent()
        cases = [
            (b"not json", 400),
            (b"[]", 400),
            (json.dumps({"state": "x"}).encode(), 400),
            (json.dumps({"questions": QUESTIONS}).encode(), 400),
            (body(state=42), 400),
            (body(questions={}), 400),
            (body(questions={"q": "a string"}), 400),
            (body(questions={"q": {"type": "essay", "instructions": "x"}}), 422),
            (body(questions={"q": {"type": "choice", "instructions": "  ", "criteria": ["a"]}}), 422),
            (body(questions={"q": {"type": "choice", "instructions": "x"}}), 422),
            (body(questions={"q": {"type": "score", "instructions": "x", "criteria": {"a": 1}}}), 422),
            (body(state="x" * (vs.MAX_STATE_CHARS + 1)), 413),
            (body(questions={"q%d" % i: QUESTIONS["lead_kwant"] for i in range(vs.MAX_QUESTIONS + 1)}), 413),
            (body(questions={"q": {"type": "choice", "instructions": "x",
                                   "criteria": ["o%d" % i for i in range(vs.MAX_CHOICE_OPTIONS + 1)]}}), 413),
            (b"{" + b" " * vs.MAX_BODY_BYTES + b"}", 413),
        ]
        for raw, want in cases:
            status, out = vs.decide(agent, raw, "vega")
            self.assertEqual(status, want, raw[:80])
            self.assertIsInstance(out["detail"], str)
        self.assertEqual(agent.calls, [])

    def test_valid_question_kinds_pass(self):
        qs = {
            "c": {"type": "choice", "instructions": "x", "criteria": ["a"]},
            "s": {"type": "score", "instructions": "x", "criteria": ["low", "high"]},
            "n": {"type": "noul", "instructions": "x"},
        }
        state, questions = vs.parse_body(body(state="plain text", questions=qs))
        self.assertEqual(state, "plain text")
        self.assertEqual(questions, qs)

    def test_agent_errors(self):
        status, out = vs.decide(FakeAgent(error=ValueError("question 'q' needs criteria")), body(), "vega")
        self.assertEqual((status, out["detail"]), (422, "question 'q' needs criteria"))
        # internal errors are not echoed
        status, out = vs.decide(FakeAgent(error=RuntimeError("CUDA OOM at /secret/path")), body(), "vega")
        self.assertEqual((status, out), (500, {"detail": "inference failed"}))
        # an answer missing a question is a failure, not a partial answer
        status, out = vs.decide(FakeAgent(result={"answers": {"lead_kwant": VEGA_ANSWER}}), body(), "vega")
        self.assertEqual((status, out), (500, {"detail": "inference failed"}))

    def test_loopback_only(self):
        for host in ("127.0.0.1", "::1", "localhost", "", " 127.0.0.1 "):
            vs.check_host(host)
        for host in ("0.0.0.0", "::", "192.168.1.5", "example.com"):
            with self.assertRaises(SystemExit):
                vs.check_host(host)

    def test_thread_cap(self):
        self.assertEqual(vs.thread_count({}), 4)
        self.assertEqual(vs.thread_count({"KWNT_VEGA_THREADS": "4"}), 4)
        self.assertEqual(vs.thread_count({"KWNT_VEGA_THREADS": "2"}), 2)
        self.assertEqual(vs.thread_count({"KWNT_VEGA_THREADS": "0"}), 1)
        self.assertEqual(vs.thread_count({"KWNT_VEGA_THREADS": "999"}), vs.MAX_THREADS)
        self.assertEqual(vs.thread_count({"KWNT_VEGA_THREADS": "lots"}), 4)

    def test_health(self):
        h = vs.health("vega", 0.0)
        self.assertEqual((h["status"], h["engine"]), ("ok", "vega"))


if __name__ == "__main__":
    unittest.main()
