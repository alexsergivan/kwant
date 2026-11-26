# Scroll probe: does the chat hold the reader's place?

`probe.mjs` loads the stream harness (`perf/stream`, a 300+ message mixed chat) in real Chromium and WebKit, scrolls
like a person (wheel, touch, keys, scrollbar thumb, load earlier, cards arriving), and counts **jumps**: a move of
more than 4 px in a frame with no input, or a move against the person's own input. A run also notes "pulled back"
(it ended at the bottom though the person was reading) and "never loaded" (load earlier didn't add rows).

```sh
npx vite build --config perf/stream/vite.config.mts        # -> perf/stream/dist
CHROMIUM=<chrome-headless-shell> WEBKIT=<pw_run.sh> \
node perf/scroll/probe.mjs --engine chromium|webkit|webkit-old --width 1280,390 \
  --scenario all|1..7 --stream on|off|both [--dist perf/stream/dist] [--json out.json] [--strip out.png] [--nolazy]
```

- Scenarios: 1 wheel up in small then big steps, 2 scroll up a long way and wait 10 s, 3 drag the scrollbar thumb up
  and down, 4 reach the top so load earlier fires and keep scrolling, 5 reading mid-chat while a message, an approval
  and a question card arrive and the strip hides, 6 phone touch-scroll with the keyboard opening and closing (desktop:
  the composer grows), 7 a short chat (`?n=1`, nothing to scroll): wheel up, a finger drag down (phone), PageUp and
  Home scroll nothing, then a reply streams past the fold and must still be followed ("not followed" fails the run).
  Each runs still and with a reply streaming.
- `webkit-old` forces `overflow-anchor: none` and `content-visibility: visible`, like an older WKWebView (the Mac app).
- `--nolazy` turns 33q's lazy rows off everywhere, to measure their cost.
- `--strip` saves 8 shots 250 ms apart from the first input on, tiled (needs ffmpeg).
- Readings are taken after paint (a MessageChannel task posted from rAF). In rAF, resize-observer corrections for that
  frame haven't run yet, which shows false jumps.
- Chromium touch goes through raw CDP `Input.dispatchTouchEvent`. `synthesizeScrollGesture` doesn't scroll in the
  headless shell.
- The exit code is the jump total, so `0` means every run held still.
