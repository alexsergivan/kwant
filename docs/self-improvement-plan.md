# 22. Self-improving bots: lessons, the measured loop and the ladder (Phase E)

Status: a plan, not shipped (2026-11-03). Written in the style of the numbered sections of `docs/mission-control.md`, so sections 1 to 10 can move there as §22 when the core ships. Sections 1 to 10 are the core: about three weeks of PR-sized steps that close the loop with no hidden turns and no machine-written memory. Section 11 is the ladder after the core, each rung gated by the evidence the core produces. Section 12 says how the plan was made.

Answers: "What can we implement to make kwant bots auto-improve themselves automatically?"

Spine: the "Lessons first" design (PR-sized steps on existing ledgers). Grafted from the other designs where the judges marked them worth stealing: a structured lesson key and a counts-only digest (reflection design), a measured skill effect with a concrete consumer (skills loop), attribution columns on `model_usage` and the `turns_completed` write (autonomy ladder, closed loop), and an automatic withdrawal rule for every automatic change (closed loop). Every blocker the judges and the critics raised is resolved below and marked "(resolved)"; the three claims the code readers refuted are corrected where the plan relied on them.

## 1. The question and what "self-improving" means here

kwant already records everything a colleague would learn from: `task_failures` with a classified cause, `autofix_attempts` with the strategy that worked, `outcome_checks`, `task_events` (including the person's rejection reasons), `performance_observations`, `decision_notes`, `route_handbacks`. Nothing reads any of it back into the next turn or the person's decisions. Self-improvement here is a feedback path over those ledgers, not a new learner, a hidden turn or a stored score.

The loop, per component:

| step | what it is in this plan |
|---|---|
| signal | rows kwant wrote itself: `task_failures.class`, `autofix_attempts.status`, `outcome_checks.kind`, `task_events` by actor `person` (rejections with their reason, blocks, edits, retries), `performance_observations.result`, `decision_notes.source`, `route_handbacks` |
| learner | pure functions computed on read (the scorecard convention): `lessons::block`, `lessons::effect`, `skills::effect`, `changes::effect` |
| change | one sentence in the runner brief, a derived prompt section, one retry's model tier; or a proposal the person taps |
| rail | caps, fixed templates, the person's explicit choices always win, every lasting change is a tap, every automatic change withdraws itself when its numbers fall |
| measure | `lessons_shown`, `skills_in_prompt` and `self_checked` task events joined to `performance_observations` with `performance::wilson`; shown in Insights and on the bot before anything acts on them |

"The bot got better" means, per bot and per bot x class where n allows: verified rate holds or rises (Wilson lower bound), first-try rate rises (`card_of`'s rule: verified with `attempts <= 1` and no `task_failures` row), repeated failures fall, person interventions per task fall, false success does not rise (resolved: a check failed by a peer review or the person after the owner's own check passed, plus drift flags, computed per bot x class in `lessons::false_success` from `check_result` and `review` events; today's `false_success_7d` counts only checks failing after a non-person move to review), tokens and cost per verified task do not rise more than 25%, and hand-backs plus "Wrong bot?" corrections per 100 routed messages fall. Never a bot's word: a task is verified only through `verdict()` and the `tasks_verified_needs_check` trigger, and this plan adds no path to `verified`. A bot learning to pass its own weak check rather than do the work is what the false-success figure catches; it is a pause criterion.

What this plan deliberately does not do: no hidden turns, no notes written into `kwant_memories` by the machine, no automatic change to persona, soul, rules, tier, model pin, skills or routines. (resolved) The rails for a hidden reflection turn exist (the Foundry and the Auditor run `always_ask` turns with held tools, a purpose, a daily cap and a model setting); the exclusion is a cost choice, not a safety one, and `lessons_reflect` is specified under "Deferred".

## 2. Principles and the autonomy ladder

Two kinds of change, two rails: lasting changes are proposals the person decides; non-lasting adjustments may be automatic only when bounded, windowed, visible, attributable and self-withdrawing. The ladder:

| change class | mode | rail that replaces the person | evidence that moves it up | setting |
|---|---|---|---|---|
| attribution records (the new `task_events` kinds; `model_usage.turn_id`, `tier`, `effort`; `kwants.turns_completed`) | report-only | append-only ledgers, no prompt bytes, person-only reads of scores | none; always on with the feature | `lessons`, none for Phase 4 |
| self-check sentence in the runner brief | automatic | fixed text, no data; the check runs through the existing `task_request_check` tool and its approvals | n/a | none |
| derived `[Lessons from earlier tasks]` section | automatic | computed on read from kwant-written rows and person-written reasons; fixed templates; 30-day window; byte caps; omitted when empty; paused on a measured drop | n/a (the bottom rung) | `lessons` off/measure/on |
| deep-tier retry for one AutoFix attempt | automatic | inside the AutoFix caps and a daily cap; only `retry_with_diagnostics`; never a pinned model; one per task; paused when deep costs more without succeeding more | n/a | `autofix_retry_tier` same/deep |
| pinned lesson (a user skill for a procedure, a note for a fact, a `routine_instructions` change for a routine) | proposal | report item with Pin or Dismiss; person-only trigger; caps never evicted | same key 3 or more times across 2 or more tasks in 30 days, once after it was shown | `lessons` on |
| disabling a skill that measures worse | proposal | `SkillsPanel` badge, later a `skill_hurts` notification plus the existing person-only disable route | n >= 10 with and without in the same class, "with" upper bound below "without" lower bound | `lessons` on |
| undoing a lasting change that measures worse (pinned lesson, granted tier, enabled skill, applied workflow change) | proposal | `change_regret` notification deep-linking to the existing undo | 14 days before versus after, same rule | `lessons` on |
| persona, soul, rules, tier, model pin, routines, hires | proposal via existing tables | `workflow_change_needs_person`, `tier_proposals_only_decide`, person-only routes | not in this plan | existing |
| hand-backs as a routing exclusion; model or effort prior | deferred, report-only | counts in `GET /api/performance`, which stays report-only | the sticky-question path hinted; Phase 4 plus 4 weeks of data | none yet |

Every new setting ships off (or `same`) by default (resolved: the spine proposed two switches on; the convention and the red-team list win). The switches are app-wide keys in the global `settings` table, like `autofix`; the rows the loop writes carry `space_id`, and every new route resolves its kwant or task through the request's `Scope` and returns 404 across spaces (a case in `tests/spaces_isolation_test.rs`).

## 3. Architecture overview

Signals already collected and what each component reads:

| ledger | columns used | by |
|---|---|---|
| `task_failures` | `task_id`, `source`, `class`, `attempt`, `created_at` (never `summary` or `excerpt`) | C1, C5, C6, C7 |
| `outcome_checks` | `kind`, `spec` (through `redact_value`, only when `approved_by` is `person` or `mission`), `status`, `output_excerpt` (120 redacted chars) | C0, C1 |
| `autofix_attempts` | `failure_id`, `strategy`, `status`, `tokens_spent`, new `tier_requested`, `tier_ran` | C1, C2, C7 |
| `task_events` | `state` by actor `person` with `detail.reason`, `edited` with `acceptance: "changed"`, new kinds `self_checked`, `lessons_shown`, `skills_in_prompt`, `memories_in_prompt` | C0, C1, C3, C6, C7, C8 |
| `decision_notes` | `source = 'changed'` | C1 digest |
| `performance_observations` | `task_id`, `kwant_id`, `task_class`, `result`, `fresh_tokens`, `cost_usd` | C1 digest, C3, C7, C8 |
| `route_handbacks` | `from_kwant_id`, `created_at` | C4 (report) |
| `kwant_memories`, `skills`, `workflow_changes`, `capability_events` | timestamps of the person's lasting changes | C8 |

New code: `src/server/missions/lessons.rs` (pure block, key, repeats, effect, false_success, report items; routes), `src/server/missions/changes.rs` (C8), `src/db/lessons.rs` (tables, indexes, readers), small changes in the runner, scheduler, listener, continuity, skills, routes, auth and performance modules, and the matching frontend panels.

Where the rails sit: the block builder renders only enumerated fields and person-written text through templates (`lessons::line`, `lessons::safe_text`); the retry tier passes through `router::with_tier` and the existing AutoFix caps; the pin goes through `create_user_skill` or `add_memory_from` with `source='user'` (resolved: `add_memory_from` is the runtime insert path, not the only one; the bundle importer in `src/db/imports.rs` is a second direct INSERT that bypasses its caps, so the pin relies on `add_memory_from`'s own `MemoryError` variants for its 409); every score read is person-only by an `auth::person_only` arm (HEAD counts as GET there); `lessons::effect` is the one function the Insights tile and the auto-pause share. Attribution events are logged in `run_kwant_turn_inner` just before the driver is called, after `flight_school::refuse_turn` and the engine check, once per turn across the failover loop (resolved: a refused or parked turn must not count as "with"; `TurnOpts.on_started` is awaited before routing, so it is the wrong place).

## 4. Components, in rollout order

### C0. Self-check before hand-in

Purpose (resolved: the biggest lever on first-try rate needs no learning). The runner brief tells the bot to attach evidence and call `task_update` review; it never says to run `task_request_check`, which already runs the task's approved checks. Every `check_failed` lesson is a check the bot could have run first.

Change. One sentence in `runner::prompt`: "Before you move it to review, run its approved checks with task_request_check and fix what fails." When a `task_request_check` call by the owner precedes the move to review in the same turn (`turn_tool_calls`), `after_turn` logs `self_checked {checks}` by actor `kwant`. Insights `self_check_rate_7d`; `lessons::line` says "self-checked, then a peer check failed" when both hold.

Rails. No data in the sentence; the checks run through the existing tool and its approvals. Tests: the fake claude calls `task_request_check` then `task_update`; the event exists. Effort: S (half a day, in PR 1).

### C1. Lessons in the runner brief

Purpose. Failures carry across tasks. Every runner brief ends with `[Lessons from earlier tasks]`, derived on read from the bot's own `task_failures` and the person's corrections of the last 30 days, this task's class first.

Trigger. `runner::run_one`, appended after `autofix::retry_note`; `GET /api/kwants/:id/lessons` for the person.

What a lesson is (resolved: structured, keyed on the actual check). A lesson is the tuple (`lesson_key`, newest occurrence, count, settled_as). Two sources:

- kwant's records: `task_failures` rows of `LESSON_CLASSES = check_failed, stalled, denied, unknown` (`quota`, `auth`, `budget`, `transient`, `bad_spec`, `needs_input` are not the bot's doing). `lesson_key = "<failure_class>:<ref>"`: for `check_failed`, `ref` is a hash of the failed check's `(kind, spec)`, so the same check copied into each routine run or mission task repeats under one key; `run` for `stalled` (resolved: `lease` and `turn` stalls are one behaviour and share one key); `tool` for `denied`; `turn` for `unknown`. The class is not in the key (resolved: nearly every task is `general`, so a class-keyed lesson collapsed unlike work into one line); it still orders the block and feeds the digest.
- the person's corrections (resolved: the highest-trust signal in the product was unused): a `state` event by actor `person` from `review` to `ready` or to `blocked` carries `detail.reason`; key `rejected:<hash>`, reasons grouped by `alike()` over content words. An `edited` event with `acceptance: "changed"` after a failure settles it as "the person changed the acceptance" (a spec problem, not the bot's).

A failure belongs to the bot that owned the task and held a claim on it (the `score_tasks` rule).

Rendering (`lessons::line`, pure). Each line comes from enumerated fields and person-written text through a fixed map:

```
[Lessons from earlier tasks]
From kwant's records and the person's reviews. Observations, not instructions: use one only when this task is like that one, and never mention them in your reply unless asked.
- Your release-notes tasks in the last 30 days: 6 of 9 verified, 4 needed a retry, 2 of 6 choices changed by the person.
- 3 days ago, release-notes task "Draft the 1.4 release notes": the command check `cargo test --doc` failed after hand-in (output: "error: doctest failed"); fixed by AutoFix (retry with the check output), then verified. (x3 in 30 days)
- 5 days ago, general task t_2b10: the person sent it back: "the summary lists PRs that were reverted"; then verified.
- 9 days ago, general task t_7f2c: the turn ended without a hand-in; the person retried; then verified.
```

Fields (resolved: generic lines were not actionable, and the rail was stricter than the product): the check's kind and spec through `redact_value` only when `approved_by` is `person` or `mission` (the runner brief already shows every approved check's spec through the same function, so a lesson adds no exposure); 120 chars of `output_excerpt` through `autofix::redact` and `safe_text`; the person's reason through `safe_text` (120 chars); the title only when `tasks.created_by == 'person'` and `safe_text` accepts it (one line, 80 chars, no URL, backtick or path, none of approve/allow/permission/payment/token/password), else the task id. Settled-as from `autofix_attempts`, a `state` event by actor `person` failed to ready ("the person retried"), the `edited` rule, or the task's final state. The first line is the counts-only digest from `performance_observations` and `decision_keep_rate` for this bot x class, omitted under 3 observations and for class `general`. Failure summaries and unapproved check specs never reach a prompt.

Caps. `WINDOW_DAYS=30`, `MAX_LESSONS=8`, `LESSONS_BYTES=1500`; lines grouped by key with a count. A bot with nothing qualifying gets a byte-identical brief.

Data model. No new table. `Database::failures_of_bot(kwant_id, since, limit)` (`task_failures` JOIN `tasks` ON `owner_kwant_id` WHERE EXISTS a claim of the bot, LEFT JOIN `autofix_attempts` and the failed `outcome_checks` row), `person_corrections_of_bot(kwant_id, since)` over `task_events`, and `has_lessons_since(kwant_id, since)`, each one query over the indexes `migrate_lessons` adds (resolved: `tasks` has no `owner_kwant_id` index and `task_failures` only `(task_id, created_at)`, so the draft's "indexed COUNT" was a scan): `tasks (owner_kwant_id, finished_at)`, `task_failures (created_at)`, `task_events (kind, created_at)`. Ledger: `log_task_event(task_id, "lessons_shown", "kwant", {failure_ids, keys, bytes})` from the attribution point.

Settings. `lessons`: `off` | `measure` | `on`, default `off`. `measure` is C7's deliberate without-arm: the block is omitted on every third task-bound turn, chosen by a hash of the task id, so with and without interleave. `lessons::settings_error`; the five `/api/settings` touch points; a `SwitchRow` with a radio in `SettingsModal`, `parseLessonsSettings` returning null on older servers.

Routes. `GET /api/kwants/:id/lessons` returns the lines as JSON plus `effect` (C7). New arm `["", "api", "kwants", _, "lessons", ..] => true` in `auth::person_only` (every method: bots never read their own numbers), an assertion in `person_only_routes`; the handler resolves the kwant through `Scope`.

Insights. `repeat_failures_7d` (failures whose key already occurred for the same bot in the prior 30 days) and `lessons_shown_7d`; `InsightsStrip` tile "Repeated failures"; `eventText` cases `lessons_shown`, `self_checked`.

Rails. Only `autofix::record` writes `task_failures` (append-only by trigger) and only the person writes a rejection reason, so a bot cannot add, edit or remove a lesson; computed on read, never stored; templates and `safe_text` only; no block for hidden-turn purposes (C5's allow-list).

Tests: §10. Docs: §22.1, a row in the §1 Settings table, §6 routes (person-only), §7 live events (new `task_events` kinds), CHANGELOG under `### Added` in `## Unreleased` (resolved: the section exists and is empty; bullets live under `### Added` and `### Changed`).

Effort. M (4 days).

### C5. Lessons in every turn

Purpose. Routine runs, hand-off cards and chat turns never see the runner brief. A smaller class-agnostic block (5 lines, 1000 bytes) goes into `continuity::prompt_block` after the memory block and before `APP_TOOLS_MARKER`, so `TurnInput::drop_app_tools` (which truncates at the marker) keeps it.

Trigger (resolved: an allow-list, not a gate on `None`). `prompt_block` gains `lessons: bool`; `run_kwant_turn_inner` passes `lessons::wants_block(opts.purpose)`, true only for `None` (chat), `mission`, routine and hand-off turns, so `decide`, `audit`, `foundry_draft`, `foundry_replay`, `work_ahead` and any future hidden purpose get none; a unit test enumerates the purpose constants. Gated by `has_lessons_since`.

Data model. None beyond C1. A `lessons_shown` event is logged only when the turn carries `TurnOpts.task` (C3).

Rails and tests. Own section and budget; `MEMORY_PROMPT_BYTES` untouched; the golden fixture included at `tests/truth_ledger_on_test.rs:250` stays byte-identical with `lessons=on` and no failures; a bot with one failure gets the section before `[Your tools for continuity]`; a Foundry draft turn's prompt has no section. Effort. S (1 day).

### C7. Lesson effect and automatic pause (measurement)

Purpose (resolved: the measurement lives in the product). Per bot: verified rate and false-success rate of settled tasks whose brief carried a `lessons_shown` event versus settled tasks of the same bot in the same 30 days without one, each with `performance::wilson`, `low_confidence` under `MIN_N=5`. (resolved) Within one bot every task after the first failure is "with", so the draft compared time, not lessons, and needed 20 settled tasks per bot per month: `lessons=measure` interleaves the arms, and the pause decision pools them per space.

Trigger. On read: `GET /api/kwants/:id/lessons` (`effect`), `GET /api/missions/insights` (`lesson_lift_30d`, `lessons_paused_bots`); after settlement, chained onto the observation (resolved: `record_soon` spawns `record_performance`, so a check spawned beside it reads one task behind): `performance::record_then(state, task_id, |stored| lessons::maybe_pause(..))`.

Pure function. `lessons::effect(observations, shown_task_ids, false_success_ids) -> Effect {with: {n, k, fs, ci}, without: {...}, low_confidence, verdict}`; the panel says it is a trend alarm, not a causal test.

Automatic withdrawal. When both arms have n >= 10 and the "with" verified upper bound is below the "without" lower bound, or the "with" false-success lower bound is above the "without" upper bound, kwant inserts a `lesson_pauses` row (30 days) per bot of the space: the block is omitted, `RuntimeEvent::MemoryChanged {kwant_id}` is sent so the `MemoryPanel` refetches, a `lessons_paused` notification deep-links to the bot's memory tab, and the pause ends by time or by `POST /api/kwants/:id/lessons/resume` (person-only, same arm, through `graph::idempotent`, 409 when no pause is open). A second pause within 90 days suggests turning `lessons` off. Weaker evidence only shows in the panel.

Notifications (resolved: a push body goes through the browser vendor's push service, and today no body carries chat text). Bodies hold counts and the bot name only: "Ada's lessons are paused for 30 days: 4 of 12 verified with lessons, 9 of 12 without". Five touch points for this and every kind the plan adds: `NOTIFY_KINDS`; `KINDS_SINCE_FIRST_LIST` (resolved: a kind missing from it counts as known and is therefore off for every install that ever saved notification settings); the `NotifyKind` union and list in `continuity.ts`; `notifyKindsFor` and `notifyOnToSave`; an `App.tsx` toast and tap branch through a new `openMemoryRef` beside `openSkillsRef` (`Notification` has no link field).

Data model. `lesson_pauses` as in §5, trigger `lesson_pauses_no_delete`. UI: `MemoryPanel` "Lessons" group under the notes (lines, uses, effect, paused state, Resume); `InsightsStrip` tile "Lessons" with lift and a low-confidence badge.

Tests. Unit tests for `effect`, the `measure` interleaving and the pause rule; integration: seeded events and observations trip the pause through `record_then`, the next prompt omits the block, Resume restores it and replays on the same key, a bot token on Resume gets 403; `effective_kinds` covers the new kind. Effort. M (3 days).

### C3. Skill use recording and skill effect

Purpose. Nothing records whether an enabled skill was in a turn or helped. `skills::prompt_block_for` already keeps the `shown` names; it returns the shown skills' ids, and a task-bound turn writes a `skills_in_prompt` task event. A pure `skills::effect` joins those events with `performance_observations`. Report first, with one consumer.

Binding (resolved: `task_of_turn` falls back to any open claim of the bot, so it cannot bind a chat turn safely). `TurnOpts.task: Option<TaskHint {id, class}>`, set by `runner::run_one`, by `listener` for the hand-off turns it starts with a held task, and for routine turns (resolved: routines are the most repeated work and the draft skipped them) by `scheduler::execute`: `start_run` calls `listener::on_routine_run(.., "running")` inline before spawning `execute` (as `autofix::retry_routine` already does), so the task exists when the turn starts and `execute` passes it from `task_by_source("routine", run_id)`. Chat turns carry no hint and log nothing.

Data model. `task_events` kinds `skills_in_prompt {skills: [{id, name, version}]}` and `memories_in_prompt {ids}` (the same shape, so note usage feeds the deferred "Tidy notes" item), actor `kwant`, once per turn; `version` from `skill_meta` for a Foundry lineage, else null (ordinary, imported and user skills have no version). New reader `Database::events_of_kind_since(space, kind, since)` over the `(kind, created_at)` index, space filtered through `tasks`.

Comparison (resolved: "with" tasks were those whose text matched the skill and "without" different work, so the verdict measured task mix). Per skill, bot and `task_class`: with = settled tasks where the skill was shown; without = settled tasks of the same bot and class where the skill was enabled but not shown, plus tasks before `skills.reviewed_at` whose title and acceptance score `skills::relevance >= MIN_RELEVANCE`. An empty arm gives "not comparable yet", never a verdict.

Routes. `GET /api/skills/effect?since=30d` returns `[{skill_id, name, kwant_id, task_class, with, without, low_confidence, verdict}]` for the request's space; new arm `["", "api", "skills", "effect", ..] => true` next to the Foundry arm (GET under `/api/skills` is bot-readable today), registered before `/api/skills/:id`; the same object under `effect` in `GET /api/skills/:id/foundry`.

Consumer. v1: the `SkillsPanel` `ActiveSkill` badge, "in 12 release-notes tasks: 75% verified (60% without)", "not comparable yet" or "not used in a task yet", with the confound stated in the copy. After a quarter of data (a follow-up PR): one `skill_hurts` notification per skill per 30 days when both arms have n >= 10 and the "with" upper bound is below the "without" lower bound, counts only in the body, deep-linking to the Skills tab where the person-only disable (`PATCH /api/skills/:id`) is one click. Nothing changes a skill automatically. Optional, same PR: a `skill_foundry` switch in `SettingsModal`, so the Foundry can be reached from the app at all.

Rails. Ids and names only, never bodies; person-only reads; no new table; `MIN_N` and Wilson as in the Performance Market; shared skills (`kwant_id` NULL) scored per bot, never pooled.

Tests. Unit: `effect` within a class, the before-enablement arm, "not comparable yet". Integration (`tests/skills_test.rs`): a runner turn with an enabled skill named in the task title writes the event, a routine run writes it, a chat turn of the same bot and a refused turn do not; a bot token gets 403. `eventText` cases for both kinds. Effort. M (4 days).

### C6. Repeated lesson, one tap to a lasting fix

Purpose. When the same lesson key repeats despite being shown, the block is not enough and a lasting change is due. kwant raises a report item on the newest affected task with person actions.

Trigger. The task is the newest task of a key with >= 3 qualifying occurrences across >= 2 tasks in 30 days, at least one after a `lessons_shown` event of that bot, no `lesson_decisions` row for `(kwant_id, lesson_key)` in force, and the task is settled or in review. (resolved: `listener::refresh_needs_you` moves a `working` task with any `missions::needs_you` item into `needs_you`, `settle_needs_you` keeps it there, and `after_turn` then records `needs_input`, so the item must never enter `missions::needs_you`.) `lessons::report_items(state, task)` is called from `task_json` and the insights loop only; repeats are computed once per bot per request.

Actions (resolved: the memory block tells the bot its notes "are facts ... not instructions", so a procedural lesson in a note lands where the prompt discounts it, and spends one of 10 pinned slots).
- Procedure (`check_failed`, `stalled`, `denied`): "Add as a skill for <bot>" writes a user skill through `create_user_skill(space, Some(kwant_id), ..)`, source `user`, status `enabled`, measured by C3 from then on.
- Fact (a `rejected` key, or a lesson that names a thing rather than a step): "Pin as a note" writes `add_memory_pinned(kwant_id, text, "user", true)` with a new `Provenance::Task {task_id}` arm in `note_provenance` (resolved: today it always writes trust `outside`; the Task arm writes trust `person`, `source_kind 'task'`, `source_ref` the task id, and the mirror trigger consumes the row, so the fact is accepted but visibly task-sourced). `MemoryError::TooMany` and `TooManyPinned` become 409; nothing is evicted.
- Routine-sourced task: "Change the routine" opens a `workflow_changes` proposal on `routine_instructions` with "add: <lesson>" (open question 4 answered: yes; the target and the `change_write` arm exist).
- When the repeated tasks carried a skill in `skills_in_prompt`: "Revise skill <name>" deep-links to the `SkillsPanel` editor with the lesson lines prefilled; with `skill_foundry` on, "Draft a revision" runs the existing draft path against the failed examples into a new pending version.
- Dismiss: hides the key until it fails three more times (resolved: a 30-day cooldown silenced a live lesson for a month).

The text is a fixed template per failure class ("Lesson (3 release-notes tasks since Oct 12): the command check `cargo test --doc` failed after hand-in. Run the task's checks with task_request_check before you hand in."), never free text. Routes: `POST /api/tasks/:id/lessons/:key/pin` with body `{as: "skill" | "note" | "routine"}` and `POST .../dismiss`, person-only by the existing `/api/tasks` non-GET arm. (resolved: `graph::idempotent` honours an optional `Idempotency-Key`; without the header `f` just runs, and the Discovery dismiss route has no key at all.) The pin route requires the header itself, 400 when missing as the Discovery accept route does; dismiss needs no key, the unique index makes a repeat a 409. Once pinned, the key leaves the derived block; if the person later deletes the skill or note, the line returns.

Data model. `lesson_decisions` as in §5, UNIQUE `(kwant_id, lesson_key)`, trigger `lesson_decisions_person_only`. Task events `lesson_pinned {ref_id, key, as}` and `lesson_dismissed {key}`; `eventText` cases for both.

Frontend. The item is a separate `task.report_items` field, kind `lesson_repeats {id: "<kwant_id>~<key>", text <= 200, kwant_id, count, actions}`, so the `NeedsYouItem` union is untouched; `columnOfTask` puts a task in Needs you when a report item is present (next to `openFlags` and `PERSON_ITEMS`), the insights `needs_you_now` predicate counts it in the same PR, and `TaskDetailBody` renders "Repeated lesson" with the buttons (resolved: those three are the touch points; there is no server-side `PERSON_ITEMS`).

Rails. `source='user'` makes the skill and the note bot-immutable; `RuntimeEvent::MemoryChanged`; one open item per key; undo is deleting the skill or note.

Tests. Unit tests for `repeats` and the settled-only rule; integration: three alike failures with a `lessons_shown` in between raise the item on a settled task while a `working` task of the same key stays `working` and records no `needs_input`; pin as skill writes an enabled `source='user'` skill and the item disappears; pin as note writes a pinned note with a task provenance row; a bot token gets 403; dismiss hides it until three more failures; the Needs-you count matches the column. Effort. M (4 days).

### C8. Before and after for lasting changes

Purpose (resolved: every lasting change in kwant is a person tap, so the loop improves through lasting changes only by telling the person which taps to undo; the draft withdrew only its own automatic adjustments). A pure `changes::effect` over timestamps already stored: `lesson_decisions.decided_at`, `capability_events` grants, `skills.reviewed_at` on enable, `workflow_changes.decided_at` with status `applied`. For the bot x class the change touches: verified, first-try and false-success rates in the 14 days before versus after, with `performance::wilson` and `MIN_N`.

Consumer. A `change_regret` notification (counts only) once per change when both windows have n >= 5 and the after verified upper bound is below the before lower bound, or false success rises the same way, deep-linking to the matching undo: `revoke_capability`, the skill disable, deleting the note or skill, or a prefilled reverse `workflow_changes` proposal. The report sits under `changes` in `GET /api/kwants/:id/lessons` and in the `MemoryPanel` group. The same per-class counts appear as one line under the owner field in the task editor ("release-notes in 30 days: Ada 8 of 9 verified, Bo 2 of 5"), for the person only.

Rails. No automatic undo; the notification joins the five touch points; no new table. Tests: unit tests on seeded windows; a frontend test for the owner line. Effort. S (1.5 days).

### C2. Deep retry: one AutoFix attempt on the deep tier (optional, cut first)

Purpose. The retry that carries the failing check output runs on whatever tier Laya picked and often fails the same way. For mission and person tasks (routine retries are new routine runs made by `autofix::run_due`, never `runner::run_one`), the runner runs the `retry_with_diagnostics` attempt on the engine's deep tier at effort high. (resolved: moved after the spine; it is the only cost-raising component and has the weakest expected effect, since a bigger model rarely fixes a wrong command or a missing file.)

Trigger. `runner::run_one`, when `autofix::retry_tier(state, task, bot)` returns `Some(Tier::Deep)`: the task's `current_attempt` is `running` with strategy `retry_with_diagnostics` (stalled retries stay on Laya's tier), however it was started (`on`, `suggest` then Run, or `run_autofix`), `parse_pin(bot.preferred_model).model` is `None`, no earlier attempt ran deep, the space is not deep-paused, and `autofix_deep_daily_cap` is not reached.

Data model. `autofix_attempts.tier_requested`, `tier_ran`, `model` (pragma-guarded `ALTER`s in `migrate_autofix`). (resolved: `choose_model` falls back to the engine's default when no listed model carries `Tier::Deep`, the usual case for Codex, Grok and custom accounts, and the per-profile failover can move the turn to another engine, so the tier requested is not the tier run.) `tier_ran` and `model` are written after the turn from the `ran_on` meta; the pause rule and Insights count only `tier_ran = 'deep'`. `autofix_finished` detail gains `tier_requested`, `tier_ran`, `tokens_measured` and `skipped_because`.

Prompt and route. `opts.tier = Some(Deep)` goes through `router::with_tier`; effort high for this turn only through `TurnOpts.effort: Option<String>` applied right after `with_tier` (unchanged, so the Auditor and Foundry keep today's effort cap). (resolved: a late Allow resumes the turn with `TurnOpts::routed_as(agent, effort)`, which carries no tier, so the second half of a deep attempt ran balanced on CLI engines.) The saved chat meta gains `tier`, and `routed_as` becomes `resumed_from(meta)`, restoring `routed`, `tier` and `effort` together. `retry_note` says "(deep tier)". The Flight School guards are unaffected: the family never changes.

Settings. `autofix_retry_tier`: `same` | `deep`, default `same`; `autofix_deep_daily_cap`: 0 to 50, default 5 (resolved: the AutoFix caps are per task); validated in `autofix::settings_error`, the five touch points, a radio and a number inside the AutoFix `SwitchRow`, hint "inert for engines without a deep-tier model".

Rails. Bounded by the AutoFix caps, mission budgets and the daily cap; one deep attempt per task. Automatic withdrawal, strategy-matched and pooled per space (resolved: "the same-tier figure" mixed every strategy, and 5 measured attempts per bot never arrive): over 30 days, when deep `retry_with_diagnostics` attempts with measured tokens number at least 5, cost more than 1.25 times the same-tier figure per success, and do not succeed more often, `retry_tier` returns `None` for the space (`deep_paused`). Attempts without usage rows (Codex and Grok CLI write none) count as `unknown`, never zero, and are excluded. Insights: `autofix_retries_30d: {deep: {attempts, succeeded, measured, tokens}, same: {...}, deep_paused, deep_cap_hits}`.

Tests. `tests/mission_autofix_test.rs` (resolved: its own fake records only `{task, prompt}` lines, so the PR extends it to record argv): the second invocation after a failed command check carries `--effort high` and the deep-tier `--model`, the first neither; a pinned-model bot keeps its model; a stalled retry stays; the suggest-then-run path goes deep; the detail has `tier_ran: "deep"`; a Codex-engine case records `tier_ran: "balanced"` when no deep model is listed; `known_models` in the harness yields a `Tier::Deep` alias. `tests/late_approval_test.rs`: deep retry, card, Allow later, the resumed invocation still carries the deep `--model`. Unit tests for `retry_tier`, the pause rule and the cap. Effort. M (3 days), plus docs §13 and §1 rows.

### C4. Hand-backs, report first (learning deferred)

(resolved: both critics ranked the learning "drop first": a "Wrong bot?" tap already achieves a message-specific exclusion, and the sticky-question path at `routes.rs:2990` routes through `route_channel_query_after` without `Hints`, so "the person's corrections always win" does not hold there today.) This plan ships the report half only: `handbacks: [{kwant_id, n_30d}]` from `route_handbacks` in `GET /api/performance` and a line in `PerformancePanel`. (resolved: the draft called the tool one for messages "routed by words"; it is offered for any channel turn at depth 0 outside threads and accepted for any routed basis, keywords, skills, learned, lead, last, concierge, several, only, asked or a prior hand-back; an @mention or explicit pick carries no routed message id and is refused.) Effort S. The learning (`Hints.handed_back` through `avoided()`) is in "Deferred".

### Phase 1 fix: attribution by context (S, no setting)

(resolved: the plan's token metrics read `performance_observations.fresh_tokens`, which `observe_task` sums from every `model_usage` row of the bot inside the claim's time window, so two running missions or a chat turn during a claim corrupt it today; the draft measured with a broken scale and fixed it last.) `observe_task` prefers rows whose `context_id` equals the claim's, falling back to the time window only when none match. A unit test with an interleaved chat row.

### Phase 4 (after the four weeks): attribution foundation

Purpose. Any later model or effort prior needs attribution that does not exist: `performance_observations.engine` is always `kwants.driver_type` (`api` from every in-repo creation path), `model` falls back to `preferred_model` (`auto`), cost exists only for Claude CLI turns.

Scope. Pragma-guarded `ALTER`s on `model_usage`: `turn_id`, `tier`, `effort`, written by `record_cli_usage` and `record_call` from new `TurnInput` fields; Codex and Grok CLI turns get a `model_usage` row with `reported = 0` from the CLI's own model and effort flags, so tier and model are known even when tokens are not (resolved: the columns alone left those engines without rows); `observe_task` matches `turn_id` when set; `performance_observations` gains nullable `tier`, `effort`; `GET /api/performance?by=tier`; `UPDATE kwants SET turns_completed = turns_completed + 1` from the turn-end guard (`level` stays untouched). No behaviour change, no setting. Effort M.

## 5. Data model summary

| object | change |
|---|---|
| `autofix_attempts` | `tier_requested`, `tier_ran`, `model` TEXT (NULL = same as Laya) |
| `lesson_decisions` (new, `src/db/lessons.rs`) | `id, space_id, kwant_id, lesson_key, decision CHECK ('skill','note','routine','dismissed'), ref_id, decided_by CHECK ('person'), decided_at, until_failures, task_id`; UNIQUE `(kwant_id, lesson_key)`; trigger `lesson_decisions_person_only` |
| `lesson_pauses` (new) | `id, space_id, kwant_id, until, reason, created_at, ended_at, ended_by CHECK ('person','time')`; trigger `lesson_pauses_no_delete` |
| indexes (`migrate_lessons`) | `tasks (owner_kwant_id, finished_at)`, `task_failures (created_at)`, `task_events (kind, created_at)` |
| `fact_provenance` | new `Provenance::Task {task_id}` arm in `note_provenance`: trust `person`, `source_kind 'task'`, `source_ref` task id |
| `task_events` kinds | `self_checked`, `lessons_shown`, `skills_in_prompt`, `memories_in_prompt`, `lesson_pinned`, `lesson_dismissed`; `autofix_finished` detail gains `tier_requested`, `tier_ran`, `tokens_measured`, `skipped_because` |
| settings (app-wide) | `lessons` off/measure/on, `autofix_retry_tier` same/deep, `autofix_deep_daily_cap` 0 to 50 |
| task JSON | `report_items` (kind `lesson_repeats`) beside `needs_you` |
| notify kinds | `lessons_paused`, `change_regret`, later `skill_hurts`; each through the five touch points |
| routes (person-only) | `GET /api/kwants/:id/lessons`, `POST /api/kwants/:id/lessons/resume`, `GET /api/skills/effect`, `POST /api/tasks/:id/lessons/:key/pin` and `.../dismiss` |
| Phase 4 | `model_usage.turn_id, tier, effort`; `performance_observations.tier, effort`; `kwants.turns_completed` written |

`migrate_lessons` is called from `src/db/mod.rs` after `migrate_automation_discovery` and before `migrate_truth_ledger`, which stays last. Columns follow the chain's idiom (resolved: a `pragma_table_info` check, then `ALTER TABLE ... ADD COLUMN` with the error propagated, not the ignored `ALTER`s the draft named), tables and triggers `CREATE ... IF NOT EXISTS`.

## 6. Rollout

| phase | delivers | depends on | cut line |
|---|---|---|---|
| 1 (days 1 to 6): the fixing loop learns | C0 and C1 with the `lessons` setting, the route, Insights, docs, CHANGELOG (PR 1); the `observe_task` context fix (PR 2) | none | C0 alone is still worth shipping; C1 is the spine |
| 2 (days 7 to 13): every turn learns, measured | C5 continuity block (PR 3); C7 effect, pause, `lessons_paused` (PR 4); C3 skill and memory use, effect, badge (PR 5) | C1 | `memories_in_prompt` and the Foundry `effect` field can slip |
| 3 (days 14 to 21): repeats become proposals, lasting changes measured | C6 (PR 6); C8 change regret and the owner line (PR 7); C4 report line (PR 8); day 21 metrics check-in | C1, C5, C3 | drop C6's "Draft a revision" first, then C8's owner line, then C4 |
| later | C2 deep retry when the AutoFix retry numbers justify it; Phase 4 attribution; the deferred list | phase 1 numbers | whole items |

Total about 21 maintainer-days for phases 1 to 3 (resolved: the spine said 12.5; each M component carries a migration or DB reader, a person-only arm with its test, frontend work, docs and an integration test in its own binary); C2 is 3 more.

Everything works with Flight School, the Foundry, the Auditor and the Truth Ledger off: lessons read `task_failures` (written by AutoFix in every mode, including `off`) and the person's review events; skill effect works for any enabled skill; the pinned note mirrors into `knowledge_facts` in every ledger mode. With AutoFix `off` "fixed by AutoFix" never appears, but the block, the digest, the rejection and the person-retried lines still do.

Deferred, with the unlock: hand-backs as a routing exclusion (unlock: `hints_for` built before the sticky check so `route_channel_query_after` takes `Hints`; rows with NULL `text` never learned); a class suggester (a pure `classes::suggest` clustering titles by `content_words` against the space's classes, offered as a one-tap chip on task create); `lessons_reflect` (at most one hidden turn per bot per day, only on a new qualifying failure, reusing the Foundry's held-tools and cap pattern, whose only output is a tool call `lesson_note {key: an existing key, text <= 120 chars through safe_text}` shown as the line for that key and never written to memory; unlock: the template lines measured useless); a "Tidy notes" item from `memories_in_prompt` (notes never shown in 90 days or near-duplicates via `similar_note`, Merge or Delete taps); a model or effort prior for `auto` bots (Phase 4 plus 4 weeks of data); automatic tier review; skill auto-disable; audit risks as lesson lines; Laya outcome feedback (nothing to build).

## 7. Metrics and automatic rollback criteria

| metric | source | target |
|---|---|---|
| `self_check_rate_7d` | `self_checked` events over hand-ins | above 80% within 2 weeks of C0 |
| `repeat_failures_7d` | `task_failures` keys repeating within 30 days per bot | halves within 4 weeks of C1 and C5 for bots with >= 10 settled tasks |
| first-try rate per bot (per class where classes exist) | `score_tasks` / `card_of` rule | rises after C0 and C1 (14 days before vs after) |
| `lesson_lift_30d` | `lessons::effect` with `measure` interleaving, pooled per space | "with" lower bound above "without" point estimate once n >= 10 each; the same function drives the pause |
| false success per bot x class | `lessons::false_success` | does not rise after any component; a rise pauses lessons (C7) |
| `autofix_retries_30d` | `autofix_attempts` by `tier_ran`, strategy-matched | deep success rate above same; tokens per success within 1.25x |
| skill effect | `skills_in_prompt` x `performance_observations` within a class | count of skills with "with" lower bound above "without" upper bound |
| lasting changes | `changes::effect` | no regretted change left undone past 14 days |
| hand-backs and corrections per 100 auto-routed messages | `route_handbacks`, `route_examples` | reported; the unlock for the deferred learning |
| `lesson_repeats` opened vs pinned vs dismissed per week | `lesson_pinned`, `lesson_dismissed` events | dismiss rate above 50% means the repeat rule is too loose |
| `interventions_7d`, `needs_you_now`, `tokens_per_verified_7d`, `cost_per_verified_7d_usd` | existing Insights, after the `observe_task` fix | must not rise |

Automatic rollback criteria, each written to a ledger row and a notification: lessons pause 30 days for the space's bots when "with" verified upper < "without" lower, or "with" false-success lower > "without" upper, at n >= 10 per arm (C7); deep retries pause for the space when deep tokens per success exceed 1.25x same at n >= 5 measured with no higher success rate, and stop for the day at the cap (C2). Lasting changes are never withdrawn automatically; C8 tells the person which one to undo.

## 8. Risks and mitigations

- Wrong or stale lessons mislead a bot: `bad_spec` is excluded, an acceptance edit settles the failure as the person's, each line says how it settled, the window is 30 days, and C7 pauses the block when the numbers fall.
- Free text from tool output reaching prompts: only enumerated fields, person-written text through `safe_text`, person-approved check specs through `redact_value` and 120 chars of redacted check output are rendered; failure summaries stay in the AutoFix retry note.
- Prompt bytes: 1500 (brief) plus 1000 (continuity); own budgets, omitted when empty, byte-identical prompts proven by the golden fixture.
- Deep retries raise cost: `same` by default, one per task, a daily cap, `tier_ran` measured, a strategy-matched pause per space.
- Attribution noise and selection bias: events logged only once the driver is about to run, task-bound turns only; `measure` interleaves the lesson arms; the skill comparison stays within a class and says "not comparable yet" when an arm is empty; both panels call it a trend alarm.
- A report item parking a working task: `lesson_repeats` never enters `missions::needs_you` and attaches to settled or in-review tasks only; a test asserts a `working` task stays `working`.
- Notifications silently off or landing in chat: every new kind joins `KINDS_SINCE_FIRST_LIST`, the frontend lists and an `App.tsx` branch; bodies carry counts and the bot name only.
- SQLite contention, scans and races: single-statement `log_task_event` inserts or `begin_write` transactions; readers hit the three new indexes; repeats computed once per bot per request; the pause rule runs after the observation row is stored.
- Needs-you drift and test flakiness: the `report_items` column rule and the `needs_you_now` predicate change in one PR with a test asserting the count; pure functions carry the logic, fake-claude tests only prove the wiring and run in their own binaries.

## 9. Open questions for the maintainer

1. Defaults: every switch ships off (`same`). Is a one-line "Turn on lessons" prompt in the Mission board empty state wanted?
2. Should lessons be per bot only (v1) or include teammates' failures on the same check ("team lessons")?
3. `measure` omits the block on one task-bound turn in three. Is a third right, and should `on` keep a one-in-ten holdout so the pause rule keeps data later?
4. C6 pins a procedure as a user skill and a fact as a note. Is the split right, or should every lesson be a skill?
5. Is C2 wanted in the first month at all, given the daily cap, the `tier_ran` bookkeeping and the late-approval change it needs?
6. Phase 4 attribution: schedule it as week 5 so a model or effort prior becomes possible, or decide that Laya stays the router for good and skip it? And `kwants.level`: drop it later, or leave it?
7. The class suggester is deferred; is a one-tap chip on task create worth an S now, since every per-class number in this plan is empty while tasks stay `general`?

## 10. The first PR: self-check and lessons in the runner brief (C0 and C1)

Scope. The self-check sentence and `self_checked` event; the `lessons` setting; `lessons::{block, line, lesson_key, safe_text, wants_block}` as pure functions; the three DB readers and `migrate_lessons` with the three indexes; `TurnOpts.task: Option<TaskHint>` set by `runner::run_one` and the `lessons_shown` write at the attribution point (the field C3 and C5 build on); the block appended after `autofix::retry_note`; `GET /api/kwants/:id/lessons` with the `person_only` arm and `Scope` resolution; the Insights fields and tile; the `SettingsModal` `SwitchRow` and radio; `eventText` cases; docs §22.0 and §22.1 plus the §1, §6 and §7 entries; CHANGELOG under `### Added` in `## Unreleased`: "**Lessons (off by default).**"

Files. `src/server/missions/lessons.rs` and `src/db/lessons.rs` (new), `src/db/mod.rs` (migrate chain), `src/server/missions/runner.rs` (`prompt`, `run_one`, `after_turn`), `src/server/routes.rs` (`TurnOpts.task`, the attribution write, the settings structs, validation and write block), `src/server/missions/routes.rs` (insights), `src/server/mod.rs` and `src/server/auth.rs`, `frontend/src/missions.ts`, `SettingsModal.tsx`, `MissionBoardView.tsx`, `docs/mission-control.md`, `CHANGELOG.md`.

Tests. Unit: `block` ordering, caps, grouping, class filter, title and reason rules, spec only for approved checks, lease and turn stalls sharing a key, `measure` omitting one turn in three by task id, `settings_error`. Integration (`tests/lessons_test.rs`, its own binary, `mod common`): a command check fails on task A and the person rejects task B with a reason; task C's recorded prompt for the same bot carries both lines and the self-check sentence, and `lessons_shown` plus `self_checked` events exist; with the fake claude off PATH no `lessons_shown` event is written; a bot with no failures gets a byte-identical prompt; a bot token gets 403; another space's kwant gets 404. Frontend: `parseLessonsSettings` returns null on an older server.

## 11. After the core: the ladder, gated by evidence

The core above closes the loop with no hidden turns and no machine-written memory. It is also what makes the rest possible: once lessons, skill use and lasting changes are measured (C7, C3, C8), kwant has baselines, and a baseline is what lets a change apply itself and withdraw itself. This section is the road after the core, taken from the four designs the judges ranked below the spine and ordered by the evidence that unlocks each piece. None of it is in the three-week plan, and none ships on by default.

### The rung map

| rung | what applies | who decides | rail | unlocked by |
|---|---|---|---|---|
| 0 | attribution rows (F1): which engine, tier, effort, skills and hints each turn ran with, and a cost estimate for every engine | nobody, it is bookkeeping | no prompt bytes, no network, one INSERT per turn | the core's Phase 4 |
| 1 | the core's automatic parts: the self-check sentence, the derived lessons block, later the deep retry | kwant, within caps | fixed text, windows, templates, auto-pause | shipped above |
| 2 | self-withdrawing changes under a canary: hand-backs as a routing exclusion (F0), a model or effort prior per bot x class (F3), `[How you work best]` hints (F4), the skill gardener (F7) | kwant in `self_improve = on`; the person in `suggest` | the canary judge (F2): baseline snapshot, cohort, rollback at n >= 5, freeze on a denial or false success, Undo card | F1 plus 4 weeks of data; a month in `suggest` first |
| 3 | lasting changes with evidence attached: tier down, persona append, engine pin or switch, skill versions, hires (F8) | the person, always | `decided_by = 'person'` triggers, compare-and-set on `before`, 30-day cooldown after a decline | a kept rung-2 canary, or a scorecard rule |

The rails from the red team bound every rung: nothing adds a path to `verified`; signals come from ledgers, never reply text; machine-written prompt text is rendered from fields through fixed templates into its own labelled section with its own byte budget; every automatic change has an append-only ledger row, an SSE event with `space_id`, a notify kind and an undo path before it ships; no hidden turn without a purpose, a timeout, a fresh-token watcher, a daily cap, a run row with compare-and-set moves, a restart sweep and a Drop guard; nothing new leaves the machine; space is a hard boundary; the person's explicit choices (an @mention, a correction, a pinned note, an `avoids` entry, a pinned model) always outrank a learned one.

### F0. Hand-backs as a routing exclusion (rung 2, M)

The learning half of C4. `route_handbacks` gains `text` (clipped to 500 chars) and `space_id`; `Database::handbacks_for_learning(space_id, since, per_bot_limit)` returns the last 30 days' hand-backs whose message has no `route_examples` row (a correction or an ask-card pick means the person decided), newest 10 per bot, rows with NULL `text` never. `Hints.handed_back` extends `avoided()` in `pick_by_words`: a bot is avoided for this message when one of its learned hand-backs is alike (two shared content words and overlap at or above the learned threshold), so it enters the scoring before the winner is computed, the tie branch never lists it, and naming the bot still wins because the @mention and name branches return earlier. `Pick.reason` says "<bot> handed a message like this back on <date>". Unlock, from the core: `hints_for` built before the sticky-question check so `route_channel_query_after` takes `Hints`, and the C4 report line showing hand-backs that repeat. Rollback paths: a "Wrong bot?" correction on that message, expiry, the setting `routing_learns_handbacks`. Laya's payload stays byte-identical.

### F1. Attribution ledger and a local price book (rung 0, M)

One `turn_runs` row per turn (turn id, bot, space, task id when claimed, task class, purpose, engine family, model, tier, effort, `picked_by` laya|pinned|prior|bandit, skills `[id@version]`, hint ids, fresh tokens, cost), written where `meta.ran_on` is built at the end of `run_kwant_turn_inner` and by the hidden-turn drivers in `foundry.rs` and `auditor.rs`. `observe_task` then takes model, engine and cost from the turn rows inside the claim windows instead of a time window, which fixes `engine = 'api'` and `model = 'auto'` in `performance_observations` for good (the core's Phase 1 fix narrows the window by `context_id`; this replaces it) and stops a chat turn beside a hidden run being billed to the run (`tokens_between` sums every row of the bot in the window). A static price table in the binary (USD per million tokens by model prefix; unknown stays NULL) fills `cost_usd` for API, Codex and Grok rows as an estimate marked `cost_source = 'estimated'`, so Insights can show how much of a cost figure is a receipt and how much a guess. `kwants.turns_completed` gets its first writer in the same transaction. No setting, no behaviour change.

### F2. The canary judge and the `self_improve` setting (rung 2 rail, L)

`self_improve`: `off` (default) | `suggest` | `on`, the AutoFix shape. A `self_changes` ledger (target, target id, before, after, reason, baseline JSON, status `proposed | applied | canary | kept | rolled_back | stale | frozen`, decided_by) with an append-only `self_change_events` table. On apply, the baseline is the key's last 30 days from `performance_observations` joined to `turn_runs`: n, verified rate, tokens and cost per verified, interventions per task, denials, false successes. `judge::tick` runs hourly and once more after settlement, chained onto the observation the way C7's pause is (`performance::record_then`), when a settled task belongs to a live cohort; `judge::verdict(baseline, cohort, now)` is a pure function with unit tests like `card_of`. Rollback rules in order: any denied external call or false success in the cohort rolls back now and freezes the bot (all its rung-2 rows roll back, the bot behaves as `suggest` for 30 days, one notification); n >= 5 and the cohort's Wilson upper bound below the baseline rate; n >= 5 and cost or tokens per verified above 1.25x; the window ends with n >= 5 and nothing fired: kept; the window ends under n = 5: extended once, then kept with `low_confidence`. One live canary per bot. Reverts are compare-and-set writes; a target the person edited meanwhile goes `stale`, never clobbered. The judge never calls `transition()` and never takes `listener::SERIAL`. Insights gain `self_improve_7d {applied, kept, rolled_back, proposals_open, frozen_bots}`; an Improvements panel next to Tier requests lists Proposed, In canary, Kept and Rolled back with Undo. The same judge can later carry the narrow slice of workflow changes AutoFix only proposes today (a check inside a routine made optional, a `max_attempts` bump), applied under a canary and reverted by compare-and-set; it is listed here, not specified.

### F3. Model and effort prior per bot x class (rung 2, M)

`router::with_prior(route, prior)` runs in `run_kwant_turn_inner` after `apply_preferred_model` and before `with_tier`, skipped when `opts.routed` or `opts.tier` is set, for runner, routine and hand-off turns only (chat turns have no class). A prior is `{tier, effort}` per (bot, task class) and sets `route.tier` and `route.effort_level` only: never a model, never a family. With `auto` it acts only when 80% or more of the bot's turns in that class ran on one family. The judge proposes a prior when, over 30 days, a (tier, effort) group with n >= 5 has a Wilson lower bound above the current group's verified rate, or the same rate at 70% or less of the tokens per verified. Shadow first: a `routing_bandit = shadow` month logs "would have picked" on `turn_runs` while Laya's route runs, and the Performance panel shows how often it would differ and the counterfactual cost. Laya's payload stays byte-identical, so its test stays green and the sidecar learns nothing. `meta.ran_on.picked_by = 'prior'` makes the cohort exact. This relaxes "nothing routes on performance" for task turns of `auto` bots only, within the family, with a pinned model always winning; it is the open question the maintainer has to answer before F3 is built.

### F4. Prompt hints: `[How you work best]` (rung 2) and persona append (rung 3) (M)

When the same lesson key repeats (the C6 rule) and the person has not pinned a fix, a standing hint is the next step up from the derived line. A hint is at most 3 lines and 300 characters per (bot, class or `*`), stored as a `self_changes` row with target `prompt_hint`, injected by `continuity::prompt_block` after the lessons section and before `APP_TOOLS_MARKER`, and into the runner brief for the class, with its own 1 KiB budget and at most 3 live hints. The wording comes from the bot (through the sealed reflection turn, F5, with a one-shot `hint_propose` tool) or from a template when F5 is not built; the recurrence rule is checked server-side from `task_failures`, never from the bot's text; the text passes `autofix::redact`, the memory text cleaner, and a deny-list (no "approve", "allow", "password", "key", no URL) so a hint can never carry a permission. It applies under a canary measured on the class's verified rate through `turn_runs.hints`. A hint kept through two windows with 10 or more verified tasks files a rung-3 `persona_append` proposal applied by compare-and-set on `persona_prompt == before` and recorded as a `bot_actions` `bot_update` row, so the existing Undo arm covers it. `persona_prompt` and `soul` are never written on rung 2.

### F5. The sealed reflection turn (`lessons = full`, L)

The core defers `lessons_reflect`, one hidden turn per bot per day whose only output is a `lesson_note` line for an existing key. This is what it grows into. For a task with a story (a failure row, a person intervention, a failed peer review, a false success, an open audit risk, or attempts >= 2) one short hidden turn of the owner reflects like a colleague and calls `lesson_add` once. New `TurnOpts.own_records_only`: the approvers refuse with text, never a card, every tool outside the read-only self tools plus `lesson_add` and `lesson_retire`, including the engine's own Read, Bash and WebFetch and every MCP server but kwant's own. The turn can therefore not read outside text and never waits on a card, so the start-of-turn taint for always-ask turns is skipped and the lesson lands with trust `bot`; if an outside tool is even attempted the run is marked tainted and the lesson becomes `outside`/proposed. Context is the task's own records only (title, acceptance, checks with redacted specs, the failure chain, AutoFix strategies, denied calls, the reviewer's note, the C1 line it should refine, the three nearest live lessons to answer "same as" instead of adding), not the direct chat's last 20 messages. Caps: fast tier by default (`lessons_model`), 5 minutes, 20,000 fresh tokens by `usage_of`, a daily cap counted since UTC midnight, one at a time server-wide, a `lesson_runs` row with compare-and-set moves, a migrate sweep to `interrupted` and `resume_after_restart` in `serve.rs`. Runs only where kwant gates every call (Claude CLI with the hook, API drivers); Codex and Grok CLI log `retro_skipped {engine_cant_seal}` and the derived line stands. Lessons written this way live in their own table with their own cap and section, never against the 100-note or 6 KiB memory limits, and C7's effect and pause cover them like the derived lines. The taint exemption for a sealed turn is a decision for the maintainer; without it, model-written lessons are always unconfirmed and only the derived lines carry trust.

### F6. More work becomes verifiable (automatic, M to L)

- Class Lens. The core defers a one-tap class chip on task create; this is the automatic version. A pure classifier gives every task a `work_class`: the person's class when set, else a class learned from the person's own `class_changed` corrections (Jaccard >= 0.6 of `automation_discovery::norm(title)` words against a 200-row `class_examples` table), else `t-<6 hex of sha256(norm(title))>` (the Automation Discovery `cluster_key` rule), else `general` when the title is too vague; routine runs get `r-<routine_id>`. It never writes `tasks.task_class`, so Flight School tiers never move on an inferred class. The Foundry miner groups by `work_class` with the `task_class <> 'general'` predicate removed, so it finally fires on real workloads, and every per-class number in the core stops being empty.
- Skill binding with skill-carried checks. On claim, the owner's enabled Foundry skill for the work class is bound: first in `[Skills]`, named in the brief, and its `skill_meta.checks` instantiated as `outcome_checks` with `skill_id` provenance. File and public-http checks are approved because the person approved them once at skill approval; command checks only when the person ticked "also approve its checks". Placeholders substitute a strict character class; anything else falls back to `needs_approval`.
- Checks First. A non-owner teammate gets one short always-ask read-only turn in the task folder to propose file and public-http checks with `task_request_check` for tasks that have no runnable check and no bound skill; the existing non-owner rule makes the task verifiable with no person action. Daily cap, one run per task, the challenger picker from `auditor.rs` extracted into a shared `pick_reviewer`.
- Auditor verdicts. A challenge that raises zero risks and cites evidence is recorded as a passing `peer_review` check through a shared `record_peer_review`, so the Auditor stops throwing its result away. This changes the §18 promise that an audit never changes verification, and is an opt-in `auditor_verdicts` setting behind an open question.

### F7. The skill loop closes both ways (M to L)

- Gardener (rung 2). Nightly, for every enabled Foundry skill with a confident C3 card: roll back to the previous version when it measures confidently better; disable when the skill is confidently worse than the bot's baseline without it, or has 3 or more `check_failed` failures on bound tasks and zero verified. Both actions only narrow what reaches a prompt, write a `bot_actions` card with the numbers and an Undo arm, and are bounded (3 per day, one per lineage per 14 days, 90 days of peace after the person re-enables). Approve, reject, edit, enable and disable start writing `skill_events` so the gardener can read the person's decisions. This is the automatic form of C3's `skill_hurts` notification.
- Refinery (rung 3, opt-in canary). When 2 or more bound tasks of a skill fail their checks in 14 days, the owner gets one hidden draft turn with the body, the failing tasks' redacted check output and the verified examples, and `skill_draft` produces version n+1 through the existing lineage path; the dry-run replay runs on the failing examples first. Default: a pending skill the person approves with the diff and the replay report (the automatic form of C6's "Draft a revision" tap). With `skill_foundry_autoversion = canary`, a body-only diff whose replay passed a kwant file check on a task that failed before goes live at once and the gardener re-judges it after 5 bound settles.

### F8. Rung 3 with evidence (M)

The person keeps every high-risk decision but stops deciding blind: a `tier_down` proposal when a class scorecard shows 2 or more denials in the last 10 tasks, a false success, or a freeze (approve calls the existing Flight School policy write with `granted_by = 'person'`); `persona_append` from F4; `engine_pin` after a prior is kept 30 days with n >= 10, `engine_switch` when a cross-family candidate beats the current rate with n >= 10; hire cards from the Concierge carry the ledger's figures (hand-back count, the bouncing bot's verified rate). All appear in the Improvements panel and, when a task is involved, as a report item beside `needs_you` the way C6's `lesson_repeats` does, never inside `missions::needs_you`.

### F9. Hygiene and the mission retrospective (L)

The core defers a "Tidy notes" item built on `memories_in_prompt`; this is the automatic form. A nightly deterministic pass expires lessons older than 90 days with no use in 60, merges near-duplicates (Jaccard >= 0.6, the `similar_note` helper lifted into a shared function), retires lessons that hurt 3 or more times and never helped, and keeps 60 live per bot; every move is restorable. A memory tidy turn (sealed, at most once per 7 days per bot, only at 90 of 100 notes or 2 or more duplicate agent-written pairs) merges or drops the bot's own unpinned notes with `memory_update` and `memory_remove`, each change a `bot_actions` row with an Undo card and a `memory_tidy` undo arm; person-written, imported and person-pinned notes are never touched. When a mission goes `done`, a pure `mission_digest` (tasks, first-try verified, failures by class x bot, AutoFix attempts, interventions, blocked minutes, tokens, cost, slowest task, open risks) is posted as a chat line, and, under `lessons = full`, one sealed turn of the lead writes shared lessons (`kwant_id` NULL, 40 per space, labelled "shared by <lead>") and may file proposals only through the existing person-decided tables (`skill_propose`, `workflow_propose` on routine targets).

### Unlock gates

| item | unlocked by |
|---|---|
| F0 hand-back learning | the C4 report line showing hand-backs that repeat; `hints_for` built before the sticky check |
| F1 attribution | the core's Phase 4; no gate of its own |
| F2 judge and `self_improve` | F1 and 4 weeks of attributed observations; a month in `suggest` |
| F3 prior | F2; a month of `routing_bandit = shadow` showing a cost delta or a rate gap the maintainer accepts; the open question answered |
| F4 hints | C6 repeats data; F5 for bot-written wording, or templates without it |
| F5 sealed reflection | the core's `lessons_reflect` measured useful; the taint decision |
| F6 class lens, binding, checks first, verdicts | class lens needs nothing; binding needs the Foundry on and one approved skill with checks; checks first needs the Auditor's picker extracted; verdicts need the §18 decision |
| F7 gardener, refinery | C3 skill effect with n >= 10 per skill; `skill_events` written by the person's routes |
| F8 rung 3 | F2 kept canaries, or Flight School on for tier-down |
| F9 hygiene, retrospective | F5 |

### Open questions the ladder adds

1. May `auto` bots ever get a cross-family prior automatically, or is a family change always a rung-3 proposal with Wilson evidence? The designs say proposal.
2. Is a sealed `own_records_only` turn exempt from the start-of-turn taint, or must every kwant-started turn stay `outside`?
3. May a clean, evidence-cited Auditor challenge count as the peer review that verifies, changing §18, or should verdicts come from a separate reviewer turn?
4. May an owner-requested check count when it equals a person-approved skill template with substituted inputs?
5. One live canary per bot (clean attribution, one change per window) or per (bot, target) (twice the rate, confounded cohorts)?
6. Shared lessons from a mission lead: every bot in the space sees another bot's task details, or a person-approved promotion like `promote_fact`?
7. Thresholds (MIN_N 5, cost ratio 1.25, canary 7 days, freeze 30 days, 3 hints, 60 lessons): constants or settings? The designs make only the canary days and the daily caps settings.
8. Defaults after the ladder lands: `off` like every Phase D feature, with the docs recommending a month of `suggest`.

## 12. How this plan was made

Eight readers mapped the subsystems the plan touches (hidden turns and the scheduler; settings, Needs-you, SSE and the ledgers; memory and the Truth Ledger; skills and the Foundry; the task lifecycle, checks and scorecards; routing, models and usage; the web app; tests and conventions), citing file and line for every fact. Five designs were written from different angles (the autonomy ladder, closed-loop measurement, reflection and memory, procedures and verifiability, and the smallest set of changes with the biggest payoff), and a red team listed 24 threats with the rail for each and 17 things the feature must never do. Three judges scored every design for fit, payoff and safety; the pragmatic design scored highest (7.0 of 10 on every lens, against 5.7 to 6.3 for the others) and became the spine of sections 1 to 10, with the judges' "worth stealing" items grafted in. The synthesized plan listed 68 claims about the existing code; an agent read the code to refute each one, 3 were refuted and corrected (the hand-back tool's trigger, the idempotency helper's behaviour, the memory insert paths), and two critics (coverage of self-improvement mechanisms; operational completeness) added the self-check step, the person's corrections as a lesson source, the measurement arm, the regret detector, the indexes, the notification touch points and the attribution fix, after which the plan was revised. Section 11 is the rest of the ladder, taken from the other four designs and ordered by what unlocks each piece; every code name it uses was checked to exist.
