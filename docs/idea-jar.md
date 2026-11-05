# Idea jar

The idea jar (0.7) is where the person drops an idea they don't want to act on yet. When a bot in
the space is free, it peeks at the oldest idea and writes a **take**: a verdict (`worth_it`,
`maybe` or `not_now`), why, the effort (`S`, `M` or `L`), risks, a plan of at most 6 steps, open
questions, and which bot should do it. The person then chooses what happens next:

- **Do it** turns the plan into a **draft** mission led by the suggested bot, one task per step, in
  order. A draft runs nothing: the person still approves it on the board, like any other plan.
- **Not now** puts the idea on the Later shelf.
- **Edit** saves new text and puts the idea back in the jar for a fresh look.
- **Look now** asks a free bot to peek straight away, in the same order as the jar's own peeking
  (below).

An idea comes in through `POST /api/ideas`, or from the chat composer as `/idea <text>` (any
case). `/idea` saves no chat message and starts no turn, so no bot sees it there. An idea is at
most 2,000 characters long.

Everything in the jar belongs to the person. Every `/api/ideas` route is refused (403) for a bot's
turn token, reads included.

## Statuses

| Status | Meaning |
|---|---|
| `new` | In the jar, waiting for a bot to peek. |
| `peeking` | A bot is writing its take right now. Only one idea per space is peeked at a time. |
| `ready` | The take is written; waiting for the person. |
| `picked` | Do it was pressed: the idea has a `mission_id`. |
| `later` | On the Later shelf. |
| `failed` | No take was written; `error` says why in plain words. |

`new` → `peeking` → `ready`, then `picked` or `later`. Editing sends an idea back to `new`.
Restoring one from the Later shelf sends it back to `ready` if it has a take, otherwise to `new`.
Each change sends an `idea_updated` event to the space, and a finished take sends an `idea_ready`
notification.

## Routes

| Route | What it does |
|---|---|
| `GET /api/ideas?space_id=&status=` | The space's ideas, newest first, with `counts` per status, `daily_cap`, `peeks_today`, `peeks_left` and `cap_reached`. |
| `POST /api/ideas {space_id, text}` | Drops an idea in the jar. 201 with the idea. The space can also be given as `?space_id=` (or `?space=`); the body wins. |
| `GET /api/ideas/:id` | One idea and its takes, newest first. |
| `PATCH /api/ideas/:id {text}` | Edit: new text, back to `new`. 409 `already_peeking` while a bot peeks, `picked` once it is a mission. |
| `DELETE /api/ideas/:id` | Deletes the idea and its takes. A take still running is dropped when it ends. |
| `POST /api/ideas/:id/look` | Look now. 202 with the bot that peeks; 409 `no_free_bot`, `already_peeking` or `picked`. |
| `POST /api/ideas/:id/pick` | Do it. 201 with the draft `mission_id`. 409 unless the idea is `ready`. |
| `POST /api/ideas/:id/later` | Not now: onto the Later shelf, from `new`, `ready` or `failed`. |
| `POST /api/ideas/:id/restore` | Off the Later shelf. |

## Who peeks, and how often

Every 10 minutes kwant looks in each space's jar. It skips a space whose daily cap is used up,
that already has an idea being peeked at, that is archived, or that is paused after a failure
(see below). In the other spaces it takes the oldest `new` idea and gives it to the first free bot.
It asks the bot `@named` in the idea first, then the space's lead, then the rest. A bot is free
when nothing of it is running and its last turn ended at least 10 minutes ago.

**The daily cap** is the `idea_jar_daily_cap` setting (`POST /api/settings`, and in Settings): how
many takes the jar starts on its own per space per UTC day. The default is 3, the most is 10, and 0
turns the jar's own peeking off. Look now is the person asking, so it ignores the cap. It still
needs a free bot, though: in a busy space, such as one where every bot has just reviewed a pull
request in Dev mode, Look now answers `no_free_bot` until one has been idle for 10 minutes.

`GET /api/ideas` shows three numbers:

- `peeks_today`: every take started today, Look now included.
- `peeks_left`: the cap minus today's takes the jar started on its own, never below 0. This is the
  "N of 3 peeks left today" line. It is 0 when the cap is 0, when a take that day went over its
  token cap, or when the count can't be read. So `peeks_left > 0` exactly when the jar will peek
  again today.
- `cap_reached`: `peeks_left == 0`.

A take turn may use at most 30,000 fresh tokens. A turn over that isn't retried, and the jar stops
peeking in that space for the rest of the day. A take turn that runs past 5 minutes is dropped.

## Takes run with no tools

The idea is the person's text, but a take treats it as data, never as instructions. The take is
one hidden turn with `TurnOpts::no_tools`, the same mode as Dev mode's card text (see
[Dev mode](dev-mode.md)): no MCP servers, no browser, and none of the engine's own tools. The
idea is quoted into the prompt with its angle brackets and fences defused, and the reply is saved
in no chat. The reply must be exactly one JSON object with the take's keys, each in range. If it
isn't, the bot gets one retry; after a second bad reply the idea is `failed`. Whatever the reply
says only ever becomes the take's text, and the only thing that can come of it is a draft mission
the person approves.

## When a take fails

A take that fails because of the bot (two bad replies, a timeout, an engine error) leaves the
idea `failed` with the reason. It counts toward the cap like any other take.

A take that fails for kwant's own reasons is not the bot's doing:

- **A usage limit.** When every model is out of usage, the take **fails at once and is not
  parked**: nothing runs again by itself when the limit resets. (A parked retry would be an
  ordinary chat turn with tools on, in the lead's chat.) The idea is `failed` with "Every model was
  out of usage, so no bot could look at it. Use Look now after HH:MM UTC." The person presses Look
  now once the limit has reset.
- **An engine that can't run with every tool off** (any CLI but claude, codex and grok). That bot
  is not run, and the idea is `failed`. Look now tries again with the first free bot.
- **A restart.** A take still running when kwant stops is closed as failed, and its idea goes back
  to `new`, so the jar can peek at it again.

These takes count toward neither the daily cap nor `peeks_today`. Each one pauses the jar's own
peeking in that space for 60 minutes, so an outage or a crash loop can't use up the day's scans.
Look now ignores the pause.

(QA note N1 on #320 found that a take failed by a restart still counted toward the cap, so a crash
loop could burn the day's 3 scans. That was fixed before #320 merged: such a take now counts
toward neither number. `kwants_own_failures_count_toward_neither_the_cap_nor_peeks_today` in
`tests/idea_jar_test.rs` covers it.)

## Exports

Nothing in the jar is exported. A space or bot export holds bots, teams and shared skills only,
so ideas, takes and the `idea_jar_daily_cap` setting stay on this kwant. A take's turn is saved in
no chat, so it doesn't reach an exported bot either. A mission made with Do it is an ordinary
mission once it exists.

The tables (`ideas` and `idea_takes`) are add-only. Setting `idea_jar_daily_cap` to 0 turns the
jar's own peeking off without a rebuild.

## Tests

```sh
cargo test --test idea_jar_test
cargo test --lib -- idea_jar
```
