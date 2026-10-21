# Approvals

Each space has an approvals mode, and each bot can override it (`inherit` follows the space).
Only the person changes it: `PUT /api/spaces/:id/approvals` and `PUT /api/kwants/:id/approvals`
with `{"approvals": "ask" | "smart" | "off"}` (a bot also takes `inherit`). Bots' own tools
refuse it.

| Mode | UI label | What happens |
|---|---|---|
| `ask` (default) | Ask before running commands | Every tool call outside the bot's folder shows an approval card. |
| `smart` | Ask only when risky | Ordinary work runs without a card; anything risky or unclear shows the card, with the reason. |
| `off` | Don’t ask | Everything runs. |

Whatever the mode, a call that names kwnt's own secret files (the vault, its key, the API token)
is refused and recorded as `denied`. Calls that run without a card are recorded as
`auto_allowed` (`GET /api/approvals?status=auto_allowed`); under `smart` the row's
`details.smart_reason` says why it was safe (e.g. "runs inside the bot's folder and does nothing risky"), and a card's `details.smart_reason` says why it asked.

`smart` and `off` both need Bitwarden while the built-in password store holds passwords: see
[saved-passwords.md](saved-passwords.md). `smart` is a guard against accidents, not a sandbox: a
script in the bot's folder that it runs can do anything the bot's user can.

## What `smart` runs without a card

- **Shell commands** (claude's `Bash`, API drivers' `execute_shell` in Local mode) that
  `src/mcp/shell_risk.rs` finds safe in the bot's folder. It is an allowlist and fails closed:
  only programs it knows run (file and text tools like `ls`, `cat`, `cp`, `mv`, `mkdir`, `sort`,
  `jq`; build tools like `cargo`, `npm`, `make`, `git` with checks; programs inside the folder),
  and anything else, or anything it can't read confidently, asks. It asks for:
  - `$`, `$(…)`, backticks, unquoted `( ) { }`, heredocs, newlines, `eval`, `source`, `exec`,
    shells with `-c`, a pipe into a shell, inline code (`python -c`, `node -e`, …);
  - wrappers other than `env`, `command`/`builtin` (`-p` too), `nice`, `nohup`, `time`,
    `timeout`, `setsid`, `stdbuf`, and wrapper options it doesn't know; the wrapped command is
    judged;
  - network tools (curl and wget run only for `localhost`/`127.0.0.1` URLs with options from a
    short list: proxies `-x`/`--proxy`/`--socks*`, config files and `@file` ask), `sudo`/`su`, containers and
    cloud CLIs, mail senders, process and system tools, credential tools, `xargs`/`parallel`,
    `env`/`printenv` on their own;
  - `git push` and other git commands that leave the machine, touch global config or run a
    command (`rebase -x/--exec`, `--upload-pack`, `ext::`, `grep -O`, …); publish, login and
    install commands of npm/pnpm/yarn/bun/cargo/pip/uv/gem/poetry/twine/go; `npx`, `bunx`,
    `npm exec`, `pnpm dlx`, `yarn dlx`, `bun x` (they download and run package code);
  - `find -exec/-delete`, `sed` scripts beyond print/delete/substitute (`w`, `e`, `-f`), `awk`
    programs that redirect `print`, pipe, read `ENVIRON` or call `system`/`getline`, and awk
    options other than `-F`/`-v`; `tar` options that run a program (`--checkpoint-action`,
    `--to-command`, `-I`, …) or a `host:file` archive; `grep`/`rg` options that run a program
    (`--pre`) or read a pattern file outside the folder (`-f`, `-e` stuck to its value is read
    correctly); `sort --compress-program`, `zip -TT`;
  - `rm -r` of the whole folder or of a wildcard straight under it (`rm -rf ./*`, `rm -rf */`);
  - any path argument, redirect target or `cd` that resolves outside the folder. Symlinks are
    followed, also for a plain name (`cat link`) and for links a wildcard could match;
    relative paths count from the folder after any `cd`; `/dev/null` is fine;
  - a leading `VAR=value` other than harmless names like `RUST_LOG`, `CI`, `NODE_ENV`.
- **Reads** (Read, Glob, Grep, LS, NotebookRead) outside the folder, unless they touch:
  - live system data: `/proc`, `/sys`, `/dev`, `/run` (`/proc/self/environ` holds the gateway
    token);
  - credentials, matched without regard to case: `.ssh`, `.aws`, `.azure`, `.gnupg`, `.kube`,
    `.docker`, `.netrc`, `.pgpass`, `.vault-token`, `.claude`, `.codex`, `.cargo/credentials`,
    `.config/gh`, `.config/gcloud`, password managers, browser profiles, keychains and keyrings,
    shell histories, `.env*` files, key files (`.pem`, `.key`, `.p12`, `.kdbx`, `id_rsa`, …) and
    files named `credentials`, `secrets` or `token` (the full list is in
    `src/mcp/builtin_tools.rs`);
  - for Grep and Glob, a search rooted at `/`, at a home folder or above one, or at a folder with
    any of the above (or a symlink, or more than 5,000 entries) somewhere below it.

  Paths are checked as written and with symlinks followed; relative paths count from the bot's
  folder. A `~` path asks (a read with no path searches the folder, as before).
- **MCP tools** the server marks `readOnlyHint: true` without `destructiveHint: true`. That is
  the server's own claim. If kwnt can't check it, the call asks.

**kwnt itself always asks.** kwnt's person-only routes take no token from this computer, so a
request to kwnt from a bot could turn approvals off or answer its own cards. A curl, wget or
WebFetch to a loopback host (`localhost`, `127.x`, `0.0.0.0`, `[::1]`, …) on kwnt's serve port,
8799 or 8800 (no port counts as 80/443), or to any loopback path starting with `/api/` (after
decoding `%xx` and working out `..`), shows a card. For WebFetch this holds under `ask` too.
Other local dev servers (`localhost:3000`, `localhost:5173`) still run.

Writes and edits outside the folder and unknown tools still ask. Under every mode, a Glob whose
`pattern` is absolute, starts with `~` or contains `..` counts as a read outside the folder.

## "Always this session"

Under `ask` and `smart`, "Always this session" on a claude tool's card allows that tool (e.g. all
`Bash`) for that bot and conversation until kwnt restarts. Under `smart`, API drivers'
`execute_shell` is remembered the same way, per conversation; under `ask` it covers that one
command only.

## Tests

`cargo test --test smart_approvals_test --test approvals_off_test --test approvals_passwords_test`
and the classifier tables in `src/mcp/shell_risk.rs` (`cargo test --lib shell_risk`).
