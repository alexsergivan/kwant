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
| `mission_default_budget_tokens` | int 0 or 1 000–5 000 000 | `0` | A new mission's budget when none is given. 0 = no limit. |
| `mission_default_task_budget_tokens` | int 0–5 000 000 | `0` | A new task's budget when none is given (capped at a limited mission's budget). 0 = no limit. |

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
                           (`owner_kwant_id` on POST and PATCH is who should do it: it is stored as
                            `suggested_kwant_id`; the task's `owner_kwant_id` stays empty until that bot claims it)
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
| `routine` | a routine run starts | working → review when it completes, failed if it fails; with the routine's class, acceptance and checks (§20), verified when they pass |
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

Budget: `budget_tokens` 0 means no limit, on missions and tasks alike, and is what new ones get
unless a budget or a Settings default is given (§15). A mission with no limit never refuses a claim.
In a limited mission a claim needs `spent + reserved + task budget ≤ mission budget`, where `reserved`
is what working tasks with a limit may still spend; a task with no limit reserves nothing and needs
only one token left. After every turn the tokens are added. A task over its own budget is
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
and `decisions.md` (D4, D6, D8, D9). A checkpoint is a snapshot of a task's folder (see "The task folder" below). A restore writes back only the paths this task's
own runs changed. What a tool did outside the folder (Bash, web, email, other bots) is never undone:
it is listed as "not undone".

**The task folder** (`missions::task_folder`) is where a task's file checks, evidence, checkpoints and
audit turns look: the mission's `workspace_path`; else, for a task whose thread belongs to a team (a
hand-off made in a team's channel or thread), the team's folder, where that hand-off's turns run;
else the owner bot's own workspace. Before 2026-10-30 a team hand-off card without a mission fell back
to the owner's own folder, so its checks and audit didn't see the files its owner wrote in the team
folder. A hand-off from a direct chat (no team) still uses the owner's own folder.

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
  - A restore never re-runs a task by itself (D10). When the task ends up ready or backlog it is held (`held_at` set, `held` task event): neither the runner nor a bot's `task_claim` takes it until the person calls `POST /api/tasks/:id/run` (person only; `Idempotency-Key` accepted, and the same key replays the first 200; 409 `not_held` when it isn't held; `released` task event). `rerun: true` ("restore and run again") leaves it unheld, or lets go of an earlier hold.
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
- `classify(source, status, text)` is deterministic. A check that failed is `check_failed`, or `bad_spec` when its command can't run (exit 127, "command not found"). A check that errored is `transient` for a timeout or 429/5xx (an `http` check whose connection was refused, dropped or timed out says "network error" by reqwest's error kind, not its wording), else `bad_spec`. A lease that ran out or a turn that ended without a review is `stalled`; over budget is `budget`; a denied action is `denied`; a task waiting on the person is `needs_input`. Other errors are `quota` (a usage limit or empty balance: Claude's "usage limit reached" / "hit your limit" / "5-hour limit reached", credit too low, insufficient_quota, 402, and kwant's own "is out of usage" / "Quota exceeded"), then `auth` (not logged in, 401, expired token, and kwant's own "signed out or its key was rejected" / "No account can run bots" / no other account on), then `transient` (rate limit, overloaded, timeouts, connection errors, 429/500/502/503/504/529 as whole numbers), else `unknown`.
- The text is redacted before it is stored. Labelled values are masked however short they are: `password=`/`token=`/`secret=`/`key=` and their `--flag value` / `--flag=value` forms (including `--…-key`), `"key": "…"`, `Authorization:` (the scheme is kept), the password in `user:pass@host`, and `-p`/`-a` only on a line that runs `mysql`, `mysqldump`, `mysqladmin`, `mariadb`, `sshpass` or `redis-cli`. A bare word is masked when it has a known key prefix (`sk-`, `xai-`, `ghp_`…) or reads as generated: 20+ letters and digits that break into short pieces (a hex or mixed-case key). Names built from words and a number, paths, URLs and UUIDs stay readable, and only the secret word itself is replaced. Masking is idempotent, so text masked twice is unchanged. A short bare password with no label can't be told from a word and is not masked. A check's spec is masked the same way wherever it is shown (task JSON, Needs-you text, failure excerpt, retry prompt, workflow changes); the stored spec keeps the real values, and a `workflow_propose` whose spec still holds `[redacted]` is refused. The excerpt is at most 2,000 characters. The full redacted output goes to `<data_dir>/autofix/<task_id>/<uuid>.log` (folder 0700, file 0600, 256 KB). No route returns that path.

### Attempts

- `autofix_attempts`: `id`, `task_id`, `failure_id`, `strategy`, `status` (`suggested`, `scheduled`, `running`, `succeeded`, `failed`, `skipped`), `not_before`, `reason`, `retry_task_id`, `tokens_spent`, `created_at`, `finished_at`, and server-only `idempotency_key` (unique: sha256 of task, failure and strategy, so a failure is never planned twice). At most one `scheduled`/`running` attempt per task (`autofix_one_live`), and a trigger refuses one past the task's `max_attempts`.
- Strategies: `transient` → `wait_and_retry` after 1, 5, then 25 minutes; `stalled` → `retry_from_checkpoint` (the folder is as the run left it) or `retry_same`; `check_failed` → `retry_with_diagnostics`; `bad_spec` → `propose_change` (a `check_spec` proposal making the check optional; the attempt stays `suggested` and never runs); `unknown` → `retry_same`, once per task. `auth`, `quota`, `budget` and `denied` are never retried: a `skipped` attempt (reason `not_retryable: <what to do>`) and a `fix_needed` Needs-you item saying what to do. Neither repeats the failure's summary, which the task already shows. `needs_input` and hand-off tasks get no attempt.
- Limits, checked when planning and again when starting: a `blocked` task is never moved (`not_retryable`); no attempts left (`no_attempts_left`); `recovery_spent_tokens` plus the average spend per attempt over the recovery budget (`over_recovery_budget`). A limit makes the attempt `skipped` with that reason; a skip for `over_recovery_budget` also gives the failed task a `fix_needed` Needs-you item ("Retry budget used up: raise it or retry by hand.").
- While attempts are left and AutoFix isn't off, a failure parks the task in `failed` instead of `ready` (no "failed" chat line). The retry moves it `failed → ready` as `Actor::AutoFix`, the only move that actor may make, with a task event of kind `autofix`. A routine run is retried as a new routine run; its task gets `forked_from` = the failed task and counts on from its attempts, and the failed task stays `failed`.
- Recovery spend: a claim taken while an attempt runs carries its `autofix_id`, and that claim's spend (at hand-in and at turn end) also adds to `tasks.recovery_spent_tokens` and the attempt's `tokens_spent`. Plain runs never count as recovery. Token budgets (the task's and the recovery budget) count fresh input, cache writes and output, as Phase A spend does; cached input that is read again (`cache_read`) is left out. `model_usage.cost_usd` is the real-money figure.
- The retry prompt (runner and `task_claim`) says which attempt it is and why, the failing check and its redacted output, `KWANT_IDEMPOTENCY_KEY=<key>`, and the external tool calls earlier attempts already made. Calls in an attempt's turns record the key in `turn_tool_calls.idempotency_key`.
- An attempt ends `succeeded` when its task is verified, `skipped` when it is cancelled, `failed` at the next failure.
- Scheduled attempts live in the table; the scheduler tick runs the due ones, so a restart loses nothing.

### Workflow changes (person only)

- `workflow_changes`: `id`, `task_id`, `routine_id`, `target` (`check_spec`, `task_acceptance`, `max_attempts`, `routine_instructions`, `routine_schedule`), `before`, `after`, `reason`, `proposed_by`, `status` (`proposed`, `approved`, `rejected`, `applied`, `stale`), `decided_by`, `decided_at`, `created_at`. Triggers: only the person decides; `applied` only after `approved`.
- Bots propose with the `workflow_propose` tool; kwant reads `before` itself. Approving applies the change once, only while the target still holds `before`; otherwise it goes `stale` and nothing is written.

### Routes

- `GET /api/tasks/:id/failures`: oldest first, each `{id, attempt, source, class, summary, excerpt, checkpoint_id, created_at, autofix: attempt|null}`.
- `POST /api/tasks/:id/autofix` (person only, `Idempotency-Key` accepted), body `{failure_id?}`: runs the suggested or scheduled attempt now, or plans one. 409 with `code` `not_retryable`, `no_attempts_left` or `over_recovery_budget`.
- The manual `POST /api/tasks/:id/retry` stays open after an `auth` or `quota` failure and returns 200: the person retries by hand once they have signed in again or the limit has reset. Only AutoFix's own retry (`POST /autofix`) refuses these classes.
- `POST /api/tasks/:id/autofix/:aid/cancel` (person only): a `suggested`/`scheduled` attempt becomes `skipped`.
- `GET /api/workflow-changes?status=&task_id=` (both filters optional), `POST /api/workflow-changes/:id/approve`, `POST /api/workflow-changes/:id/reject`: all person only (`/api/workflow-changes` is a person-only segment).
- `task_json` gains `recovery_spent_tokens`, `recovery_budget_tokens`, `last_failure: {class, summary, created_at}|null` and `autofix: attempt|null` and `autofix_attempts` (retries AutoFix started: running, succeeded or failed). Needs-you kinds: `autofix_suggested`, `fix_needed`, `workflow_change`.
- Insights gain `autofix_7d: {attempts, succeeded, tokens}` and `failures_by_class_7d`, for the space.
- Task events: `failure`, `autofix_scheduled`, `autofix_started`, `autofix_finished`, `workflow_change_proposed`, `workflow_change_decided`, `workflow_change_applied`.

## 14. Flight School (Phase C3)

Design and decisions: `/home/maus/shared/ship-software-safely/mission-control/phase-c/contract.md` §3
and §9 (D4, D7, D8, D9, D10). A bot's tier for a class of tasks is a **ceiling** on top of its approval
mode: it can add a card, never remove one. Scorecards and tier proposals are C4, not here.

### Settings

- `flight_school` (bool, default `false`). Off, nothing is gated, whatever tiers are saved: every call
  takes exactly today's path. On, tiers apply to turns that work a task (a chat turn of a bot with no open
  claim is never gated).
- `flight_school_new_bot_tier` (`observe`, `sandbox` or `act`, default `sandbox`): the `'*'` tier a bot hired
  while Flight School is on starts at (a trigger on `kwants`). Bots that were there before have no row,
  which is `act` (D4), so switching the feature on changes nothing until the person lowers a bot.

### Tables

- `tasks.task_class` (`TEXT NOT NULL DEFAULT 'general'`, slug `[a-z0-9-]{1,40}`). Set by the person
  (`POST /api/tasks`, `PATCH /api/tasks/:id`, `POST /api/missions`) or a lead's `mission_plan`, never for a
  task the planning bot would run itself (owner, or no owner and it leads). Forks inherit it. A change
  writes a `class_changed` task event.
- `capability_policies`: `id`, `kwant_id`, `task_class` (a class or `'*'`), `tier`, `granted_by`
  (`CHECK = 'person'`), `granted_at`, `revoked_at`, `note`. One live row per bot and class
  (`capability_live`). Rows are never deleted or edited; the only update sets `revoked_at` once (triggers).
- `capability_events` (append-only): `kwant_id`, `task_class`, `kind` (`granted`, `revoked`,
  `new_bot_default`), `tier`, `actor`, `detail`, `created_at`.
- Rollback: `kwnt::db::flight_school::FLIGHT_SCHOOL_DOWN` drops the tables and triggers and the column
  (tested).

### The gate

`flight_school::tier_gate(db, data_dir, req)` is the one function, asked at four points:
`BrokerApprover::classified` (MCP tools and claude's tools that ask), the built-in Allow of claude's
permission prompt (an allowed in-folder write falls through to the approver instead), `SelfToolsApprover`
(kwant's own tools) and `approve_local_shell_in` (the API drivers' `execute_shell`). It reads the policy on
every call. In order:

1. The payment floor (`payment_floor::reason`). No tier skips it; at `act` a payment still cards.
2. Flight School off, or the turn works no task → no gate.
3. The tier for the task's class: the bot's row for that class, else its `'*'` row, else `act`. A policy
   that can't be read counts as `observe` (fails closed).
4. The call's effect: `checkpoints::effect` (the ledger's), with kwant's `memory_*`, `fact_*`, `task_*`,
   `mission_plan`, `tier_status`, `ask_user` and read-only self tools as `none`.

| effect | `observe` | `sandbox` | `act` |
|---|---|---|---|
| `none` | today's path | today's path | today's path |
| `workspace` (file tool in the task folder) | card | runs if this run has a `before_run` checkpoint, else card | today's path |
| `external` (`Bash`, MCP tools, outside files, other self tools) | card | card | today's path |

A forced card carries `details.tier_reason` ("Sandbox: commands that can change things ask", "Observe: every change
asks", …). Approvals `off`, smart Allow and `always_ask` don't skip it; a session "always allow" for that
tool in that chat does (D9), except on a payment.

**Known gap (waived): read-only commands in the bot's own folder.** The Claude CLI runs with
`--permission-mode default`, and its own allow-list runs read-only commands such as `ls`, `cat`, `pwd`
and `date` inside the turn's folder without asking kwnt. They never reach the gate, with Flight School
on or off, and no ledger row is written. A command that changes something, or that reads outside the
folder, still asks, so at `sandbox` "commands that can change things ask" is what holds, not "every
command". The API drivers' `execute_shell` goes through `approve_local_shell_in`, so there every
command asks below `act`. Accepted 2026-10-28 (Rigel's C3 check, Ada's decision): nothing can be written
this way, and the bot can read only its own folder.

**A card answered in time** sends no event of its own. `POST /api/approvals/:id/resolve` re-runs
`listener::refresh_needs_you`, and `task_update` and the runner's `after_turn` first move a task that
waits in `needs_you` on nothing back to `working` (`listener::settle_needs_you`). Before this, a bot's
`task_update review` right after an answered card was refused ("the task is waiting on the person"),
the turn ended, and the next tick put the task back in `working` with no turn running it.

### Revoke and engines

- `POST /api/capabilities/:kwant_id/revoke` writes the lower tier in one transaction; the running turn's
  next call is gated by it. A call already allowed and running finishes.
- The engine guard (D7): a task whose bot is below `act` on Codex or a custom engine (whose own tools kwant
  can't see) is moved to `blocked` with the reason "This engine can't be held to its tier…" and the
  Needs-you item `engine_cant_sandbox`: in the runner before the claim (pinned engines), and at turn start
  once the route is known (an `auto` bot routed there). Claude and Grok run.

### Routes

- `GET /api/capabilities?kwant_id=&space=` (open): `{flight_school, kwant_id, default_tier, policies,
  effective: [{task_class, tier, from: class|any_class|default|unreadable}], events}`.
- `POST /api/capabilities` `{kwant_id, task_class, tier, note?}` (person only): sets the tier; the same tier
  again returns `unchanged: true`.
- `POST /api/capabilities/:kwant_id/revoke` `{task_class, to_tier: observe|sandbox, note?}` (person only):
  lowers it; a tier that isn't lower returns `unchanged: true`.
- Bot tool `tier_status {task_class?}` (read-only, offered with the task board).
- `task_json` gains `task_class` and `owner_tier` (`null` while Flight School is off).

## 15. Opt-in budgets

Token budgets are opt-in. `budget_tokens` 0 means no limit, for tasks and missions.

- New tasks (person, hand-off, routine, chat, work-ahead) and missions get 0 unless a budget is
  given or the person set a default (`mission_default_task_budget_tokens`,
  `mission_default_budget_tokens`, both 0 by default). A plan task gets the task default, capped at a
  limited mission's budget; a task's own budget must be 0 or at most its mission's.
- No limit is never enforced: no `budget` block, `over_budget` stays false, nothing reserved, and
  AutoFix has no recovery cap (`recovery_budget_tokens` is 0, meaning none). The prompt a bot gets for
  a task leaves out the "Budget: N tokens" line.
- `PATCH /api/tasks/:id` and `PATCH /api/missions/:id` take `budget_tokens` 0 to remove a limit, or a
  positive number (tasks 1–5 000 000, missions 1 000–5 000 000). Anything else is 400.
- **Unblocking a task blocked for its budget** (`state` `blocked`, `blocked_reason` `budget`):
  `PATCH /api/tasks/:id {"budget_tokens": 0}` (or a higher limit), then `POST /api/tasks/:id/unblock`.
  The task goes `blocked → backlog → ready` when its dependencies are verified and its mission is
  active. A hand-off task then carries on (§16).
- Migration (once, when the stored table still has `CHECK (budget_tokens > 0)`): `tasks` and
  `missions` are rebuilt with `CHECK (budget_tokens >= 0)`, keeping every column, index and trigger,
  and in the same transaction rows still at the old silent defaults lose them:
  `UPDATE tasks SET budget_tokens = 0 WHERE budget_tokens = 50000` and
  `UPDATE missions SET budget_tokens = 0 WHERE budget_tokens = 200000`. Task states are not changed.
- The hand-off token limit per request (`chain_token_budget`, fresh tokens) is separate and is
  opt-in too: 0 (no limit) is the default. A person who sets a number gets the `budget_paused` notice
  and Continue as before. Migration (once, flag setting `chain_token_budget_opt_in_v1`): a stored
  `chain_token_budget` of exactly `200000`, the old default that Settings wrote back on every save,
  becomes `0`; any other stored number stays, and so does `200000` saved after the migration ran.

## 16. Hand-off cards that go on

A hand-off task (`source_kind` `delegation`, `source_id` the delegation) is held by kwant with a
system claim whose context is the hand-off's thread. The mission runner never starts hand-offs, so
the listener (`missions::listener`) does it when such a card reaches `ready` (unblock, person retry,
AutoFix retry; also the scheduler's reconcile, for cards left in Ready earlier) and isn't held by a
restore (`held_at`):

| Hand-off status | What happens |
| --- | --- |
| `queued` / `running` | kwant takes the card again → `working` |
| `completed` | → `working` → `review` (its result was already delivered; the work isn't run again) |
| `failed` / `cancelled` / `interrupted` | the delegation is reopened (`queued`, error and times cleared) and the teammate gets a carry-on turn in the same thread ("carry on from where you stopped", with why it stopped); the card goes back to `working`; the new result is posted to the origin chat and the lead is resumed with it alone |
| can't carry on (a bot is gone, or another call reopened it first) | `ready → blocked`, reason `listener::CANT_CARRY_ON` |

A card still over its own token limit is blocked again (`blocked_reason` `budget`) instead.

- A budget block lands at turn end, so it never stops the live hand-off: the hand-off finishes (or
  fails) and its result reaches the lead as usual. While the card is blocked, its end is logged once
  as a `handoff_ended` task event (`{"status", "note"}`); unblocking then follows the table.
- A hand-off cut short by two restarts (`interrupted`) fails its card (AutoFix records it as a
  `handoff` error) instead of returning it to Ready.

### Every card in Review gets a lead review

A finished hand-off's card goes to `review` with the result as evidence, and only its lead's
`task_review` (or the person) settles it. Each time the lead is asked, a `lead_review_asked` task
event is logged; per stay in Review it's asked at most `listener::MAX_REVIEW_ASKS` (2) times:

| Path | How the lead is asked |
| --- | --- |
| batch results turn (also carry-ons, review send-backs, late answers, channel @mention hand-offs) | its results prompt names every card of the batch; if the turn ends with cards still unreviewed, one reminder turn lists all of them |
| the card was blocked when the hand-off finished, then unblocked | `delegation::spawn_review_turn` right after it moves to `review`, plus the one reminder |
| kwnt restarted before or during the lead's turn, or a turn was lost | `listener::sweep_unreviewed` (from reconcile): a card quiet for `REVIEW_GRACE` (2 min) with no turn or batch running gets a review turn |
| `continue_lead: false` (a one-way notice; nobody is resumed with its result) | never put in Review: the card is closed (`cancelled`, never `verified`) with reason `listener::NOTICE_CLOSED` ("Notice, no review needed"); `task_json` gives it as `closed_note`, and the board labels it "Closed" |

After both asks the card waits, "Waiting for <lead> to review", for the lead or the person. Cards
from before this change (no result evidence) and cards whose lead was deleted are left for the person.

### One card per hand-off line

When a lead's message names the same teammate on two or more lines that each @mention only that
teammate (`@Frontend, please build the menu.` on one line, `@Frontend, please build the footer.` on
the next), each line is its own hand-off: its own delegation, thread and board card, titled after
its line, with the whole message as context and "Your task is this line of <lead>'s message only:
<line>". Each card gets its own review and checks. The lead is still resumed once, when the last
hand-off of the batch ends. Lines inside a code fence don't count.

Dedupe, and why: a teammate named on **one** line, or on lines that also name other teammates,
still gets one hand-off with the whole message, as before, since kwant can't tell which part of a
shared line is whose. The same line repeated word for word counts once (it's a repeat, not more
work). Lines past the per-teammate cap (4) wait for Continue; on Continue, lines already sent are
skipped, so no line is sent twice. The lead's open limit (`delegation::MAX_OPEN_PER_LEAD`, 5 unfinished)
still applies per hand-off: a line past it is refused with the usual `handoff_refused` note. A dependency wait
("after @X") on a teammate with several cards waits for the last of them started.

### Capped hand-offs: the Continue notice

One person message starts at most `routes::MAX_DELEGATIONS_PER_REQUEST` (16) hand-offs by mention,
at most `routes::MAX_MENTION_DELEGATIONS_PER_BOT` (4) to one teammate. These caps are what stop
loops by default; `chain_token_budget` adds a token limit only when the person sets one (§15). Mentions past a cap are not dropped: kwant saves one system message in the channel
with meta

```json
{"kind": "handoff_capped", "lead": "<lead id>", "waiting": ["<name>"], "waiting_ids": ["<id>"],
 "sent": 16, "text": "<the lead's reply>", "user_prompt": "<the person's message>", "resumed": false}
```

and text "Paused after N hand-offs so it doesn't loop: A, B not sent yet. Press Continue or say
"continue" to hand it on." It works like the `budget_paused` notice: `POST
/api/chains/:message_id/continue` (person only; 409 once continued, 410 if the lead is gone), or the
person typing "continue" while it's the newest bot message. Continuing sets `resumed` true and hands
only the waiting teammates the lead's reply, with a fresh allowance.

## 17. Performance Market (Phase D1, report-only)

kwant observes every task once it settles and reports, per task class × bot (× model), how often work
ends **verified**, how long it takes, what it costs and how often the person stepped in. Nothing reads
these numbers to route work: Laya stays the router, and bots can't read the report.

**One row per task** in `performance_observations`, written when a task moves to `verified`, `failed`
or `cancelled` (`after_move` → `performance::record_soon`, in the background; a failure is logged and
never fails or slows the move). `cancelled` counts as `unverified`, and only if the task ran (had a
claim). Tasks with no owner are skipped. A reopened task that settles again replaces its row, so it
counts once, by its last outcome, with the earlier attempts in `retries` / `autofix_attempts`.

| Field | From |
|-------|------|
| `latency_ms`, `runs`, `started_at` | the task's claims (time a bot held it), their count, the first one |
| `fresh_tokens` | `tasks.spent_tokens` |
| `model`, `cost_usd` | the owner's `model_usage` rows inside the claim windows (model with the most tokens, else the bot's preferred model); `cost_usd` is null if none was reported |
| `engine` | the bot's driver type |
| `approvals`, `check_approvals`, `unblocks`, `retries`, `autofix_attempts` | tool cards answered, owner checks the person approved, person moves out of blocked/needs you, person `failed → ready`, AutoFix attempts that ran; `interventions` is their sum |

Model and cost are attributed by time, not by turn: a bot chatting elsewhere while it holds a claim
adds to the task's cost. The token number is the exact mission meter.

**Backfill.** On startup, once (setting `performance_backfill_v1`), every finished task is observed
with the same rules and inserted as `source = 'backfill'`; rows that exist are never overwritten.

**`GET /api/performance?space=&class=&bot=&since=&by=`** (person-only, read-only). `since` is
`Nd` (1–365), a `YYYY-MM-DD` date or an RFC 3339 time, default `30d`; `by` is `model` (default) or
`bot`; anything else is 400. Each group has `n`, `verified` / `failed` / `unverified`,
`verified_rate`, `verified_ci95` (Wilson score interval, z = 1.96), `median_latency_ms`,
`median_fresh_tokens`, `cost_usd` and `cost_per_task_usd` (over rows with a cost),
`interventions_per_task` and `low_confidence` (n < 5). Groups sort by class, then n descending.

## 18. Swarm Auditor (Phase D2)

For high-impact or low-confidence tasks, one teammate challenges with evidence, the owner answers once,
and the risks left unresolved are listed on the task. **An audit never changes verification**: it
never moves a task, never adds or runs a check and never writes a peer review; "no risks raised" is not
a pass. Design note: `phase-d/auditor.md`.

**Settings** (app-wide, like the other Mission Control settings): `auditor` = `off` (default) | `on`;
`auditor_daily_cap` 1–50 (default 5) audits started per UTC day; `auditor_model` = `fast` (default) |
`balanced` | `deep` | `bot`: the model tier both audit turns run on. A tier keeps the engine Laya picks
and uses that engine's model of the tier (a pinned model of another known tier gives way; effort is at
most medium below `deep`); where the engine has none, its default model. `bot` routes as usual.

**Trigger.** A task entering `review` with an owner, while `auditor` is on, when its priority is high or
urgent (`high_impact`) or no check that counts and isn't the owner's will judge it (`low_confidence`,
`checks_summary.status = not_checked`). The person can start one with `POST /api/tasks/:id/audit`
(`manual`). One audit per task attempt; one audit runs at a time server-wide. A skipped audit (cap,
no challenger) logs `audit_skipped`.

**Challenger.** A bot of the task's space other than the owner, the mission's lead and the lead that
handed the task off. Ranked by verified tasks of the task's class (Performance Market), then fewest
challenges in the last 7 days, then name and id.

**Exchange.** Two hidden turns, each in the task folder (§12; where the owner wrote the files, not the
challenger's own folder), each `always_ask` (any call that could change something asks the
person; payments ask in every mode, D8), each capped at 20 minutes:

1. The challenger calls `audit_challenge {audit_id, risks: [{risk, evidence, evidence_ids?}]}` once:
   at most 5 risks, each with evidence (≤1000 chars); `evidence_ids` must be ids of the task's evidence. An empty list
   ends the audit `done` with "no risks raised". No call → the audit ends `failed`.
2. With risks, the owner calls `audit_rebut {audit_id, answers: [{risk_id, stance, answer, evidence?}]}`
   once; `disputed` needs evidence. Unanswered risks stay `open`.

**One card per call.** Each audit call asks the person once. If the person allows it and the tool then
refuses its input (an evidence id that isn't the task's, a risk too long), the same bot's corrected
call for that audit stage goes through without a new card, at most twice per stage; the tool checks it
again as usual. The task's own id in `evidence_ids` is dropped rather than refused. Refused audit calls
are logged with their reason.

While an audit of a task is `queued`, `challenge` or `rebuttal`, its challenger and owner can't use
`task_claim`, `task_update`, `task_attach_evidence`, `task_request_check` or `task_review` on it.

**Risks** end `open` or `disputed`; both are unresolved. Only the person closes one: `resolved` or
`dismissed`. Tokens of each turn (`model_usage` in the turn's window) are stored per stage.

**Ledger** (`task_events`): `audit_started`, `audit_challenge`, `audit_rebuttal`, `audit_done`,
`audit_skipped`, `risk_resolved`.

**Restarts.** An audit is never left running. At startup, one in `challenge` or `rebuttal` becomes
`interrupted` ("kwant restarted during the audit", logged as `audit_done`; risks already raised stay
listed); a `queued` one runs now, or is `skipped` ("the auditor was turned off") when the auditor is
off. A driver that stops early (a panic) ends its audit `failed`.

**API**

- `GET /api/tasks/:id/risks` → `{audits: [{…, risks: [...]}], open_risks}`.
- `GET /api/risks?space=` → `{risks}`: unresolved risks of the space, newest first.
- `POST /api/tasks/:id/audit` (person only) → 202 with the audit; 409 when the auditor is off, the task
  isn't in review or verified, has no owner, its attempt was audited, the cap is reached or nobody can
  challenge.
- `POST /api/tasks/:id/risks/:risk_id/resolve {resolution: "resolved" | "dismissed", note?}` (person
  only); 409 when already closed.

## 19. Skill Foundry (Phase D3)

When a bot has finished the same class of work well several times, kwant asks that bot to write the
procedure down as a skill draft, replays the draft as a dry run on a few of those tasks, and shows both
to the person, who enables, edits or rejects it. **A Foundry skill is text only, like every skill**: the
tools it lists are shown, never granted, and enabling it changes no tool, server or approval. Design
note: `phase-d/skill-foundry.md`.

**Settings** (app-wide): `skill_foundry` = `off` (default) | `on`; `skill_foundry_model` = `balanced`
(default) | `fast` | `deep` | `bot` (as `auditor_model`); `skill_foundry_daily_drafts` 1–20 (default
2) draft runs started per UTC day; `skill_foundry_replays` 0–5 (default 3) examples each draft is
replayed on (0: no replays); `skill_foundry_replay_tokens` 5,000–500,000 (default 60,000) fresh tokens
one replay turn may use.

**Clusters** (the miner, hourly while on): one space, one bot, one `task_class` other than `general`,
from Performance Market rows of the last 60 days, with ≥ 3 ended `verified` (Verified Outcomes: only a
check that counts and isn't the owner's verifies) and ≥ 70% of the class's rows verified. A cluster gets
no draft while one waits for review (`has_draft`), while the bot has a live Foundry skill for the class
(`has_skill`; the person can still ask for a new version), for 30 days after one was rejected
(`rejected`), or for 7 days after its last draft run (`cooling_off`).

**Draft.** One hidden turn of the owner bot, `always_ask`, capped at 20 minutes, with up to 5 verified
and 2 failed tasks of the cluster (title, acceptance, checks with secrets redacted, evidence, the
side-effect ledger's actions). The bot calls `skill_draft {run_id, name, description, body, inputs?,
checks?, examples, tools?}` once; the tool is listed only while that bot has a draft run open, and needs
no card (it only stores a pending skill). Server-side: name, description and body as `skill_propose`;
≤ 8 inputs `{name, description, example?}` (≤ 300 chars each); ≤ 5 checks that pass the task-check
validator (no `person` checks); 2–5 distinct examples, each a verified task of the cluster; ≤ 30 tools,
each a built-in tool, one of kwant's own tools the bot is offered, or `mcp__<server>__<tool>` of one of
the bot's active servers (only the server is checked: kwant can't list a server's tools without starting
it). The draft is a `pending` skill (`source = agent`) with a `skill_meta` row (class, inputs, checks,
examples, tools, replay report, `version`, `lineage_id`). When the bot has a live skill for the class,
the draft is its next version (`replaces_id`). A turn without a draft ends the run `failed`.

**Replay** (after each draft, or `POST /api/skills/:id/replay`): for up to `skill_foundry_replays`
examples, one hidden turn of the owner with the draft in its prompt, in a private scratch folder
(`<data>/foundry/<run>/<n>`, 0700) holding a copy of the task's first checkpoint (empty without one).
The turn's approvals are a dry run: in-folder file tools and web reads run, kwant's read-only tools run,
and every other call that reaches the approver (Bash, files outside the folder, other servers' tools,
kwant tools that change something, the browser) is refused with "dry run" and recorded as what the bot
would have done; subagents are refused. Only Claude's permission hook holds every call to this, so on
another engine the replay is refused before any model runs. Each turn is capped at 15 minutes and is
stopped once it reports more fresh tokens than `skill_foundry_replay_tokens`. After the turn, the
draft's file checks run on the scratch folder; `command` and `http` checks are `not_replayable`, never
passed. The scratch folder is then deleted. Per example the report has the verdict (`passed`: every
replayable check passed; `failed`; `not_checked`: nothing replayable; `error`: the turn didn't run), the
checks, the "would have…" lines next to what the original task did, tokens and seconds. A replay is
evidence, not a gate, and never changes the draft's status.

**Versions and rollback.** Enabling, editing and rejecting are the usual skill routes. `POST
/api/skills/:id/rollback` (person only; works with the Foundry off) makes a `superseded` version live
again and the lineage's live version `superseded`, in one transaction; 409 for a version that isn't
superseded or whose name clashes with another live skill, 404 for a skill without Foundry history.

**Ledger.** Skill events: `skill_drafted`, `skill_replayed`, `skill_rolled_back`. Task events:
`skill_drafted_from` on each example.

**Restarts.** Runs (`foundry_runs`: `queued | drafting | replaying | done | failed | interrupted`) are
never left running: at startup a `drafting` or `replaying` one becomes `interrupted`; a `queued` one
runs now, or fails ("the Skill Foundry was turned off") when the Foundry is off. A driver that stops
early ends its run `failed`. One Foundry turn runs at a time server-wide.

**API** (all person only)

- `GET /api/skills/foundry?space=` → `{enabled, settings, drafts_today, clusters: [{id, kwant_id,
  task_class, verified, total, block}], skills: [skill + foundry meta], runs}`.
- `POST /api/skills/foundry/:cluster/draft` (`:cluster` = `<bot id>~<class>`) → 202 with the run; 409
  when off, at the cap, a draft waits for review or one is being written; 404 when the cluster doesn't
  pass the thresholds.
- `POST /api/skills/:id/replay` → 202; 409 when off, replays are 0, the draft isn't pending or a replay
  of it runs.
- `POST /api/skills/:id/rollback` → the skill.
- `GET /api/skills/:id/foundry` → `{skill, versions, events, runs}`.

Not built yet: recording `skill_id@version` on a task's claim, and warning when a bot lost a listed
tool.

## 20. Automation Discovery (Phase D4)

Design note: `phase-d/automation-discovery.md`. v1 learns only from board tasks the person created.
**Build step 1 (shipped): what a routine's runs are held to. Step 2 (shipped): the miner and a
read-only scan. Step 3: offers that are kept, Accept, Dismiss, the hourly scan.** The screens come in
step 4.

**Routine outcomes.** Every routine can carry, besides its instructions:
- `task_class`: the Flight School class of each run's task (§14). `null`, `""` or `general` clear it.
- `acceptance`: what a good result is, up to 4,000 characters. Without it, a run's task uses the
  instructions, as before.
- `checks`: up to 10 `[{kind, spec, required = true}]`, validated like task checks (no `person` or
  `peer_review` checks). `null` or `[]` clear them.

`POST`/`PATCH /api/routines` take them, and the routine views return them. Check specs are shown with
secrets masked, and a spec containing `[redacted]` is refused, so a masked spec can't be saved back.
When a run starts, its board task gets the class (event actor `person`), the acceptance and a copy of
each check, already approved by the person. When the run ends `ok`, the checks run and can take the
task to `verified`. A routine without these fields runs exactly as before.

**The class and checks are the person's.** A bot's turn token on `POST`/`PATCH /api/routines`, or the
bot's own routine tools, get a 403 when they set `task_class` or `checks`. Bots may still change
everything else, the acceptance included, and their changes keep the person's class and checks. An
undone delete brings back the class, the acceptance and the checks. The exception is a check whose spec
held a secret: its stored spec is never shown, so the undo can't restore it.

**The miner** (`src/automation_discovery.rs`, pure: no model, clock or database). It reads the
person's board tasks of the last 120 days: `created_by = person`, `source_kind` `person` or `mission`,
not forks, not cancelled. Tasks a bot, a routine, a hand-off or a chat made never count. Tasks group by
habit: `class:<class>` for a Flight School class other than `general`, else `title:` and 12 hex digits
of the SHA-256 of the normalised title (NFKC, lowercase, without numbers, dates, ordinals, English and
German weekday and month names, today/tomorrow/heute/morgen; too vague under 2 words and 8 letters).
Asks are counted once per local day in the person's time zone, so daylight saving never moves a habit.
Cadences, most specific first, each needing coverage ≥ 0.75 (slots asked, first to last) and precision
≥ 0.80 (asks on a slot):
- monthly, day N (a short month's last day stands in) or the last day: 120 days, ±3 days, ≥ 3 asks;
- weekly on 1–3 weekdays, each asked twice and each covering ≥ 0.75 of its own slots: 60 days, ±1 day,
  ≥ 3 asks over ≥ 14 days;
- weekdays (Monday–Friday, weekend asks count against it): 60 days, ≥ 5 asks on ≥ 4 weekdays;
- daily: 60 days, ≥ 5 asks including a weekend day.

A habit is `stopped` once two of its slots (plus their shift) passed without an ask. Then the gates: at
least 2 verified and verified ≥ 2/3 of verified plus failed (`not_verified`); one bot did ≥ 2/3 of the
verified ones (`mixed_owners`) and is still in the space (`bot_unavailable`); no routine of the space,
on or off, does it (`covered`: accepted from this habit, or the same bot with the same class or ≥ 0.6
word overlap with its name or first instruction line); not dismissed or deleted within 30 days
(`cooling_off`); no live offer (`already_offered`); not a step of another offer (`in_chain`); within
`automation_discovery_daily_cap` per 24 hours and 5 live offers per space (`capped`, strongest habits
first). Too few asks is `too_short`, no rhythm `not_regular`. A follow-up task created after ≥ 3 and ≥
80% of the last 20 asks, within 24 hours of each being done, is a chain step: written into the
instructions when the same bot did it, only mentioned when another bot did. The time is the median ask
time, down to the quarter hour ("time varies" past 4 hours apart); the first run is the next one after
now and at least half a period after the last ask. Checks that passed with the same spec on ≥ 2 verified
asks are kept (≤ 10); `person`/`peer_review` checks, ones whose spec changes every time and ones that
passed once are listed as left out. The name is the newest ask's title without its dates, years, numbers,
ordinals and weekday or month names, unless every source title (at least two) has the same word ("Top
10 leads for Monday", "Q4 board pack"); a shared month name or date counts only when the sources span
more than one month, a shared year more than one year. "For", "vom" and the like this leaves at an
end go too, as do empty or unpaired brackets ("Pay supplier invoices for October" →
"Pay supplier invoices"); a title that is nothing but those keeps its words without digits, or stays as
it is. Skipped habits are titled the same way. Each source is one matched ask: `local_date`
(`YYYY-MM-DD`) and `local_time` (`HH:MM`) in the person's time zone.

**Settings:** `automation_discovery` `off` (default) or `on`, `automation_discovery_daily_cap` 0–10
(default 2; anything else, a non-number too, is refused). Every `/api/automation-discovery` route is the
person's only: a bot's token gets 403. Nothing is created without Accept.

**Scan** (`POST /api/automation-discovery/scan?space=`): scans now (ignoring the hourly wait, not the
cap) and keeps what it finds. `{blocked_by: null, preview: false, timezone, offered: [suggestion],
skipped: [{cluster_key, title, asks, reason}], expired: [id], superseded: [id]}`. `blocked_by` is `off`,
`board_off` or `no_timezone` (none set, or one this machine can't read), with empty lists; with the
setting off nothing is read or written. `?preview=1` is step 2's preview: `{blocked_by, preview: true,
timezone, offered: [offer], skipped}`, writing nothing at all.

A scan, per space: a live offer whose habit no longer passes the miner is `expired`; one whose habit
still passes but changed (another bot, another rhythm, days or time zone, or the time moved ≥ 30
minutes) is replaced in one transaction: the old row becomes `superseded` (event `superseded`,
`{replaced_by}`) and the habit as it is now is stored live (event `offered`, `{cluster_key, replaces}`),
keeping the old `offered_at`, or neither happens. A replacement adds no live offer and is no new offer,
so the daily cap and the live limit don't apply to it, nothing counts it toward them, and it sends no
notification. Then each new offer is stored (`sug_…`) with one `offered` event, within the daily cap
(new offers in the last 24 hours, rolling, not local midnight; replacements don't count) and 5 live per
space, checked in the same statement as the insert, so two racing scans store one offer per habit.
`stats.offered_30d` counts new offers the same way. Delivery is re-checked: a thread that is gone or archived, or
a channel of another space, falls back to the bot's direct chat. Checks a routine may not carry are
moved to the left-out list.

**The hourly scan.** While the setting is on, the board is on and a time zone is set, every non-archived
space is scanned once an hour (the loop wakes every 10 minutes and reads
`automation_discovery_last_scan`). It writes nothing otherwise. Each new offer sends one notification,
kind `automation_suggested` ("Suggested routine: <name>", "<bot> could do this on a schedule. Accept or
dismiss it in Routines."), with no chat text; it can be turned off like any kind and is not a Needs-you
item. Lists saved before the kind existed count it as on.

**List** (`GET /api/automation-discovery?space=&status=`): `{setting: "on"|"off", daily_cap, blocked_by:
[every reason, empty when scans run], last_scan_at, suggestions: [suggestion], stats: {offered_30d,
accepted_30d, dismissed_30d, retained, dropped}}`. `status` is `offered` (default), `accepted`,
`dismissed`, `expired`, `superseded` or `all`, newest first; anything else is 400. While scans can't run,
live offers are hidden.

A **suggestion**: `{id, space_id, cluster_key, kwant_id, name, instructions, schedule, schedule_text,
first_run_at, first_run_text, delivery: {channel_id?|thread_id?|direct_kwant_id?}, task_class,
acceptance, checks: [{kind, spec, required}], left_out_checks: [{kind, spec, reason}], chain,
source_task_ids, sources: [{task_id, title, local_date, local_time, state}], time_spread, evidence,
permissions, status, routine_id, cooldown_until, routine_deleted_at, retention, retention_at, offered_at,
decided_at}`. Check
specs are shown with secrets masked. A live offer's first run is recomputed whenever it is shown: the
next slot after now and at least half a period after the last ask. Once accepted, it is the routine's
first run.

`permissions` previews what the bot may do when the routine runs; Accept grants nothing: `{flight_school,
tier: observe|sandbox|act, tier_from: class|any_class|default|off, tier_label, approval_mode:
ask|smart|off, payments_ask: true, past: {external, workspace, needed_ok}}`. `past` counts the tool calls
of the source tasks' turns that changed something outside or in the workspace, and those the person
approved.

**Accept** (`POST /api/automation-discovery/:id/accept`, `Idempotency-Key` required, else 400). Body,
all optional: `{name, instructions, schedule, kwant_id, channel_id, thread_id, acceptance,
keep_checks: [index]}`; unknown fields are 400. Checks can be dropped (`keep_checks`, out of range: 422),
not added. Another bot must be in the suggestion's space; a thread or direct chat belongs to the
suggested bot, so another bot delivers to its own direct chat unless told otherwise. The routine is
built and checked like `POST /api/routines`, then gets `origin: "discovery"`, the `suggestion_id` and
the first run recomputed for now. The status change, the routine and the `accepted` event are one
transaction. 201 `{suggestion, routine, first_run_text}`; the same key replays it
(`idempotent-replayed: true`). Once the offer isn't live: 409 `{error, status, routine_id}`. A `?space=`
naming another space: 404.

**Dismiss** (`POST /api/automation-discovery/:id/dismiss`): 200 `{suggestion}`, with `cooldown_until`
30 days on; the habit is `cooling_off` until then. 409 as for Accept.

**After Accept.** Turning the routine off or on appends `disabled`/`enabled` (actor `person` or
`kwant`); deleting it sets `routine_deleted_at`, appends `deleted` and cools the habit off for 30 days.
An undone delete makes the new routine the suggestion's again (`origin`, `suggestion_id`, and the
suggestion's `routine_id`), so the habit is covered again. 14 days after Accept, each scan's retention
pass marks the suggestion `retained` (its routine exists and is on) or `dropped`, once, with an event;
it runs every 10 minutes whatever the setting says now.

**Export.** A bundle's routines carry `task_class` and `acceptance`, and `checks` only when no spec
holds anything that looks like a secret. An import brings the class and the acceptance; checks are
never applied (the person approves checks), and the preview says so.

**Storage** (add-only, idempotent): `routines` gains nullable `origin` (`discovery` for an accepted
offer), `suggestion_id`, `task_class`, `acceptance` and `checks` (JSON). `routine_suggestions` holds
offers, with one live offer per `(space_id, cluster_key)`. `routine_suggestion_events` is their
append-only history, enforced by triggers. Older binaries ignore all of it.

## 21. Truth Ledger (Phase C1, shadow mode)

Contract: `phase-c/contract.md`. **C1 ships the ledger in shadow mode only: it records, the person can
read and decide, and nothing the bots see changes.** Memory stays the write path; C2 makes it a view of
the ledger.

**Setting.** `truth_ledger`: `off` (default), `shadow` or `on`, set by the person in `POST /api/settings`
(a bot's token gets a 403, and the bots' own settings tool refuses it). **In C1 `on` behaves exactly like
`shadow`.** The mirror below runs in every mode, so turning the ledger on later shows the whole history;
the setting only gates the routes.

**What it covers (D1).** Bot memory (`kwant_memories`), the person's notes on a bot and imported notes.
Team and space facts, evidence, citations and disputes have tables but nothing writes them in C1.

**The mirror (D2).** SQL triggers on `kwant_memories` mirror every insert, text change, pin and delete
into `knowledge_facts`, so any writer (an older binary included) is seen; no row is moved. A fact has a
`trust` and a `status`:
- the person's note: `person`/`accepted`, decided by the person;
- a bot's note: `bot`/`accepted`;
- a bot's note from a turn that read outside text, or an imported note: `outside`/`proposed`
  (`source_kind` `bot` with the turn id, or `import` with the import id). The memory row is unchanged:
  an import's note keeps `source = 'user'` and `imported = 1`, and the bot still sees the note (D3).

An edit supersedes the fact in force and adds a new one (`superseded_by` points to it). A bot rewriting
a note whose history ever had an `outside` fact lands `outside`/`proposed` again, even in a clean turn
and even after the person accepted an earlier version; so does a bot re-adding the exact text of an
`outside` fact it had (delete, then add). A delete supersedes the fact with no successor. One fact per
memory is in force at a time; the rest are its history. Deleting a bot, or undoing an import, deletes
its facts and their events: those two paths hold a row in `fact_purge_permits` for the bot inside their
transaction, and any other `DELETE` of a fact or of a living fact's event is refused.

**Taint.** A turn is tainted once it calls a tool that brings outside text in: web fetch or search, a
shell, a sub-agent, any MCP server other than kwant's own, the browser, `read_attachment`, `mcp_search`
or `page_source`, or when every call needs approval (the turn is assumed to). A bot note written later in
that turn is `outside`. The bit lives in memory for the turn and is cleared at its end.

**Rules in the database.** `superseded` is final; accepting a `proposed` or `disputed` fact needs
`decided_by` and `decided_at`; an `outside` fact is accepted only by the person, and so is any team or
space fact that is accepted or `outside`. A fact's trust leaves `outside` only with `decided_by =
'person'`, and once the person decided a fact nothing changes `decided_by` again, so an ex-outside fact
never ends up `decided_by = 'evidence'`. `fact_events` is append-only (`created`, `superseded`,
`pinned`, `unpinned`, `removed`, `backfilled`, `accepted`, `rejected`). The triggers are recreated on
every start, in one transaction, so a database an earlier build migrated gets the current rules.

**Routes, all person only** (a bot's token gets a 403 on every one, reads included; bots get facts
through tools in C2):
- `GET /api/facts?space=&scope=&scope_id=&status=&q=`: `{setting, facts}`, newest first, at most 500;
  with the ledger off `{setting: "off", facts: []}`.
- `GET /api/facts/:id?space=`: `{fact, events, evidence: [], citations: [], disputes: []}`; 404 when off.
- `POST /api/facts/:id/accept` and `/reject` (`Idempotency-Key` optional; the same key replays): a
  `proposed` fact becomes `accepted`, or `superseded` with no successor, decided by the person. 409
  `{error, status}` when it isn't proposed, so of racing clicks exactly one changes it; 409 when off. In
  C1 a rejected note stays in the bot's memory.

**Storage** (add-only, idempotent): `knowledge_facts`, `fact_evidence`, `fact_citations`,
`fact_disputes`, `fact_events` and `fact_provenance` (a write's pending trust, consumed by the mirror).
The first start backfills every memory that has no fact (an `imported` note as `outside`/`proposed`,
others by source) with a `backfilled` event; running it again adds nothing. Older binaries ignore the
tables and their memory writes are still mirrored. **Rollback:** releases before the ledger (2b936e4)
need nothing. A703732 deletes a bot's facts without a permit, so before rolling back to it run
`DELETE_GUARDS_DOWN` (drops `knowledge_facts_no_delete` and `fact_events_no_delete`), or deleting a bot
fails; the next start of a later build puts them back.

**Known limits.** Taint is per turn and coarse: a tool call that was asked about and then denied still
taints, and a bot can save text it read in an earlier turn from a clean one, or re-add a removed
`outside` note in other words. Notes from before the
ledger can't say whether their turn read outside text, so they are `bot`/`accepted`. An edited or
deleted note keeps its old text as history in the ledger until the bot is deleted.
