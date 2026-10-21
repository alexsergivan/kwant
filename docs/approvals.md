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
  `src/mcp/shell_risk.rs` finds safe in the bot's folder. It fails closed: anything it can't read
  confidently asks. It asks for:
  - `$`, `$(…)`, backticks, unquoted `( ) { }`, heredocs, newlines, `eval`, `source`, `exec`,
    shells with `-c`, a pipe into a shell, inline code (`python -c`, `node -e`, …);
  - network tools (curl and wget run only for localhost URLs with plain flags), `sudo`/`su`,
    containers and cloud CLIs, mail senders, process and system tools, credential tools,
    `xargs`/`parallel`, `env`/`printenv` on their own;
  - `git push` and other git commands that leave the machine or touch global config; publish,
    login and install commands of npm/pnpm/yarn/bun/cargo/pip/uv/gem/poetry/twine/go;
    `find -exec/-delete`, `sed` scripts beyond print/delete/substitute (`w`, `e`, `-f`), `awk`
    programs that write files, pipe or call `system`/`getline` (or `-f`), `rm` of the whole folder;
  - any path argument, redirect target or `cd` that resolves outside the folder (symlinks
    followed; `/dev/null` is fine), and programs outside the folder and the system bin folders;
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
