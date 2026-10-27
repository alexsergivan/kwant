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
  (`required: false`) are shown but never hold, fail or verify a task.
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
// check's id, approve it with POST /api/checks/:id/approve) while in review

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
