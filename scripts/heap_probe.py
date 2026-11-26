#!/usr/bin/env python3
"""THROWAWAY (red-proof/probe draft for #1283, never merged): one heaptrack profile of a release kwnt at idle.

Starts kwnt the way perf_budget.py's memory runs do (empty HOME and data folder, fake claude, one bot, one message,
registry sync off), under heaptrack. 30 s after the reply it reads kwnt's VmRSS/RssAnon, then SIGKILLs kwnt so
heaptrack's "leaked" list is exactly the heap that was live at that moment (nothing gets freed on the way out).
Usage: heap_probe.py KWNT OUT_DIR
"""
import json
import os
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import perf_budget as pb  # noqa: E402


def kwnt_child(parent):
    """The kwnt process under heaptrack's launcher script (a descendant whose comm is kwnt)."""
    todo, seen = [parent], set()
    while todo:
        pid = todo.pop()
        seen.add(pid)
        for entry in os.listdir("/proc"):
            if not entry.isdigit() or int(entry) in seen:
                continue
            try:
                stat = Path(f"/proc/{entry}/stat").read_text()
            except OSError:
                continue
            comm, rest = stat[stat.index("(") + 1:stat.rindex(")")], stat[stat.rindex(")") + 2:].split()
            if int(rest[1]) == pid:
                if comm == "kwnt":
                    return int(entry)
                todo.append(int(entry))
    return None


def main():
    kwnt, out = Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve()
    out.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix="heap-probe-"))
    home, binp, data = pb.make_machine(root)
    port = pb.free_port()
    # heaptrack's launcher is a shell script that needs more than the bare PATH; /usr/bin goes after the bare tools
    env = {"HOME": str(home), "PATH": f"{binp}:/usr/bin:/bin", "KWNT_DATA_DIR": str(data), "KWNT_HOST": "127.0.0.1",
           "KWNT_PORT": str(port), "KWNT_MCP_REGISTRY_URL": "http://127.0.0.1:9"}
    ht = subprocess.Popen(["heaptrack", "-o", str(out / "kwnt-idle"), str(kwnt), "serve", "--no-laya"], env=env,
                          stdout=open(out / "heaptrack-run.log", "w"), stderr=subprocess.STDOUT, cwd=str(root))
    t0 = time.perf_counter()
    up = pb.wait_health(port, 180)
    code, bot = pb.request(port, "POST", "/api/kwants", {"name": "Clio", "title": "Tester", "persona_prompt": "Test persona",
                                                         "avatar_sprite": "s", "driver_type": "api", "preferred_model": "claude"})
    events = pb.Events(port)
    events.kwant = bot["id"]
    events.ready.wait(10)
    pb.request(port, "POST", "/api/chat/send", {"kwant_id": bot["id"], "prompt": "hi", "sender_id": "user"})
    events.seen.wait(60)
    time.sleep(max(0.0, (events.first_text or time.perf_counter()) + 30 - time.perf_counter()))
    pid = kwnt_child(ht.pid)
    status = Path(f"/proc/{pid}/status").read_text()
    reading = {"cold_start_s_under_heaptrack": up - t0, "pid": pid, "vmrss_mb": pb.rss_mb(pid), "split": pb.rss_split(status),
               "threads": int(next(l.split()[1] for l in status.splitlines() if l.startswith("Threads:")))}
    print(json.dumps(reading), flush=True)
    (out / "reading.json").write_text(json.dumps(reading, indent=2))
    os.kill(pid, signal.SIGKILL)
    ht.wait(300)
    events.close()


if __name__ == "__main__":
    main()
