# Zombie Mission Hunter

"Find anything important I started but never finished." kwant reads the person's own
conversations, read-only, and keeps a short queue of stalled cases. Each case has the evidence
(a verbatim quote) and a drafted next action. kwant never sends anything: the person copies the
draft or opens it in their mail app.

Code: `src/zombie/` (sources, rules, model check, drafts, scan and review), `src/db/zombie.rs`
(case store), `src/server/zombie_routes.rs` (API). It runs the same on the VPS and the Mac: it
needs no Docker and no system packages.

## Sources

Every source implements `MissionSource` (`src/zombie/mod.rs`) and only reads.

| Source | What it reads | How it stays read-only |
|---|---|---|
| `kwant` (built in) | kwant's own threads, open questions to the person, and failed delegations | Database reads only |
| `imap` | INBOX and the Sent folder (or the configured `folders`), up to `lookback_days` back | `EXAMINE` (never `SELECT`), `BODY.PEEK[]`, and no `STORE`/`EXPUNGE`/`MOVE`. The e2e test compares every message's flags before and after a scan. |
| `github` | Issues and PRs the token's user opened or is assigned to, with their comments | `GET` requests only |

Logins live in the password vault (`CredentialStore`, owner `kwnt-zombie-hunter`). They are
never stored in config, env, argv or logs, and the API never echoes them. A source is tested
before it is saved, and nothing is stored when the test fails. Deleting a source deletes its
secret, its cases and its mutes. A plain-text IMAP connection or a self-signed certificate is
only accepted for a server on this computer (Proton Bridge, a test server).

Like any saved password (see `saved-passwords.md`), a login is refused with 409
`{needs_bitwarden: true}` on the built-in store while approvals are off anywhere, before the
connection test runs. Existing logins count toward `local_count` and move with "Move to Bitwarden".

The case store is not tied to mail: `source`, `conv_kind`, `kind`, `evidence` and `deadline` fit
any open loop, and source kinds are checked in Rust, not by the table, so later sources (a
photographed letter, a bank statement) need no table rebuild.

## Finding cases

1. **Rules** (`detect.rs`) look at each conversation's last messages and decide whether someone
   is still waiting:
   - `awaiting_reply`: you asked something and nobody answered.
   - `promise_lapsed`: they promised a date and it passed.
   - `you_promised`: you promised something.
   - `you_owe_reply`: they asked you something.
   - `stale_issue`: a GitHub issue or PR went quiet.
   - `failed_task`: a kwant delegation failed.
   - `unanswered_question`: a kwant question to you is still open.

   Auto-replies, receipts, newsletters and pleasantries ("let me know if you have any
   questions") don't count. A conversation also counts only after `stale_days` (14) of silence,
   or `kwant_stale_days` (3) for kwant's own conversations.
2. **Model check** (optional, `llm: "auto"`, `classify.rs`): a toolless model call sees one
   conversation and must answer with a verbatim quote from it. When the quote isn't in the text,
   or the model says nothing is pending, the candidate is dropped. If no engine can run, the rules'
   finding stands and the scan report says why (`model_note`).
3. **Store** (`db/zombie.rs`): one case per conversation (deduplicated on source and
   conversation key). Case states:
   - `open`: in the queue.
   - `snoozed`: comes back on its date.
   - `dismissed`: never comes back. With the `sender` scope, nothing from that address or login
     in that source comes back.
   - `waiting_reply`: you followed up. The case reopens if nobody answers within the stale days.
   - `closed`: done, or resolved by a later message.
4. **Draft** (`draft.rs`): a template reply that quotes the reference (a claim number, a date). It
   gives `mailto` for email, `open_url` for GitHub, and a thread id for kwant. "Rewrite"
   asks the model for a better draft, which is only kept when it still contains the reference and
   has no `[placeholders]`.

## Weekly "Loose ends review"

The review runs on Monday at 09:00 in the person's time zone (configurable). It scans, then
notifies with the top `max_per_review` (5) cases ranked by kind and age, as a `loose_ends`
notification. The notification has counts only, never subjects, names or quotes. It is silent
when the queue is empty. Each review is recorded (`GET /api/zombie/reviews`).

## Privacy and logging

- Message text never goes into logs. Logs carry counts, ids and error kinds only. The live run's
  log was grepped for the body text: 0 hits.
- Case titles and "why" never contain message text, only subjects and fixed sentences
  (tested). The quote lives only in `evidence`.
- Retention: `retention_days` (90) after a case is closed or dismissed, it is wiped down to its
  dedupe key, so a dismissed case stays dismissed without keeping its text.
- The API is the person's only: no token → 401, a bot's turn token → 403 (all 18 method/route
  pairs are tested).

## API

All routes are under `/api/zombie`. The full list and shapes are in the header of
`src/server/zombie_routes.rs`.

- `GET cases?status=` → `{cases: [CaseView], running}`
- `GET cases/:id` → `CaseView`
- `POST cases/:id/{snooze|dismiss|done|followed-up|reopen|draft}` → `CaseView`
- `GET|POST sources` → list, or add (201 `{source, test}`)
- `PATCH|DELETE sources/:id`, `POST sources/:id/test`
- `POST run {review?}` → 202, or 409 while a scan is running
- `GET status`, `GET|PUT settings`, `GET reviews`

`CaseView` fields:
- `id`, `status`, `kind`
- `conv_kind`: `email`, `issue`, `pull_request` or `kwant_*`
- `source`, `source_id`, `title`
- `counterpart`, `counterpart_name`, `reference`, `link`
- `why`
- `evidence`: `[{quote, from, at, outbound}]`
- `draft`: `{channel, to, subject, body, in_reply_to, mailto, open_url, kwant_thread_id,
  generated_by, created_at}`
- `age_days`, `last_activity_at`, `first_seen`, `status_changed_at`
- `snoozed_until`, `waiting_since`, `follow_ups`
- `deadline`, `resolution`, `dismiss_reason`, `space_id`

## Settings (`GET|PUT /api/zombie/settings`)

| Setting | Default | Notes |
|---|---|---|
| `enabled` | `true` | |
| `stale_days` | `14` | Email and GitHub |
| `kwant_stale_days` | `3` | kwant's own conversations |
| `lookback_days` | `120` | How far back a scan reads |
| `kwant_source` | `true` | |
| `review_enabled` | `true` | |
| `review_time` | `"09:00"` | |
| `review_weekdays` | `["mon"]` | |
| `review_tz` | `""` | Empty uses the person's time zone |
| `max_per_review` | `5` | |
| `llm` | `"off"` | `"auto"` runs the model check |
| `llm_model` | `""` | |
| `retention_days` | `90` | |
| `my_addresses` | `[]` | Addresses that are the person's, besides each mailbox's login |

## Tests

- **Unit tests:** `cargo test --lib zombie`.
- **API, store and review:** `cargo test --test zombie_test`. This includes a mock GitHub API on
  127.0.0.1:189xx and the synthetic mailbox precision report.
- **Synthetic mailbox:** `tests/fixtures/zombie_mailbox/` has 31 threads and 50 messages, 13 of
  them real cases, and 6 holdout threads written after tuning. `manifest.json` says what each
  thread should give. `generate.py` rebuilds it.
- **IMAP end to end**, against a throwaway GreenMail server (removed afterwards):

  ```sh
  docker run -d --rm --name kwnt-imaptest -p 127.0.0.1:18943:3143 \
    -e GREENMAIL_OPTS='-Dgreenmail.setup.test.all -Dgreenmail.hostname=0.0.0.0 -Dgreenmail.auth.disabled' \
    greenmail/standalone:2.1.2
  KWNT_TEST_IMAP=127.0.0.1:18943 cargo test --test zombie_imap_e2e -- --nocapture
  docker rm -f kwnt-imaptest
  ```

### Results (2026-10-20)

| Set | Precision | Recall |
|---|---|---|
| Tuning set, before tuning | 0.82 | 0.90 |
| Tuning set, after tuning | 1.00 | 1.00 |
| Holdout (6 threads, 3 cases) | 1.00 | 1.00 |

The misses before tuning were a same-second auto-acknowledgement, a rhetorical question, a
"let me know if you have questions" sign-off, and a missed lease reminder.

- **IMAP e2e:** 31 conversations, 13 flagged, and no flag changed on the server.
- **Live run** (real `claude` on 127.0.0.1:18950, `env -i`, `llm: auto` with claude-haiku-5-5):
  - The same 13 cases, and the model dropped none, in about 70 s.
  - The model rewrote drafts that kept the reference.
  - A manual review found 13 cases and showed 5.

## Not built yet

- Sending, over SMTP or GitHub. It would always need the person's approval.
- Gmail OAuth (use an app password over IMAP).
- The Batch API for the model check.
- A non-Claude classifier.
