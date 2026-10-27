# Mission Control + Verified Outcomes (Phase A)

API and data contract of the task board. Field names are stable; changes are additions only.

One task universe: every unit of work (a planned mission task, a team-lead hand-off, a routine run, a
Loose End taken on, a work-ahead run) is a row in `tasks`. Nothing reaches **verified** on a bot's word.

All times are RFC 3339 UTC strings. All ids are UUID strings unless noted. Every list is space-scoped
with `?space=<id>` (default space when omitted), like the rest of the API.

---

## 1. Settings (`GET/POST /api/settings`)

| key | type | default | notes |
|---|---|---|---|
| `mission_board` | bool | `false` | Shows the board, gives bots the mission tools, auto-creates tasks from hand-offs, routine runs, Loose Ends and work-ahead runs, and runs active missions. Off: routes still answer (read and person actions), bots get no task tools, nothing is auto-created, the runner is idle. |
| `mission_lease_minutes` | int 5–240 | `30` | How long a claim lives without activity. |
| `mission_default_budget_tokens` | int 1 000–5 000 000 | `200000` | A mission's budget when none is given. |

Changing settings is person-only (as today). Bots can't change these through `settings_update`.

## 2. States

Task `state` (exactly these strings):

| state | board column | meaning |
|---|---|---|
| `backlog` | Ready (dimmed, "waiting on …") | Mission not approved yet, or a dependency isn't verified. |
| `ready` | Ready | Can be claimed. |
| `working` | Working | A bot holds a live lease (`claim`). |
| `needs_you` | Needs you | The owner has an open question card, approval card or human step (`needs_you` lists them). |
| `review` | Review | Owner says done. Waiting for its checks. `checks_summary.status = "not_checked"` when no check that counts exists and none waits for approval. A check waiting for the person's approval keeps it here and adds a `check_approval` item to `needs_you`. |
| `verified` | Verified | Every required check that counts has passed, none waits for approval, and at least one isn't the owner's own (see §4). Terminal. |
| `blocked` | badge | `blocked_reason` says why (owner/person said so, or `budget`). |
| `failed` | badge | Checks failed `max_attempts` times. The person can retry. |
| `cancelled` | hidden by default | The person cancelled it, or its mission. Terminal. |

Transitions (enforced in Rust and by SQLite triggers; anything else is refused with 409):

| from → to | who |
|---|---|
| backlog → ready | kwant: mission is active and every dependency is `verified` |
| ready → working | a claim: bot `task_claim`, the mission runner, or a hand-off starting. Needs remaining budget. |
| working → review | the owner (`task_update {status:"review"}`), or kwant when a hand-off / routine run / work-ahead run finishes |
| working → ready | the owner releases it, or its lease expires (`attempts` +1) |
| working ↔ needs_you | kwant only: the owner opened / closed a question, approval or human step |
| backlog, ready, working, needs_you → blocked | the owner or the person, with a reason; kwant with `budget` |
| blocked → backlog | the owner or the person (then promoted to ready if it can be) |
| review → verified | kwant only, when the newest result of every required check passed and at least one passing check is not by the owner |
| review → ready | a check failed (`attempts` +1, below `max_attempts`), or the person rejects |
| review → failed | a check failed and `attempts` reached `max_attempts` (default 3) |
| failed → ready | the person retries |
| any non-terminal → cancelled | the person |

Mission `status`: `draft` (proposed by a bot, waiting for the person's approval) → `active` → `done`
(every task is verified or cancelled, at least one verified), or `cancelled`. `over_budget` is a flag,
not a status.

## 3. Tables

```
missions(id, space_id, title, goal, status, lead_kwant_id, channel_id?, direct_kwant_id?, thread_id?,
         workspace_path (server only, not in the API), budget_tokens, spent_tokens, created_by, approved_at?, approved_by?,
         created_at, updated_at, finished_at?)
tasks(id, space_id, mission_id?, title, acceptance, state, priority, owner_kwant_id?,
      suggested_kwant_id?, budget_tokens, spent_tokens, attempts, max_attempts, blocked_reason?,
      source_kind, source_id?, channel_id?, direct_kwant_id?, thread_id?, created_by,
      created_at, updated_at, finished_at?)
task_dependencies(task_id, depends_on_id)              -- same mission, no cycles
task_events(id INTEGER, task_id, mission_id?, kind, actor, from_state?, to_state?, detail JSON,
            created_at)                                -- append-only: UPDATE/DELETE abort
agent_claims(id, task_id, kwant_id, turn_id?, claimed_at, expires_at, released_at?, release_reason?)
evidence(id, task_id, kind, label, uri?, path?, sha256?, bytes?, excerpt?, captured_by, run_id?,
         captured_at)
outcome_checks(id, task_id, kind, spec JSON, required, approved, status, output_excerpt?, checked_by?,
               requested_by, approved_by?, evidence_id?, created_at, started_at?, finished_at?)
```

- `priority`: `0` low, `1` normal (default), `2` high, `3` urgent.
- `source_kind`: `mission` | `person` | `delegation` | `routine` | `zombie` | `work_ahead` | `chat`.
  `source_id` is the delegation id, routine run id, Loose End case id or work-ahead run id.
- `actor` / `created_by` / `captured_by` / `checked_by` / `requested_by`: `"person"`, `"kwant"`
  (kwant itself ran it) or a bot id.
- Only one live claim per bot at a time, and per task. A claim is live when `released_at` is null and
  `expires_at` is in the future. Any task tool call by the owner, and every turn of the owner, renews it.

## 4. Outcome checks

`kind` and `spec`:

| kind | spec | passes when | who may add it |
|---|---|---|---|
| `command` | `{"command": "cargo test -q", "timeout_secs": 300}` | exit code 0. Runs in the mission's folder with `bash -c`. | The person, or a bot in `mission_plan` (approved with the mission). A bot adding one later gets `approved:false`, and it runs only after the person approves it. |
| `http` | `{"url": "https://…", "expect_status": 200, "contains": "optional text"}` | GET returns the status (default any 2xx) and the text if given | Anyone. A URL on this computer or a private network needs the person's approval (same rule as WebFetch). |
| `file_exists` | `{"path": "dist/app.dmg"}` | the file exists inside the mission's folder | Anyone (the task's own bot: see below) |
| `file_contains` | `{"path": "CHANGELOG.md", "text": "0.1.4"}` | the file contains the text | Anyone |
| `file_sha256` | `{"path": "out.zip", "sha256": "…"}` | the hash matches | Anyone |
| `peer_review` | `{"note": "…", "evidence_ids": ["…"]}` | a bot that is **not the owner** gives `pass` and cites at least one evidence item | Recorded by `task_review` |
| `person` | `{"note": "…"}` | the person pressed Verify | `POST /api/tasks/:id/verify` |

- Paths are relative to the mission's folder (the team's workspace when the mission came from a team
  chat, else the lead bot's folder). `..`, absolute paths outside it, symlinks out of it and
  password/key files are refused.
- `status`: `pending` | `needs_approval` | `running` | `passed` | `failed` | `error` (couldn't run, e.g.
  a timeout; counts as not passed). `output_excerpt` is at most 2 000 chars, with secrets left as the
  command printed them, so commands that print secrets are the person's responsibility.
- kwant runs `command`, `http` and `file_*` checks itself (`checked_by: "kwant"`) and stores the output
  as evidence (`kind: "check_output"`).
- `approved_by`: who let the check run and count. `"person"` (the person added it, approved it, or
  made the mission), `"mission"` (part of a bot's `mission_plan` that the person approved), `"auto"` (a
  safe check a bot other than the owner added), or `null` while `status = "needs_approval"`.
- **The owner's own checks.** A check the task's owner (or suggested owner) adds with
  `task_request_check` always starts as `approved: false`, `status: "needs_approval"`, even a
  `file_*` check, so a bot can't verify itself with "the file I just wrote exists". It runs and counts
  only after the person approves it (`approved_by: "person"`). A check whose requester later became the
  owner counts only if `approved_by` is `"person"` or `"mission"`.
- `counts` (on each Check in the API): whether this check can count toward Verified under those rules.
- `required` (default `true`): **every** required check must count and its newest result must be
  `passed`. A required check that is `failed`/`error` sends the task back (`ready`, or `failed` after
  `max_attempts`). A required check that is `pending`/`running`, or waits for approval, keeps the task
  in `review`; one waiting for approval also shows as a `check_approval` item in the task's
  `needs_you` and counts in Insights `needs_you_now`. The person's own Verify (a passed `person`
  check) doesn't wait for unapproved checks, but approved checks still have to pass. Optional checks
  (`required: false`) are shown but never hold, fail or verify a task. An optional check waiting for
  approval is never a `needs_you` item and never counts in `needs_you_now`; it can still be approved,
  and `checks_summary.needs_approval` counts it.
- A task with no checks stays in `review` ("not checked") until a peer review or the person verifies it.

`checks_summary` on each task: `{"status": "not_checked" | "pending" | "passed" | "failed",
"passed": n, "failed": n, "needs_approval": n, "required_needs_approval": n, "total": n}`.
`passed`/`failed` count checks that count; `needs_approval` counts checks waiting for the person, and
`required_needs_approval` how many of those are required (so hold the task in `review`). A card shows
"Needs you" only when `required_needs_approval > 0`; `status: "pending"` with
`required_needs_approval: 0` means a required check is still running, not that the person is needed. `status` is `passed` only when the task
could be verified on them, and `pending` while a required check waits for approval (an optional
one waiting is counted in `needs_approval` but doesn't hold the task).

## 5. Bot tools (MCP self tools; only while `mission_board` is on)

| tool | args | result / rules |
|---|---|---|
| `mission_plan` | `{title, goal, budget_tokens?, tasks: [{key, title, acceptance, owner?, priority?, depends_on?: [key], budget_tokens?, checks?: [{kind, spec, required?}]}]}` | Creates a **draft** mission led by the caller, with up to 30 tasks in `backlog`. The person approves it once (card in chat + board). Nothing runs until then. Owners are bots in the same space. |
| `task_list` | `{mission_id?, state?, mine?}` | Read-only. Tasks in the caller's space. |
| `task_claim` | `{task_id}` | Atomic: `ready` → `working` with a lease. Refused if someone else holds it, a dependency isn't verified, the mission isn't active, the remaining budget can't cover the task's budget, or the caller already holds another live claim. |
| `task_update` | `{task_id, status?: "review" \| "ready" \| "blocked", note?, reason?}` | Owner only, with a live claim. `review` = I'm done (releases the lease and runs the task's checks now). `ready` = release. `blocked` needs `reason`. A `note` alone renews the lease and is logged. Can never set `verified`. |
| `task_attach_evidence` | `{task_id, kind: "file" \| "url" \| "note" \| "screenshot", label, path?, url?, text?}` | Records evidence. For files kwant reads the file itself and stores `sha256` and `bytes` (the bot's own hash is ignored). Evidence alone never verifies anything. |
| `task_request_check` | `{task_id, check_id?, kind?, spec?, required?}` | Runs the task's approved checks (or one), or adds a new check and runs it if it may run. Returns each result. |
| `task_review` | `{task_id, verdict: "pass" \| "fail", evidence_ids: [..], note}` | A teammate's review. Refused for the owner. `pass` needs at least one evidence id on that task. |

The self-tools approver treats `task_list` as read-only. All other tools change only the task board,
never files, messages or money, so they don't ask for a card. Turns keep their normal approval mode.
Sending, paying, deleting, publishing and sign-ins still ask as today.

## 6. HTTP routes

Reads (`GET`) are open like the rest of the app API. Every non-GET route below is **person-only**
(`auth.rs`), so bot tokens get 403.

```
GET    /api/missions?space=&status=           -> {missions: [Mission]}
POST   /api/missions                           -> Mission       (person creates; active at once)
GET    /api/missions/insights?space=           -> Insights
GET    /api/missions/:id                       -> MissionDetail
PATCH  /api/missions/:id   {title?, goal?, budget_tokens?}
POST   /api/missions/:id/approve               -> Mission       (draft -> active)
POST   /api/missions/:id/cancel                -> Mission
POST   /api/missions/:id/run  {max_tasks?: 1-5} -> {started: [{task_id, kwant_id}], reason?}   (test hook / "Run now")

GET    /api/tasks?space=&state=&mission_id=&source_kind=&limit=   -> {tasks: [Task]}
POST   /api/tasks          {title, acceptance?, mission_id?, owner_kwant_id?, priority?, depends_on?, budget_tokens?,
                            checks?, source_kind?: "person"|"zombie", source_id?}  -> Task
GET    /api/tasks/:id                          -> TaskDetail
PATCH  /api/tasks/:id      {title?, acceptance?, priority?, owner_kwant_id?, budget_tokens?}
POST   /api/tasks/:id/verify  {note?}          -> Task  (review -> verified, a `person` check)
POST   /api/tasks/:id/reject  {reason}         -> Task  (review -> ready)
POST   /api/tasks/:id/block   {reason}         -> Task
POST   /api/tasks/:id/unblock                  -> Task
POST   /api/tasks/:id/retry                    -> Task  (failed -> ready)
POST   /api/tasks/:id/cancel                   -> Task
POST   /api/tasks/:id/checks  {kind, spec, required?}  -> Check   (approved, runs if the task is in review)
POST   /api/tasks/:id/checks/run               -> {checks: [Check], task: Task}
POST   /api/checks/:id/approve                 -> Check   (needs_approval -> runs)
                                                    (runs in review or verified; on a verified task the
                                                     result is recorded and the state never changes)
```

Errors: `{"error": "plain sentence"}` with 400 (bad input), 404, 409 (illegal transition, budget,
claim conflict), 403 (bot token on a person route).

### Shapes

```jsonc
// Mission
{"id","space_id","title","goal","status","lead_kwant_id","lead_name",
 "channel_id","direct_kwant_id","thread_id",
 "budget_tokens","spent_tokens","reserved_tokens","over_budget": false,
 // no workspace_path: the folder stays on the server (checks and evidence paths are relative to it)
 "created_by","approved_at","approved_by","created_at","updated_at","finished_at",
 "counts": {"backlog":0,"ready":0,"working":0,"needs_you":0,"review":0,"verified":0,"blocked":0,"failed":0,"cancelled":0}}

// Task
{"id","space_id","mission_id","mission_title","title","acceptance","state","priority",
 "owner_kwant_id","owner_name","suggested_kwant_id","budget_tokens","spent_tokens",
 "attempts","max_attempts","blocked_reason","source_kind","source_id",
 "channel_id","direct_kwant_id","thread_id","created_by","created_at","updated_at","finished_at",
 "depends_on": ["task id"], "waiting_on": ["task id not verified yet"],
 "claim": null | {"kwant_id","claimed_at","expires_at"},
 "needs_you": [{"kind": "question"|"approval"|"human_step"|"check_approval", "id", "text"}],
 "checks_summary": {"status","passed","failed","needs_approval","required_needs_approval","total"}}
// needs_you: question/approval/human_step while working or needs_you; check_approval (id = the
// check's id, approve it with POST /api/checks/:id/approve) while in review, for required checks only

// TaskDetail = {"task": Task, "checks": [Check], "evidence": [Evidence], "events": [Event]}   (events oldest first, max 500)
// MissionDetail = {"mission": Mission, "tasks": [Task]}

// Check
{"id","task_id","kind","spec","required","approved","approved_by","counts","status","output_excerpt",
 "checked_by","checked_by_name","requested_by","evidence_id","created_at","started_at","finished_at"}

// Evidence
{"id","task_id","kind","label","uri","path","sha256","bytes","excerpt","captured_by","run_id","captured_at"}
// kind: "file" | "url" | "note" | "screenshot" | "check_output"

// Event
{"id": 42, "task_id","mission_id","kind","actor","actor_name","from_state","to_state","detail": {}, "created_at"}
// kind: created | state | claimed | released | lease_expired | note | evidence | check_added |
//       check_started | check_result | review | budget | edited

// Insights (counts, last 7 days unless noted)
{"verified_7d": 0, "false_success_7d": 0, "needs_you_now": 0, "blocked_now": 0, "blocked_minutes_7d": 0,
 "interventions_7d": 0, "tokens_7d": 0, "tokens_per_verified_7d": null}
```

`false_success_7d` counts tasks whose owner moved them to `review` and whose next check result was
`failed`. `interventions_7d` counts person actions on tasks (verify, reject, block, unblock, retry,
cancel, check approval).

## 7. Live events (SSE `/api/events`)

```
{"type": "task_updated",    "space_id": "…", "data": {"task": Task}}
{"type": "mission_updated", "space_id": "…", "data": {"mission": Mission}}
```

Sent on every change. The board can patch its list from `data` or refetch. The chat also gets a short
kwant line (`meta.kind = "mission"`) in the mission's chat when a mission is proposed, approved or done,
and when a task is verified or failed.

## 8. Where tasks come from (one task universe, only while `mission_board` is on)

| source | created when | state |
|---|---|---|
| `mission` | `mission_plan` / `POST /api/missions` | backlog → ready after approval |
| `person` | `POST /api/tasks` | ready (backlog if it has unverified deps) |
| `delegation` | a team lead hands work on (`delegate` tool or @mention in a team chat) | working (kwant holds the specialist's lease) → review when the hand-off completes, failed if it fails, cancelled if cancelled. Inside a mission task, it joins that mission. |
| `routine` | a routine run starts | working → review when it completes, failed if it fails |
| `zombie` | the person takes a Loose End on (`POST /api/tasks {source_kind:"zombie", source_id}`), or kwant starts its owner turn | ready, or working while the owner turn runs |
| `work_ahead` | a work-ahead run posts its message | review |

"Needs you" reuses the existing question, approval and human-step cards. The board links to them. It
doesn't create its own cards.

## 9. Mission runner

While `mission_board` is on, every scheduler tick (and `POST /api/missions/:id/run`) looks at active
missions. Each mission gets at most one runner turn at a time, server-wide at most 2. The runner picks
the highest-priority `ready` task, claims it for its owner (or suggested owner, else the lead) and runs
one hidden turn in the mission's chat with the task, its acceptance criteria and checks. At the end
of every turn its fresh tokens go to the task and the mission: the task the bot holds, or the one it
claimed or let go of during that turn (`task_update review` releases the claim before the turn ends,
and the turn still pays for itself). When the turn ends and the task is still `working`, the
lease is released (`ready`, attempt +1).

Budget: a claim needs `spent + reserved + task budget ≤ mission budget`, where `reserved` is what
working tasks may still spend. After every turn the tokens are added. A task over its own budget is
blocked (`budget`). A mission at or over its budget gets `over_budget: true`, and no new claims start
until the person raises the budget. One turn already running can overshoot by what that turn uses,
because kwant counts tokens when a model call returns.

## 10. Implementation notes

- `agent_claims` also has `context_id` (the chat the claim's turns run in; spend goes to the claim
  whose context matches the turn) and `system` (1 = kwant holds it for a hand-off or routine run; it
  doesn't count as the bot's own live claim, so the bot can still claim its own task).
- `working`/`needs_you` → `failed`: kwant, when a hand-off or routine run fails. `working`/`needs_you`
  → `cancelled`: also kwant, when a hand-off is cancelled.
- Event kind `released` is used when a runner turn ends with the task still `working` (`attempts` +1).
  If that was the last attempt, kwant then moves it `ready` → `blocked`.
- `POST /api/tasks/:id/retry` resets `attempts` to 0.
- `POST /api/missions` also takes `tasks` (same shape as `mission_plan` tasks) and `lead_kwant_id`,
  `channel_id`, `thread_id`. A command check in a person-created mission is approved at once.
- Not in Phase A: a plain @mention hand-off creates a task only when it goes through the delegation
  machinery; a Loose End's own owner turn doesn't create a `zombie` task (the person taking it on does).

## 11. Work graph (Phase B1)

Design and decisions: `/home/maus/shared/ship-software-safely/mission-control/phase-b/contract.md` §2
and `decisions.md` (D1–D3). A verified task stays final. When its inputs may have changed, kwant
**flags** it as "may be invalid", and the person decides. A flag never changes any task's state.

### Tables

- `tasks.forked_from TEXT`: the task this one redoes (a fork), else NULL.
- `task_flags`:
  - `id`, `task_id`, `kind` (`may_be_invalid`)
  - `cause`: one of `evidence_changed`, `upstream_evidence_changed`, `upstream_forked`
  - `cause_task_id`: the upstream task, or the task itself for `evidence_changed`
  - `detail` JSON, `created_at`
  - `resolved_at`, `resolved_by`, and `resolution`: one of `still_valid`, `redo`, `cancelled`
- Indexes and triggers on `task_flags`:
  - The unique index `task_flags_one_open (task_id, cause_task_id, cause) WHERE resolved_at IS NULL` allows at most one open flag per cause.
  - `task_flags_resolve_once`: a resolved flag can't change.
  - `task_flags_person_resolves`: only `resolved_by = 'person'` resolves a flag.
  - `task_flags_no_delete`: flags are never deleted.
- `idempotency_keys (key, route, response, created_at)` keeps each key for 24 h.
- `graph_scans (mission_id, scanned_at, skipped)` records each mission's last drift scan. It feeds the graph's `scanned_at`/`scan_skipped` and the 10-minute throttle.

### When a flag is raised

1. **Evidence drift.** The scan looks at each verified task's `file` evidence and uses the latest row per path.
   - If the file now hashes differently or is gone, the task gets `evidence_changed` with detail `{path, old_sha256, new_sha256|null}`.
   - Each **direct** dependent that has started gets `upstream_evidence_changed`. Started means the state is past `ready`, or `attempts > 0`, or the task was ever claimed. Cancelled dependents are left out.
2. **Fork of a verified task.** Each started direct dependent of the original gets `upstream_forked`, with detail `{fork_id, reason}`.

Things that are never flagged:
- Backlog or ready dependents, and anything further down the graph (D2). The graph shows how many tasks sit further down as `downstream`.
- A drift that a flag already recorded: same task, cause task, cause, path and new hash. Once the person calls a drift `still_valid`, the same file contents don't flag it again. A new change does.

Each flag:
- writes a `flagged` event (actor `kwant`, no state change);
- posts a line in the mission's chat (or the task's own) as the owner;
- adds a `{"kind": "may_be_invalid", "id": <flag id>, "text": "Check this still holds: …"}` item to `needs_you` in **every** state. Insights `needs_you_now` counts flagged tasks.

Bots have no tool to resolve a flag (D1).

### The drift scan

- The runner tick scans each active mission at most every 10 minutes. `POST /api/missions/:id/graph/scan` also runs it on demand for any mission that isn't cancelled, which matters because a mission whose tasks are all verified is `done`.
- It hashes at most 200 files and skips files over 50 MB.
- Paths go through the same `inside(folder, rel)` guard as evidence capture: no `..`, nothing outside the task folder, no secret or env files.
- A skipped file is reported with a reason and never counts as changed.

### Resolving (person only)

`POST /api/tasks/:id/flags/:flag_id/resolve` with body `{"resolution", "note"?}` writes `flag_resolved` (actor `person`).

| resolution | effect |
|---|---|
| `still_valid` | Closes the flag. The task is unchanged. |
| `redo` | **Verified:** forks the task and returns the fork. **review/failed:** moves it to `ready` (attempts reset). **blocked:** moves it to `backlog`. **backlog/ready:** just closes the flag. **working/needs_you:** 409 "it's running; resolve when it stops". **cancelled:** 409. |
| `cancelled` | A verified task gets 409. A cancelled task just has the flag closed. Any other task is cancelled. |

Other responses:
- 409 when the flag is already resolved. A Verified task's redo claims the flag in the same transaction as the fork, so of several simultaneous redos (with or without a key) one forks and the rest get 409 with no copy.
- 404 when the flag belongs to another task.
- Success returns `{flag, task, fork|null}`.

### Fork (person only)

`POST /api/tasks/:id/fork` with body `{"reason", "from_checkpoint"?}`.
- `reason` is required, at most 500 characters.
- `from_checkpoint` gets 400 until Phase B2.
- The `Idempotency-Key` header is optional.

The new `backlog` task copies these from the original:
- `mission_id`, `title`, `acceptance`, `priority`, `budget_tokens`, `max_attempts`, `source_kind` and the chat;
- its dependencies;
- its checks, reset to pending with results cleared. An approval carries over only when `approved_by` is `person` or `mission`. Any other check waits for approval again (`needs_approval`).

Other fields on the new task:
- `suggested_kwant_id` is the original's owner.
- `source_id` is NULL, `created_by` is `person`, and `forked_from` is the original's id.

What else happens:
- The original is untouched. Its dependents are not rewired (D3).
- `promote_one` runs on the fork.
- Events: `forked` on the original (`{fork_id, reason}`) and `created` on the fork (`{forked_from}`).
- A `done` mission opens again (`active`) so the fork can run. A cancelled mission refuses with 409.

### Idempotency-Key

The header works on `POST /api/tasks/:id/fork`, `POST /api/tasks/:id/retry` and `POST /api/tasks/:id/flags/:flag_id/resolve` (so two Redo clicks from two tabs make one fork; the route includes the flag id). Keys are 1–200 characters.
- The same key on the same route (method plus path, task id included) returns the stored response with `idempotent-replayed: true`, and nothing runs again.
- The same key on another route gets 422.
- Only successful responses are stored. Keyed requests run one at a time.

### Graph routes

- `GET /api/missions/:id/graph` returns `{mission_id, nodes, edges, scanned_at, scan_skipped}`.
  - Each node: `{id, title, state, owner_kwant_id, owner_name, attempts, max_attempts, forked_from, downstream, flags (open), checks_summary}`.
  - Each edge: `{from: depends_on_id, to: task_id}`.
- `GET /api/graph` is the active space's tasks without a mission: open ones, plus ones finished in the last 7 days. `mission_id` is null there, and so is `scanned_at`.
- `GET /api/tasks/:id/flags` lists open flags; `?all=1` adds resolved ones. Flag shape: `{id, kind, cause, cause_task_id, cause_title, detail, created_at, resolved_at, resolution}`.
- `POST /api/missions/:id/graph/scan` (person only) returns `{scanned, flagged: [flag ids], skipped: [{task_id, path, why}]}`.
- `task_json` gains `forked_from` and `flags` (open only).
- There are no new SSE types: flags and forks go out as `task_updated` and `mission_updated`.

## 12. Checkpoints and restore (Phase B2)

Design and decisions: `/home/maus/shared/ship-software-safely/mission-control/phase-b/contract.md` §3
and `decisions.md` (D4, D6, D8, D9). A checkpoint is a snapshot of a task's folder (the mission's
`workspace_path`, else the owner bot's workspace). A restore writes back only the paths this task's
own runs changed. What a tool did outside the folder (Bash, web, email, other bots) is never undone:
it is listed as "not undone".

### When a checkpoint is taken

- `before_run` when a task is claimed (runner, `task_claim`, a team-lead hand-off), and `after_run` when the claim ends (the task leaves `working`/`needs_you`).
- `manual`: `POST /api/tasks/:id/checkpoints` (person only).
- `before_restore`: always, right before a restore writes anything. If it can't be taken, nothing is restored (500, `code: checkpoint_failed`).
- The setting `checkpoints` (default on) turns off only `before_run`/`after_run`. Manual and safety checkpoints are always taken.
- A failed snapshot (for example an unreadable folder) never stops the turn. It is stored as a checkpoint with `error` set and `restorable: false`, plus a `checkpoint_failed` task event.

### The store

- Under `<data_dir>/checkpoints/` (folder mode 0700, files 0600): `blobs/<sha256>` is deduplicated content, `<task_id>/<checkpoint_id>.json` is each manifest, and `<task_id>/runs.jsonl` holds the paths each run changed.
- Snapshots include `.env`, keys and other secrets, which is why no route ever returns a manifest path, a blob path or file contents.
- Never snapshotted (counted in `skipped`; the manifest keeps each path and why): symlinks (never followed), `.git`, `node_modules`, `target`, `.venv`, files over 20 MB, and anything past 20,000 files. Past the per-checkpoint cap, the largest files are skipped first.
- Caps (settings): `checkpoint_max_mb` (10–2000, default 200) per checkpoint; `checkpoint_keep` (1–20, default 5) per task, plus the task's first `before_run`, which is always kept. All checkpoints together are kept under 2 GB, oldest first. Pruning writes a `checkpoint_pruned` event. Blobs no manifest uses are removed after every checkpoint.

### Tables

- `task_checkpoints`: `id`, `task_id`, `mission_id`, `kind` (`before_run`, `after_run`, `manual`, `before_restore`), `claim_id`, `turn_id`, `attempt`, `created_at`, `created_by`, `files`, `bytes`, `skipped` (a count; the paths and reasons stay in the manifest), `git_head`, `error`. Server-only columns `manifest_path`, `manifest_sha256`, `folder` are never serialized. Rows are never updated; pruning deletes them.
- `turn_tool_calls` (the ledger; append-only, no UPDATE or DELETE): `turn_id`, `kwant_id`, `task_id`, `tool_name`, `effect` (`none`, `workspace`, `external`), `summary`, `decision`, `approval_id`, `created_at`, and server-only `path` (for a `workspace` file call: the file it names, relative to the task folder; D11).
  - Only task turns are logged (D9): a call is logged when the bot holds a task in that turn.
  - Every call that reaches kwant's permission check is logged, including the ones allowed without a card and the ones refused.
  - `none`: read-only tools (Read, Glob, Grep, LS, WebSearch, TodoWrite…) and the `task_*`/`mission_plan` self tools. `workspace`: Write, Edit, MultiEdit, NotebookEdit inside the task folder. Everything else is `external`, Bash always (D6).
  - `summary` is at most 300 characters, never a file body or message text, with credentials masked.

### Routes

- `GET /api/tasks/:id/checkpoints`: oldest first. Each item: `{id, kind, attempt, created_at, created_by, files, bytes, skipped, git_head, error, restorable, effects_after: {workspace: n, external: [{tool_name, summary, at}]}}`. `effects_after` covers the calls between this checkpoint and the next.
- `POST /api/tasks/:id/checkpoints` (person only): a manual checkpoint; 201 with the item, 500 if the snapshot failed.
- `GET /api/tasks/:id/checkpoints/:cid/restore-preview`: `{checkpoint_id, preview_sha256, changes: [{path, action: overwrite|create|delete}], conflicts: [{path, changed_by_task}], skipped: [{path, why}], external_not_undone: [{tool_name, summary, at}], not_undone_note, effects_known, task_state_after, refused: null|fork_instead|running}`.
  - Only paths this task's runs changed are restored. A path another task's run changed after the checkpoint is a conflict, left alone unless forced.
  - Conflict attribution (D11): `changed_by_task` names a task only when that task's own logged file calls (Write/Edit/MultiEdit/NotebookEdit, `turn_tool_calls.path`) wrote the path during a run after the checkpoint. A path that merely changed while another task ran in the same folder (a shell command, or anyone) is `{path, changed_by_task: null, why: "changed outside this task"}`, unless this task's own calls wrote it and it is still as this task's last run left it.
  - Every `force_paths` entry must be listed by the preview (in `changes` or `conflicts`); otherwise 422 `unknown_force_path` naming the paths, and nothing is written.
  - `effects_known` is false when a run after the checkpoint changed files but no tool call that could have was logged.
  - 409 `not_restorable` when the snapshot failed or was pruned.
- `POST /api/tasks/:id/checkpoints/:cid/restore` (person only, `Idempotency-Key` required): body `{preview_sha256, force_paths?: [path], rerun?: bool}`. Returns `{restored, deleted, failed: [{path, why}], before_restore_checkpoint_id, external_not_undone, not_undone_note, effects_known, task}`.
  - 409 `fork_instead` for a verified task, `running` while a claim is live, `stale_preview` when the folder or task changed since the preview.
  - The task moves review/failed → ready and blocked → backlog; other states stay. A `restored` task event records the counts.
  - A restore never re-runs a task by itself (D10). When the task ends up ready or backlog it is held (`held_at` set, `held` task event): neither the runner nor a bot's `task_claim` takes it until the person calls `POST /api/tasks/:id/run` (person only; 409 `not_held` when it isn't held; `released` task event). `rerun: true` ("restore and run again") leaves it unheld, or lets go of an earlier hold.
  - Never writes through a symlink or outside the folder.
- `POST /api/tasks/:id/fork` accepts `from_checkpoint`: the original's task-owned paths are restored to that checkpoint first (after a `before_restore`), then the fork is made. The response adds `restore`.
- `task_json` gains `checkpoint_count`, `last_checkpoint_at` and `held_at` (null unless a restore holds the task).

## 13. AutoFix (Phase B3)

Design and decisions: `/home/maus/shared/ship-software-safely/mission-control/phase-b/contract.md` §4–§5
and `decisions.md` (D5, D7, D12). Every failure of a task is recorded with a class. With the setting
`autofix` on, AutoFix may retry it a bounded number of times. A permanent change (a check, the
acceptance, a routine) is only ever proposed; it applies when the person approves it.

### Settings

- `autofix`: `off` (default), `suggest` or `on`. `off` records failures only (Phase A behaviour: a failed check sends the task back to `ready`). `suggest` plans one attempt and waits for the person (`autofix_suggested` Needs-you item). `on` runs the attempt.
- `recovery_budget_pct` (0–100, default 25): the share of the task's budget that AutoFix retries may spend.

### Failures

- `task_failures` (append-only): `id`, `task_id`, `attempt`, `source` (`check`, `turn`, `lease`, `budget`, `routine`, `handoff`), `class`, `summary`, `excerpt`, `checkpoint_id` (the last good `after_run`), `created_at`, and server-only `diagnostics_path`.
- `classify(source, status, text)` is deterministic. A check that failed is `check_failed`, or `bad_spec` when its command can't run (exit 127, "command not found"). A check that errored is `transient` for a timeout or 429/5xx, else `bad_spec`. A lease that ran out or a turn that ended without a review is `stalled`; over budget is `budget`; a denied action is `denied`; a task waiting on the person is `needs_input`. Other errors are `auth` (not logged in, 401, expired token, credit too low), then `transient` (rate limit, overloaded, timeouts, connection errors, 429/500/502/503/504/529 as whole numbers), else `unknown`.
- The text is redacted (credentials in URLs, `sk-`/`xai-`/`ghp_`… prefixes, long digit-bearing tokens) before it is stored. The excerpt is at most 2,000 characters. The full redacted output goes to `<data_dir>/autofix/<task_id>/<uuid>.log` (folder 0700, file 0600, 256 KB). No route returns that path.

### Attempts

- `autofix_attempts`: `id`, `task_id`, `failure_id`, `strategy`, `status` (`suggested`, `scheduled`, `running`, `succeeded`, `failed`, `skipped`), `not_before`, `reason`, `retry_task_id`, `tokens_spent`, `created_at`, `finished_at`, and server-only `idempotency_key` (unique: sha256 of task, failure and strategy, so a failure is never planned twice). At most one `scheduled`/`running` attempt per task (`autofix_one_live`), and a trigger refuses one past the task's `max_attempts`.
- Strategies: `transient` → `wait_and_retry` after 1, 5, then 25 minutes; `stalled` → `retry_from_checkpoint` (the folder is as the run left it) or `retry_same`; `check_failed` → `retry_with_diagnostics`; `bad_spec` → `propose_change` (a `check_spec` proposal making the check optional; the attempt stays `suggested` and never runs); `unknown` → `retry_same`, once per task. `auth`, `budget` and `denied` are never retried: a `skipped` attempt with what to do and a `fix_needed` Needs-you item. `needs_input` and hand-off tasks get no attempt.
- Limits, checked when planning and again when starting: a `blocked` task is never moved (`not_retryable`); no attempts left (`no_attempts_left`); `recovery_spent_tokens` plus the average spend per attempt over the recovery budget (`over_recovery_budget`). A limit makes the attempt `skipped` with that reason.
- While attempts are left and AutoFix isn't off, a failure parks the task in `failed` instead of `ready` (no "failed" chat line). The retry moves it `failed → ready` as `Actor::AutoFix`, the only move that actor may make, with a task event of kind `autofix`. A routine run is retried as a new routine run; its task gets `forked_from` = the failed task and counts on from its attempts, and the failed task stays `failed`.
- Recovery spend: a claim taken while an attempt runs carries its `autofix_id`, and that claim's spend (at hand-in and at turn end) also adds to `tasks.recovery_spent_tokens` and the attempt's `tokens_spent`. Plain runs never count as recovery.
- The retry prompt (runner and `task_claim`) says which attempt it is and why, the failing check and its redacted output, `KWANT_IDEMPOTENCY_KEY=<key>`, and the external tool calls earlier attempts already made. Calls in an attempt's turns record the key in `turn_tool_calls.idempotency_key`.
- An attempt ends `succeeded` when its task is verified, `skipped` when it is cancelled, `failed` at the next failure.
- Scheduled attempts live in the table; the scheduler tick runs the due ones, so a restart loses nothing.

### Workflow changes (person only)

- `workflow_changes`: `id`, `task_id`, `routine_id`, `target` (`check_spec`, `task_acceptance`, `max_attempts`, `routine_instructions`, `routine_schedule`), `before`, `after`, `reason`, `proposed_by`, `status` (`proposed`, `approved`, `rejected`, `applied`, `stale`), `decided_by`, `decided_at`, `created_at`. Triggers: only the person decides; `applied` only after `approved`.
- Bots propose with the `workflow_propose` tool; kwant reads `before` itself. Approving applies the change once, only while the target still holds `before`; otherwise it goes `stale` and nothing is written.

### Routes

- `GET /api/tasks/:id/failures`: oldest first, each `{id, attempt, source, class, summary, excerpt, checkpoint_id, created_at, autofix: attempt|null}`.
- `POST /api/tasks/:id/autofix` (person only, `Idempotency-Key` accepted), body `{failure_id?}`: runs the suggested or scheduled attempt now, or plans one. 409 with `code` `not_retryable`, `no_attempts_left` or `over_recovery_budget`.
- `POST /api/tasks/:id/autofix/:aid/cancel` (person only): a `suggested`/`scheduled` attempt becomes `skipped`.
- `GET /api/workflow-changes?status=`, `POST /api/workflow-changes/:id/approve`, `POST /api/workflow-changes/:id/reject`: all person only (`/api/workflow-changes` is a person-only segment).
- `task_json` gains `recovery_spent_tokens`, `recovery_budget_tokens`, `last_failure: {class, summary, created_at}|null` and `autofix: attempt|null`. Needs-you kinds: `autofix_suggested`, `fix_needed`, `workflow_change`.
- Insights gain `autofix_7d: {attempts, succeeded, tokens}` and `failures_by_class_7d`, for the space.
- Task events: `failure`, `autofix_scheduled`, `autofix_started`, `autofix_finished`, `workflow_change_proposed`, `workflow_change_decided`, `workflow_change_applied`.
