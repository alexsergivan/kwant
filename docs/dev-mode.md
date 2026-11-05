# Dev mode

Dev mode (0.7) points a space at one GitHub repo and branch. kwnt then reads the repo, **read only**,
and turns what happens there into cards:

- **Landed**: one card per commit that reaches the watched branch, saying what changed and how risky it is.
- **Review**: one card per open pull request head, with findings. The person can press **Fix it**
  (the finding goes to the bot that wrote the PR) or **Dismiss** on each finding.
- **Release notes**: written on request, from the landed cards since the last tag.

It also turns on the **production floor** for the space's bots: steps that can reach production
always ask (see below).

Everything in Dev mode belongs to the person. Every route is refused for a bot's turn token, reads
included, because the settings hold the backup command and the cards hold the repo's diffs. The
tables (`dev_mode_spaces`, `dev_cards`, `dev_backups`, `dev_model_calls`, `dev_prod_connections`)
are left out of exports, like approval modes.

## Settings

`PUT /api/spaces/:id/dev_mode` saves these fields:

- `repo`: a URL, an ssh address or `owner/name`, normalised to `owner/name`;
- `branch`: the branch to watch;
- `deploy_hint`: how the repo ships, for example `cloudflare_pages` or `fly`;
- `backup_command`: the command that backs up production data.

`POST …/dev_mode/check` looks the repo up through the Loose ends GitHub connection and answers
`found`, `not_found` or `not_connected`. A new repo or branch starts the watch over. The same one
keeps its place.

`GET /api/dev/cards/:id` returns one card of any kind, in the same shape as the lists, or 404. The
desk reads a release notes draft here once its `dev_card` event says it is ready, since no list
returns those. A card in an archived space is refused like that space's lists.

Turning Dev mode **off** (D12) stops the polling, the reviews and the floor. Cards stay readable,
and approvals stay as they were.

## Reading GitHub

Reads go through the GitHub connection only, and kwnt never writes to the repo. Every read sends
the last ETag it got, so an unchanged answer costs no rate limit. A rate limit (a 403 or 429 with
`retry-after` or `x-ratelimit-remaining: 0`) backs the space off until the reset time GitHub gives,
at most an hour, or a minute for a bare 429. Another 403 is a permission error. The settings show
either one as the poll error.

Every 5 minutes, for each space with Dev mode on, kwnt:

1. Reads the latest tag. Release notes start from it.
2. Reads the commits on the branch since `last_seen_sha`. The first poll backfills the last 5. A
   poll writes at most 20 new cards. A rebase merge (several commits, one PR, no merge commit) is
   skipped.
3. For each commit, reads the PR it came from, the files and patches, and the check runs, and
   writes a `landed` card with its DevFacts. A patch past 60k characters is dropped whole, and the
   facts say "diff cut, N file(s) not read". The bot that wrote it is matched by commit author name
   (with "(kwant bot)" removed) or by the `<name>-bot` email.
4. Reads the open PRs and writes a `review` card per new head, at most 10 per poll. A new push makes
   the old head's card stale, and so does a draft or a closed PR.
5. Checks CI again on the last 10 cards whose CI was pending, for 6 hours. A change re-runs the
   card's text.

Each new card is pending until `crate::server::dev::generate` writes its text, using at most 30
model calls per space per day. After a restart, cards still pending are generated again, and a
**Fix it** that was sent but never reached its bot is sent again.

Card text is written from PR titles, bodies and diffs, which anyone who can open a PR controls, so
those model calls run with `TurnOpts::no_tools`: no MCP servers, no browser and none of the engine's
own tools (shell, files, web search, subagents, skills). claude, codex and grok start with every
tool switched off, and the run is stopped and its reply dropped if the CLI still offers or starts
one. Any other CLI, and any turn that was given MCP servers, is refused instead of run with tools.
The flags for each engine are on `TurnInput::no_tools` and next to `no_tools_refusal` in
`src/drivers/cli.rs`. When a bot's engine refuses (or stops) a card turn this way, that card call
fails at once: it is not asked again, with or without tools, and the card is `failed` with the
reason and keeps its rule-based parts (risk, test tamper, CI), as any failed call does.

### Verified

A landed card is **Verified** only when, as the card is written, the PR has a Dev mode review card
of the head that merged, by a bot that isn't the PR's author (compared by bot id), with no open
finding, and CI on the merge is green. A PR that merges before its review is ready, or that was
reviewed outside kwnt, gets a landed card that says "not verified", and it stays that way: a review
that finishes later doesn't rewrite the landed card (only a CI change re-runs a card's text). That
is on purpose. Verified says what was checked before the change landed, not after.

## The production floor

In a space with Dev mode on, these steps always show an approval card, whatever the bot's
approvals say (`ask`, `smart` or `off`), and whatever "always allow" was given before:

| Kind | What it is | Needs a backup |
| --- | --- | --- |
| `push_main` | pushing to the watched branch or a protected one (`main`, `master`, `production`, `prod`, `live`, `stable`, `trunk`, `gh-pages`), or `gh pr merge` | no |
| `force_push` | `git push --force`, `-f`, `--force-with-lease` or `+ref` to a protected branch | no |
| `delete_ref` | deleting a protected branch or a release tag | no |
| `deploy` | wrangler, fly, vercel `--prod`, netlify `--prod`, firebase, railway, kamal, serverless, kubectl, helm, terraform or pulumi apply, `docker compose up` or `restart` when the space deploys with Docker (or on another host, or over `ssh`), a push to a deploy remote (heroku, dokku, …), `make`, `npm` or `just` targets and scripts (`./deploy.sh`, `python release.py`, `node deploy.js`) named deploy, ship or release, deploy tools on infrastructure MCP servers | no |
| `deploy_migration` | a migration run against production: sqlx, diesel, prisma `migrate deploy`, rails `db:migrate`, alembic, `manage.py migrate` and others, when the command names a production database or environment (before or after it: `RAILS_ENV=production`, `--settings=app.prod`) | **yes** |
| `destructive` | `DROP`, `TRUNCATE`, or `DELETE`/`UPDATE` with no `WHERE` through a database client, `rm -r` on data folders or database files, `docker volume rm`, `compose down -v`, `terraform destroy`, delete tools on infrastructure apps | **yes** |
| `prod_key` | any tool on a connection the person marked **production** (D10) | no |

- The card's `details.prod_floor` is `{kind, why, blast, needs_backup, space_id, backup}`.
  - `blast` is a list of `[label, value]` rows, and only rows that are known: Where, What changes,
    Downtime, Data, Key used, Migration, If it fails.
  - `backup` is the space's backup state: `{state, age_s, bytes, path, error}`, where `state` is
    `fresh`, `old`, `none`, `no_data`, `no_command`, `taking` or `failed`.
- **Always this session** on a floor card allows that one call. Nothing is remembered, and the
  next one asks again. The same goes for a late Allow.
- **Allow** on a card that needs a backup is refused by the server (`409`, `status: "needs_backup"`)
  until the space has an `ok` backup at most 10 minutes old. This is checked when the person
  answers, so a backup taken while the card waits counts.
- `POST /api/spaces/:id/dev/backup` runs the backup command in the bot's workspace, with a
  10-minute limit and one run at a time per space. The run finishes and is recorded even if the
  request goes away (the browser closes), and the space is free for the next one either way.
  kwnt then records what it can tell:
  - the exit code;
  - the file or folder the output names (a bare name is looked up in the bot's workspace), its
    size, and whether it is empty;
  - whether that file was written by this run: one older than the run's start (with 5 seconds of
    slack) isn't counted, so an old dump can't stand in for a new one;
  - a remote target (`s3://`, `gs://`, `r2://`, …), which counts as ok on exit 0 because its size
    can't be checked.

  Exit 0 with no file or remote target named in the output is **not** a backup: the reply is
  `ok: false` with "kwant couldn't find a backup file in the output". Make the command print
  where it wrote, for example `… && echo "Backup saved to backups/app.sql"`.
- **Marking a connection production** (D10): `PUT /api/connections/:id/production?space=` with
  `{production: true|false}`. Only an MCP server in the space (`mcp:<id>`) or a Composio app
  (`composio:<slug>`) can be marked. `GET /api/connections` shows `production` on each one.

Like the payment floor, the classifier (`src/mcp/prod_floor.rs`) is a pure function. It reads only
the call and the space's Dev mode settings, never anything the bot says about the call. It runs:

- in `BrokerApprover`, which covers claude's own tools (Bash and the rest, through claude's
  permission prompt) and every MCP tool;
- in `approve_local_shell_in`, which covers the API drivers' `execute_shell`.

It asks for some things that never reach production, such as `npm run release` in a test repo.
Compose is only read as a deploy where the space's deploy hint is Docker, or when it runs on
another host (`-H`, `--context`) or over `ssh`; `compose down`, `stop`, `pull`, `rm` and `create`
never ask, and `compose down -v` always does because it deletes volumes. That is accepted: an extra card costs a tap, and a missed
deploy can cost a site.

### What the floor can't see

- **Codex's own shell.** kwnt never sees Codex's commands, only the MCP tools Codex calls. Its
  sandbox has no network, so it can't push or deploy. It can still delete files inside its
  workspace, a local database file included.
- **Commands hidden in scripts.** Running a script (`./deploy.sh`, `bash script.sh`,
  `python release.py`, `node deploy.js`, `ruby ship.rb`) is read by the script's *name* only. A script called `run.sh` that deploys
  doesn't ask. The same goes for Makefile targets, npm scripts and CI workflows named for
  something else, and for git hooks and aliases.
- **Indirection.** A command built from variables (`$CMD`, `git push origin $BRANCH`), piped into
  a shell (`curl … | sh`, `echo … | bash`), loaded from a file, or run through another language
  (`python -c "subprocess…"`) isn't seen. `eval '…'` with literal text is read, and so are
  wrappers that run the rest of the line with their own flags (`sudo -u`, `env`, `timeout 60`,
  `nice -n 10`, `ionice`, `stdbuf`, `setsid`, `nohup`, `watch -n1`, `xargs`). Nesting deeper than
  3 levels of `ssh` or `sh -c` isn't read, and neither are quotes nested inside quotes.
- **SDKs and HTTP calls.** A deploy or delete made with `curl` against a provider's API, or from
  code that uses its SDK, doesn't ask unless it goes through a connection marked production.
- **Tools and providers missing from the lists**, and MCP tools whose names don't say what they
  do. Mark such a connection production to make every one of its tools ask.
- **Git writes through a connection.** A GitHub MCP server or Composio app that isn't marked
  production can write to the repo with no card: `push_files`, `create_or_update_file` on `main`,
  `merge_pull_request`, `delete_branch` and the like reach the watched branch directly. The floor
  reads git commands and deploy tools, not these tool names. Mark that connection production
  (Connections → Production) to make every one of its tools ask.
- **Where a command runs.** The floor can't tell a production database from a local copy unless
  the command says so (a remote URL, `--prod`, `RAILS_ENV=production`, …). So a plain
  `sqlx migrate run` doesn't ask, but `DROP TABLE` does, wherever it runs.
- **What CI does after a push.** A push to the watched branch asks, because it may deploy. A push
  to another branch that CI deploys from doesn't ask, unless that branch is protected.
- **The checked-out branch** is read from `.git/HEAD` in the bot's folder, or the folder above it.
  A `git checkout` or `git switch` earlier in the same command is followed: `git switch main &&
  git push` asks. `git checkout x` could restore a file `x`, so unless `x` is protected the branch
  then counts as unknown, and a bare push after it asks. A `cd other-repo && git push` is judged by
  the first repo's branch. A bare `git push` from a
  detached head, or from a folder that isn't a repo, counts as a push to a protected branch.

## Tests

Run these focused tests:

```sh
cargo test --lib -- server::dev:: prod_floor auth::tests::person_only zombie::github connections::tests::only_mcp
```

The no-tools turns have their own: `cargo test --lib -- no_tools`.

The poller runs against a mock GitHub on a private `127.0.0.1` port. The shared mock is
`tests/support/mock_github.rs` (include it with `#[path = "support/mock_github.rs"] mod mock_github;`):
fixtures by path with ETags and 304s, `Link: next` headers (to another host too), each GitHub
error (401, 403, rate limits, secondary limits, 429, 5xx, slow answers, a lost base commit), and a
log of every request. Anything but a GET fails the test. `tests/dev_github_test.rs` holds the
contract tests and the canary: a token built at runtime goes through a whole poll, failures
included, and must appear nowhere but the vault (logs, errors, events, cards, the database, model
prompts, other files under the data folder).

Test seams, for the test server only: `KWANT_DEV_POLL_SECS` (5–86400, default 300) and
`KWANT_DEV_CALL_CAP` (0–1000, default 30), read once at the first use. The daily cap counts per UTC
day. Tests move Dev mode's clock in-process with `server::dev::set_clock_skew`
(`tests/dev_seams_test.rs`). There are table tests for each
floor kind in `src/mcp/prod_floor.rs`. The backup checks and the Allow refusal are tested in
`src/server/dev/backup.rs` and `src/server/dev/floor.rs`. `tests/dev_floor_test.rs` drives the refusal
through `POST /api/approvals/:id/resolve`: a 409 with no backup or an old one, then a 200 once a
fresh one exists, and Deny always accepted.
