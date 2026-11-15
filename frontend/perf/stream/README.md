# Streaming harness (dev only)

Measures what a streaming reply costs the browser in a long chat (roadmap 14b smooth-streaming). It is not
part of the app build or the test suite.

`main.tsx` renders the real `SpaceApp` inside Shell's frame, with a faked `fetch` and `EventSource`: one chat of
`?n=` messages (default 300; prose, code blocks and lists, one comparison table and one line chart result).
`window.__harness.stream(count, perSecond)` then sends a turn of `count` text tokens at `perSecond`, the
way the server's events arrive.

`run.mjs` serves the build, opens it in headless Chromium, runs the stream and prints one JSON line:

| field | what |
|---|---|
| `commitsPerSecond`, `commits` | React commits during the stream (a devtools hook counts them) |
| `scriptingMs`, `taskMs`, `layoutMs`, `styleMs` | CDP `Performance.getMetrics` deltas over the stream |
| `longestFrameMs`, `p95FrameMs`, `framesOver50`, `frames` | gaps between animation frames during the stream |
| `over50FirstHalf`, `over50SecondHalf` | where the slow frames fell |
| `lastWordsShown` | the reply's last words are on screen at the end |

```sh
cd frontend
npx vite build --config perf/stream/vite.config.mts          # on the team server: through bin/heavy
node perf/stream/run.mjs --n 300 --tokens 2000 --rate 50 --label after
node perf/stream/run.mjs --n 30 --tokens 2000 --rate 50
node perf/stream/run.mjs --n 300 --tokens 0                  # 40 s idle: what the machine itself costs
node perf/stream/run.mjs --n 300 --width 390 --shot mid.png  # screenshot at 60% of the stream
```

It needs `playwright-core` and a Chromium; point `PLAYWRIGHT_CORE` (path to its `index.mjs`) and `CHROMIUM`
at them. Numbers depend on the machine and its load: compare runs from the same machine and the same hour.
