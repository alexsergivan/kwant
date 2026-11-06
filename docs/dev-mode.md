# Dev mode

Dev mode (0.7) points a space at the GitHub repos its teams work in: from 0.7.2 every team's repo,
up to 10 a space (see [Every team's repo](#every-teams-repo-072)), found in the team folders and repo
links. kwnt then reads each repo and turns what happens there into cards. It writes to GitHub
only when the person presses **Post to GitHub** on a review card (0.7.1, see
[Posting to GitHub](#posting-to-github)); nothing else is ever sent there.

- **Landed**: one card per commit that reaches the watched branch, saying what changed and how risky it is.
- **Review**: one card per open pull request head, with findings. The person can press **Fix it**
  (the finding goes to the bot that wrote the PR) or **Dismiss** on each finding, and **Post to
  GitHub** to put the card on the PR as one review comment.
- **Release notes**: written on request, from the landed cards since the last tag.

It also turns on the **production floor** for the space's bots: steps that can reach production
always ask (see below).

Everything in Dev mode belongs to the person. Every route is refused for a bot's turn token, reads
included, because the settings hold the backup command and the cards hold the repo's diffs. The
tables (`dev_mode_spaces`, `dev_repos`, `dev_cards`, `dev_backups`, `dev_model_calls`, `dev_prod_connections`)
are left out of exports, like approval modes.

## Before you turn it on

Dev mode reads GitHub through the Loose ends GitHub connection, so it needs a GitHub token saved
first. With kwnt's built-in password store, a token can only be saved while **every** space and
every bot is on "Ask before running commands". New installs start on "Ask only when risky", so the
save is refused (409) until you either:

- switch every space and every bot to "Ask before running commands", or
- connect Bitwarden.

While the token is in the built-in store, approvals can't go back to Off or "Ask only when risky".
See [Saved passwords](saved-passwords.md#approvals-off-needs-bitwarden) for why.

## Settings

`PUT /api/spaces/:id/dev_mode` saves these fields:

- `repo`: a URL, an ssh address or `owner/name`, normalised to `owner/name`;
- `branch`: the branch to watch;
- `deploy_hint`: how the repo ships, for example `cloudflare_pages` or `fly`;
- `backup_command`: the command that backs up production data.

`POST …/dev_mode/check` looks the repo up through the Loose ends GitHub connection and answers
`found`, `not_found` or `not_connected`. A new repo or branch starts the watch over. The same one
keeps its place.

### The repo comes from the space (0.7.1)

Turning Dev mode on never asks for a repo. kwnt goes through the space's teams in order and takes
each team's repo link, or else its folder when that folder is a git checkout. For a folder it reads
`git remote get-url origin` (or the first remote) and the default branch (`origin/HEAD`, else the
checked-out branch). It does this locally, with no network, no prompt and a 3 second limit. A
checkout with no remote is a `local` source. Two teams on the same repo count once. The user and
password part of a remote URL (`https://user:token@host/…`) is dropped before the URL is read, and
it is never logged, returned or stored.

`PUT {on: true}` with no `repo` watches the first detected source a GitHub connection can read. If
none can, or the space has none, Dev mode still turns on with no repo. The production floor and the
command guards work without one, and the poller waits. A watched repo stays watched when GitHub
fails to answer. A `repo` asked for that can't be read also turns Dev mode on, with no repo, and the
reply carries a `check` that says why. A `branch` the repo doesn't have is a 400.

`GET` (and the `PUT` reply) add:

- `sources`: `[{team, repo, host, from}]`. `from` is `repo` (the team's repo link), `folder` (its
  folder's remote) or `local`. `repo` is `owner/name` on GitHub and the whole path on other hosts. A
  `local` source has the folder's own name and no host. Folder paths are never returned.
- `status`:
  - `watching`: a repo is watched. While off, it means turning on would watch one.
  - `no_repo`: no team points at a git repo.
  - `not_connected`: there's a GitHub source, but no connection reads it. Once a GitHub connection
    is added or changed in Loose ends, every space that is on with no repo binds the repo it finds
    and polls soon, with no off and on. The `GET` does the same if nothing else did (at most every
    30 s per space). A repo the person typed or picked is never replaced.
  - `no_git`: only GitLab, Gitea or other non-GitHub sources, or `local` ones, and no git 2.31+ on
    the server to read them. The reply adds `install_hint`. (0.7.2's `other_host` is retired in
    0.7.3: git reads those repos now, so they show `watching`.)
- `new_reviews`: review cards ready to read that were made since the desk was last opened (open
  ones only, never landed cards or floor approvals), every watched repo's together: one count for
  the space, with one seen time. A space never opened counts 0.
  `POST /api/spaces/:id/dev_mode/seen` marks the desk opened now, so phone and desktop agree. It
  sets `reviews_seen_at` only. "Mark seen" (`POST …/dev/landed/seen`) sets `seen_at`, which the
  landed rollup counts from, and never clears the badge. Before 0.7.2 both routes shared
  `seen_at`, so opening the desk emptied the rollup; a database from then starts with
  `reviews_seen_at` copied from `seen_at`, so its badge is unchanged.

### Every team's repo (0.7.2)

A space watches every team's GitHub repo at once, not just the first. Each watched repo is a row of
`dev_repos`, keyed `host/owner/name` (lowercased), with its own branch, last seen commit, poll state
and newest tag. The full contract is in the team's `dev-mode/team-repos/api.md`; in short:

- **Rows follow the teams.** Turning on, a team's repo link changing, a team made, changed or deleted,
  a GitHub connection added or changed, and a `GET` (at most every 30 s per space) reconcile the rows:
  a row for each team source a connection reads (`added_by: "team"`), and a team row whose team no
  longer points at it is dropped (its cards stay). The person can add more by hand (`PUT {repo}`,
  `added_by: "hand"`) and remove those with `DELETE /api/spaces/:id/dev_mode/repos/:key` (the key
  URL-encoded as one segment). A team row can't be deleted (409 `team_repo`): change or clear the
  team's link with `PUT {repo, team}` (`repo: ""` clears it).
- **At most 10 repos a space.** A team source past the cap shows `too_many` and has no row; a hand repo
  past it is a 409 `too_many`.
- **`GET` and the `PUT` reply** have `repos[]`: one entry per repo, in team order then hand rows, with
  `key`, `repo`, `host`, `team`, `team_id`, `from`, `provider` (0.7.3: `github` when it's read
  through a GitHub connection, GitHub Enterprise too, `git` when git alone reads it), `branch`, `status` (`watching`, `not_connected`,
  `no_git`, `error`, `too_many`; 0.7.2 also had `other_host`), `poll_error`, `poll_code` (0.7.3: `too_big` for a repo over the mirror cap or an answer from git too big to read, else null),
  `polled_at`, `last_tag` and `added_by`. The
  top-level `repo`, `branch`, `deploy_hint`, `last_seen_sha`, `polled_at` and `poll_error` are the
  first watched repo's, kept for one release only (deprecated). Top-level `status` is `watching` when
  any repo is.
- **Polling** reads each repo in turn. A repo that fails records its own `poll_error` and the next is
  still read. A rate limit is the token's, so it backs the whole space off.
- **Cards** carry `repo` (the key), `repo_name` and `team`. PR #12 in two repos is two cards. The lists
  take `?repo=<key>` (`/dev/landed`, `/dev/reviews`, and `POST /dev/release_notes` for that repo's
  notes); an unknown key is an empty list. **Post to GitHub** writes to the card's own repo and host.
- **Space-wide, unchanged:** the production floor, the command guards, approvals, backups and the 30
  model calls a day.
- **Errors** are `{error, code}`: `bad_repo`, `bad_branch`, `no_branch`, `unknown_team`,
  `backup_too_long` (400); `unknown_repo` (404); `team_repo`, `too_many`, `no_repo`, `off` (409);
  `github` (502); `internal` (500).

**Upgrading from 0.7.1** rebuilds `dev_cards` once with a `repo` column (the unique key becomes
`(space_id, repo, kind, ref, head_sha)`) and turns each space's one repo into a `dev_repos` row keyed
`github.com/owner/name`, with its branch, last seen commit and poll state, so nothing is announced
again. The first reconcile hands it to the team that points at it, re-keying it and its cards if that
team's repo is on a GitHub Enterprise host.

**Going back to 0.7.1** after this is safe but limited. 0.7.1 still starts, reads every card and
setting, and polls its one repo, but it can't make cards: its insert names the old unique key, SQLite
refuses it, and 0.7.1 logs "could not save a card" and goes on. A trigger
(`dev_cards_071_keeps_to_its_repo`) keeps it off other repos' cards: 0.7.1 finds review cards by PR
number across the whole space, so without it its poll would mark other repos' reviews stale and its CI
recheck would overwrite their facts. Every 0.7.2 write to `dev_cards` bumps a `rev` column; a write
that doesn't (0.7.1's) and would mark stale, or change the facts of, a card of a repo other than
`dev_mode_spaces.repo` is skipped. Coming back up to 0.7.2 rebuilds nothing; each repo carries on from
its own `dev_repos.last_seen_sha` (0.7.1 never writes that table), so what landed meanwhile becomes cards
once. A repo changed by hand while on 0.7.1 is not carried over (0.7.1 writes it to
`dev_mode_spaces.repo`); teams' repo links are.

### Any git repo (0.7.3)

Dev mode reads every watched repo's history with git (any-git.md G1-G3). kwant keeps a bare mirror
of each repo at `<data>/dev/<space>/<key>-<hash>.git` (deleted when the repo stops being watched; its
cards stay), or reads a `local` team folder's own `.git` in place, never touching its working tree.
Commits, diffs, file text, tags, path history and the root's names come from git; on GitHub, pull
requests, checks and review cards still come from the API, and the API answers when a fetch fails.
A team's repo on GitLab, Gitea, a bare git server or a folder with no remote now gets a row too
(`provider: "git"`, with the URL git fetches from in `git_url`) and landed cards, with no PR, CI or
review cards until its forge is added (G4, G5).

Git runs as kwant's own user, so its SSH keys and credential helpers work. A GitHub token goes to git
only as an `http.extraHeader` in `GIT_CONFIG_*` environment variables, never in argv, a file or a
log; URL userinfo is stripped before anything else. http(s) and git:// addresses lose all of it;
ssh keeps a user name (`ssh://deploy@host/x.git`, `deploy@host:x.git`) and never a password, and
`git_url` is never logged. Each call has no terminal prompt, a timeout,
`nice` and its own process group, which is killed when it runs over. The Docker image installs git
and openssh-client. Without git 2.31+, the status is `no_git`.

A remote read from a team's folder or repo link is fetched only over https, http, ssh or git, and
never from this machine or a link-local address (`localhost`, `127.0.0.0/8`, `::1`, `169.254.0.0/16`,
`fe80::/10`, `0.0.0.0`): anyone working in the folder can write its remote, so it can't aim kwant at
the server itself or a cloud metadata address. Such a source shows `no_repo`. LAN hosts (a Gitea on
`192.168.x`) are fetched.

Right before every fetch or remote read (GitHub's too), kwant looks the host up itself and refuses it
if any address is loopback, unspecified, `0.0.0.0/8`, link-local (`169.254.0.0/16`, so
`169.254.169.254`, and `fe80::/10`), broadcast, the cloud metadata addresses outside link-local
(`100.100.100.200` and `fd00:ec2::254` exactly; `100.64.0.0/10`, Tailscale's, stays allowed), or one
of these inside an IPv6 address (IPv4-mapped
or -compatible, NAT64): a name that resolves to the server is refused like the address itself. The
row reads `error` ("Dev mode can’t use this repo address or branch name."). Private LAN ranges stay
allowed. Over http(s) git is then pinned to the addresses checked with
`-c http.curloptResolve=<host>:<port>:<addresses>` (git 2.37+), so DNS can't answer differently
between the check and the connect; ssh and git:// (and git 2.31-2.36) get the check alone. git never
follows an HTTP redirect, since it would lead to a host the check didn't see: a remote that redirects
(say `http://` to `https://`) reads as an error until its URL is the one the host serves. A GitHub
remote kwant can't fetch falls back to the API, as any failed fetch does.

The address check and the pin don't cover what the server's admin set up for git outside kwant: a
proxy (`http_proxy`, `https_proxy`, `ALL_PROXY` in kwant's environment, or `http.proxy` in the
admin's git config) gets the request and looks the host up itself, so it bypasses the pin and the
check; and an ssh host alias or `ProxyCommand` in kwant's `~/.ssh/config` connects to an address the
check never saw. Over ssh, git always runs `ssh` (or the admin's `GIT_SSH`/`GIT_SSH_COMMAND`), never a
command a repo's own config names; a host whose key isn't in kwant's `known_hosts` fails ("The repo’s
host said no"), so the admin adds it first. DNS can still change between the check and the connect
over ssh, git:// and git older than 2.37 (the check stands alone there).

**Team folders.** A `local` team folder is read only if its `.git` is its own, checked again on every
poll because anyone working in the folder can change it. The row reads `error` ("Dev mode can’t use
this repo address or branch name.") when `.git` is a symlink; when it is a file (a linked worktree)
whose git dir doesn't point back at this folder or doesn't sit in its main repo's `worktrees/`; when a
`.git` directory has a `commondir`; when the git dir or its main repo is under `<data>/dev`; or when
it has `objects/info/alternates` or a symlink among its objects and refs. A real `git worktree add`
checkout reads as before; a git submodule checkout (whose `.git` file names `.git/modules/…`) is
refused.

**Mirror size.** A mirror is a full bare copy of the repo. A fetch may not write any file past 1 GB
(`RLIMIT_FSIZE`, with what it fetches kept as one pack), so a huge or hostile remote stops at the cap
rather than after it. A fetch stopped there, or a mirror over 1 GB on disk after its fetch, is deleted and a `<mirror>.too-big` note is left beside it; the row reads `error` with "This repo
is too big for Dev mode to read (over 1 GB)." and `poll_code: "too_big"` (a GitHub row falls back to
the API instead). One answer from git over 16 MB (a commit touching a vast number of files) reads
`error` with "This repo is too big for Dev mode to read." and the same `poll_code`. The repo
isn't fetched again for a day, or until it is removed and added again, which deletes the note.

**Tags and merges.** `last_tag` is the nearest tag reachable from the watched branch (`git describe`),
where 0.7.2 took GitHub's newest tag: a tag on another branch no longer counts. A merge commit's
files are its diff against its first parent, which is what the pull request changed.

**New columns, and going back to 0.7.2.** `dev_repos` gains `git_branch` and `git_url` (additive,
both `NULL` on GitHub rows). A git row keeps `branch = ''` and its real branch in `git_branch`, so
0.7.2, which ignores the new columns, never polls it (an empty branch waits for reconcile), and its
reconcile drops the row as one no GitHub source points at. Its cards stay, and 0.7.2 shows them like
any other repo's. Coming back to 0.7.3 adds the row again; its first poll back-fills the newest
merges, and a commit that already has a card never gets a second. GitHub rows are unchanged, so 0.7.2
keeps polling them from their `last_seen_sha`. The mirrors under `<data>/dev/` are left on disk by
0.7.2 and reused by 0.7.3. 0.7.2 ignores the `<mirror>.too-big` notes and the `poll_code` field;
back on 0.7.3 a note younger than a day still holds its repo back.

`GET /api/dev/cards/:id` returns one card of any kind, in the same shape as the lists, or 404. The
desk reads a release notes draft here once its `dev_card` event says it is ready, since no list
returns those. A card in an archived space is refused like that space's lists.

Turning Dev mode **off** (D12) stops the polling, the reviews and the floor. Cards stay readable,
and approvals stay as they were.

## Reading GitHub

Reads go through the GitHub connection only. kwnt never pushes, merges, approves or edits anything
in the repo; its one write is the review the person posts (below). Every read sends
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

## Posting to GitHub

`POST /api/dev/cards/:id/post` (0.7.1) puts one review card on its pull request as **one GitHub PR
review** with `event: "COMMENT"`. kwnt never approves or requests changes, never merges, and never
posts on its own: only the person's press posts, and the route refuses a bot's turn token like every
Dev mode route. Landed and release notes cards can't be posted (400). An unknown card is 404. A
review that didn't finish is 409 and makes no GitHub call: one still being written (`pending`), one
that `failed` (its card keeps only the rule checks, so it would read as a hollow "Low" review), or
one with no card text. The error is "This review didn't finish, so there's nothing to post. Run the
review again first." Nothing is stored, so the card shows no `post_error`.

**What is posted.** The review is pinned to the card's head (`commit_id`). Its body has the risk
level and its reasons, the intent check (done, missing, not asked for), the test-tamper flags with
their diff excerpts, the security tiles (secrets, new packages, OSV, Semgrep), and the findings that
can't go on a line. It ends "Reviewed by {reviewer} ({engine}) in kwant" ("Reviewed by kwant's rule
checks (no reviewer bot) in kwant" when no bot wrote it), then a hidden marker
`<!-- kwant review card {id} -->`. Each finding that isn't dismissed and whose file and line are
inside the PR's diff hunks (the new side) becomes an inline comment, `side: RIGHT`, at most 50 of
them; the rest are listed in the body. If GitHub still answers 422 about lines, kwnt posts once more
with everything in the body.

**What is never posted.** Secrets never leave kwant. A gitleaks finding is posted only as "Possible
secret in {file}:{line}", without its rule, excerpt or value. The body and every comment then go
through the same redaction as bot output (`[redacted]` over keys, tokens and long hex), so a model
finding that quotes a key doesn't carry it out either. The GitHub token is only ever a header.
Dismissed findings and the card's model prompts aren't posted.

**The token.** Posting uses the Loose ends GitHub connection, GitHub Enterprise `api_base` included.
It needs **Pull requests: Read and write** on a fine-grained token, or `repo` (`public_repo` for a
public repo) on a classic one. Reading alone works with read access, so posting is the only reason
to give it more.

**The answer** is always the card, with HTTP 200, carrying two fields (review cards only):

```jsonc
"posted": null | {"at": "…", "login": "octocat" | null, "url": "https://github.com/o/r/pull/24#pullrequestreview-1" | null,
                  "comments": 3, "forge": "github"},
"post_error": null | {"kind": "read_only" | "stale" | "failed", "message": "…"}
```

- `read_only`: GitHub answered the write with 403 or 404, so the token can read but not post.
- `stale`: the PR's head moved past the card's head (or the card is already stale). Nothing is
  posted: a new review is on its way.
- `failed`: anything else (no connection, a closed PR, a rate limit, a second 422). Press again to
  retry. A successful post clears `post_error`.

`posted.comments` is the number of inline comments (0 after the 422 retry). Each post emits the
`dev_card` event.

**Once only.** A posted card answers unchanged, without asking GitHub. Presses of one card wait on a
lock, and the database only records the first post, so a double click makes one review. If GitHub
took a post but its answer was lost, the next press finds that review by its marker and head, and
records it instead of posting again.

**Who posts.** `GET /api/spaces/:id/dev_mode` has `forge` (`"github"` once the space has a repo and a
GitHub connection is saved, else `null`; `gitlab` and `gitea` are reserved) and `forge_login`, the
login the post goes out as, from `GET /user`. That login is cached per connection for 15 minutes (a
minute when it fails). Only GitHub can be posted to in 0.7.1; the code keeps it behind one function,
`post_review` in `src/server/dev/post.rs`, so other forges can join later.

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
cargo test --test dev_post_test --test dev_github_test
cargo test --lib -- db::dev_mode::tests
cargo test --test dev_team_repos_test --test dev_autodetect_test
```

`tests/dev_team_repos_test.rs` covers 0.7.2's repos: two teams' repos with PR #12 in both (two cards,
each posted to its own repo, `?repo=` filters), a failing repo next to a working one, the 10-repo cap,
a migrated row taken over by its team, and the routes' error codes. The 0.7.1 → 0.7.2 migration is
tested on a 0.7.1-shaped database in `src/db/dev_mode.rs`.

The no-tools turns have their own: `cargo test --lib -- no_tools`.

The poller runs against a mock GitHub on a private `127.0.0.1` port. The shared mock is
`tests/support/mock_github.rs` (include it with `#[path = "support/mock_github.rs"] mod mock_github;`):
fixtures by path with ETags and 304s, `Link: next` headers (to another host too), each GitHub
error (401, 403, rate limits, secondary limits, 429, 5xx, slow answers, a lost base commit), and a
log of every request. Anything but a GET fails the test, except a write the test queued with
`accept_write` (`tests/dev_post_test.rs` queues the review posts and reads them back with `writes()`
to check what was sent, the AKIA-style secret test included). `tests/dev_github_test.rs` holds the
contract tests and the canary: a token built at runtime goes through a whole poll, failures
included, and must appear nowhere but the vault (logs, errors, events, cards, the database, model
prompts, other files under the data folder).

Test seams, for the test server only: `KWANT_DEV_POLL_SECS` (5–86400, default 300) and
`KWANT_DEV_CALL_CAP` (0–1000, default 30) and `KWANT_DEV_MIRROR_CAP_BYTES` (1 B to 1 TB, default
1 GB, so QA can make a small repo too big), read once at the first use. The daily cap counts per UTC
day. Tests move Dev mode's clock in-process with `server::dev::set_clock_skew`
(`tests/dev_seams_test.rs`). There are table tests for each
floor kind in `src/mcp/prod_floor.rs`. The backup checks and the Allow refusal are tested in
`src/server/dev/backup.rs` and `src/server/dev/floor.rs`. `tests/dev_floor_test.rs` drives the refusal
through `POST /api/approvals/:id/resolve`: a 409 with no backup or an old one, then a 200 once a
fresh one exists, and Deny always accepted.
