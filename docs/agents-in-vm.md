# Run Claude inside the VM (0.7.7, slice 1)

When the computer is in **VM** mode and **Run Claude inside the VM** is on, a bot's Claude turn
runs the `claude` CLI in a container of its own, with no network. The container never holds a
real key or sign-in. Its only way out is kwnt's broker, a Unix socket mounted read-only at
`/run/kwnt`. The broker swaps the stand-in credential for the real one, forwards a short list of
Anthropic calls, and refuses everything else.

- **Off by default.** Settings › Computer, under the VM card.
- **Linux only in 0.7.7.** On a Mac the switch is shown disabled with "Coming to Mac later".
- **Claude only.** Codex and Grok still run on this computer ("Not isolated yet").
- **Claude sign-ins only.** A Claude API-key or Custom account still runs on this computer; its
  chip says "Not isolated".
- **Local mode:** the setting is kept but does nothing, and the chip says "Not isolated".

## What it protects

A Claude bot that has been talked into misbehaving, by a web page, a file or another bot's
text, can't do these things while it runs inside the VM:

- **Read your sign-ins or keys.** `~/.claude`, `~/.codex`, `~/.grok`, `~/.ssh`, kwnt's data folder
  and the sign-in homes are never mounted. The container gets a stand-in key, and the real one
  stays in the broker. A folder that is, holds or sits inside one of these is refused before the
  container starts (`unsafe_folder`).
- **Send your key or files anywhere.** The container runs with `--network none`. The broker
  forwards only `POST /v1/messages`, the one call Gate 0 saw the real CLI make (Claude Code 2.1.316). It refuses
  `..`, `%2e`, `//`, absolute-form targets, smuggled or oversized requests, server-side tools,
  `mcp_servers` and bodies with repeated keys, and drops unknown `anthropic-beta` values.
- **Run up an unbounded bill in one reply.** Each reply may make 200 model requests (configurable);
  the next one gets a 429 and the chat says the reply hit its limit.
- **Leave a trap for kwnt on this computer.** Every folder an isolated reply used is recorded.
  In such a folder, a folder inside it or one holding it:
  - Claude on this computer starts with `--setting-sources user --strict-mcp-config`, so the
    folder's own `.claude/settings*.json` hooks and `.mcp.json` servers don't load;
  - Codex and Grok don't start at all (`folder_used_by_isolated_bot`);
  - a mission's command check, the Dev backup command, and a stdio MCP server whose program or
    arguments point at a file in the folder are refused;
  - kwnt's own git calls run with `GIT_CONFIG_NOSYSTEM=1 -c core.fsmonitor=false
    -c core.hooksPath=/dev/null -c core.pager=cat`;
  - kwnt reads bot folders (checkpoints, restore, team discovery) without following symlinks,
    one folder at a time, so a link or a swapped folder can't point a read outside. A checkpoint
    walk stops at 64 folders deep and at an entry limit, and a restore never deletes files the
    walk didn't reach.
- **Plant a git hook.** When the folder is a git repository (a real `.git` folder, not a link or
  a `.git` file), an empty read-only folder that only kwnt writes is mounted over its
  `.git/hooks` in the container: `git commit`, branches and the rest still work, but no hook can
  be added or changed. The mount's source is never a path inside the bot's folder, so a link the
  bot swaps in can't get a folder from your computer mounted. Your own hooks don't run for
  commits the bot makes inside the container; they still run for yours. A `.git/hooks` that is a
  link or a file is left as it is (kwnt's log says so).

**Forgetting a folder.** Only you can take a folder off the record, in Settings (`GET` and
`POST /api/isolated-folders/forget`; a bot gets 403). The list shows which files a program
started there would load (`.mcp.json`, `.envrc`, `.claude/settings*.json`, `.codex`, `.grok`,
`.git/config`, `.git/hooks`), so you can look at them first. A `.git` that is a file or a link,
or a `.git/hooks` that isn't a real folder, is listed too (`.git (file)`, `.git/hooks -> link`):
git then loads hooks from somewhere the list doesn't show. Once forgotten, kwnt trusts the
folder as before.

## What it doesn't protect against

- **Spending your credits.** The bot uses your real Claude account through the broker. The
  request cap bounds one reply, not a day of replies.
- **Actions through kwnt's own tools.** kwnt's MCP tools (browser, shell, files, teammates,
  approvals) are reachable through the broker, as they are without isolation. They still follow
  each bot's approval settings, so a bot that may run commands without asking still can.
- **kwnt's built-in shell.** `execute_shell` runs in kwnt's own executor container, which keeps
  its network. Isolating Claude doesn't change it.
- **Files you run later.** A bot can still write `.envrc`, package scripts or editor tasks in its
  folder. They run if **you** use them on your own computer.
- **Other ways around the read-only hooks.** `.git/config` stays writable (a bot needs `git remote`
  and branch tracking), so a bot can point `core.hooksPath` at a folder it writes, or rename
  `.git` and make a new one. kwnt's own git calls never run hooks; your own `git` there would. A
  repository the bot creates during a reply gets read-only hooks only from its next reply. The
  forget list shows `.git/config`, any hooks, and a `.git` file or link so you can check them.
- **Codex, Grok, and Claude API-key or Custom accounts.** They run on this computer as before.
- **A Docker or kernel escape.** The container drops every capability, runs as your user id with
  a read-only root and no network, but it shares the kernel.

## What changes for the bot

- **No WebSearch or WebFetch.** Claude starts with `--disallowedTools WebSearch,WebFetch`, because
  Anthropic's server-side tools reach the network from Anthropic's side. Bots use kwnt's browser
  tools instead.
- **Only kwnt's own MCP servers.** Outside servers (Notion and so on) are left out of the turn,
  since the container can't reach them. They still work with the setting off.
- **A new session.** The first isolated reply starts a fresh Claude session; `--resume` works
  from then on.
- **The first use sets up Claude's VM.** kwnt builds the image once from its executor image plus
  Claude Code (about 860 MB on disk). A reply sent meanwhile says so and asks you to retry;
  nothing runs on this computer.

## When it can't run

With the setting on, any failure stops the reply with a notice in the chat and nothing runs on
this computer. Turning the setting off is always yours to do. Codes (the `reason` of the
`turn_isolation_failed` chat notice and of each `agents.claude` chip):

| Code | Meaning |
|---|---|
| `preparing` | Claude's VM is still downloading |
| `download_failed` | The download failed; the next reply tries again |
| `docker_unavailable` | Docker isn't running |
| `relay_down` | kwnt's broker couldn't start |
| `container_failed` | The bot's container didn't start or stopped partway |
| `no_credential` / `sign_in_expired` | No Claude sign-in, or it expired and couldn't be refreshed |
| `unsafe_folder` | The folder is, holds or sits inside something private (`unsafe_kind`: `home`, `contains_home`, `root`, `sign_ins`, `kwant_data`, `docker`, `shared_folder`) |
| `folder_used_by_isolated_bot` | Codex or Grok was asked to run in a folder an isolated Claude used |
| `host_is_root` | kwnt runs as root; the container never does |
| `folder_name_unsupported` | The folder's name has a newline or isn't UTF-8 |
| `folder_record_failed` | kwnt couldn't note the folder (D4), so it didn't start |
| `unsupported_os` | Not Linux |

A reply that reaches the request cap ends with a `turn_request_limit` notice instead of a raw
429.

## Server settings

In `<data>/config.toml`:

```toml
[agents_in_vm]
request_limit = 200   # model requests per reply (env KWNT_AGENT_REQUEST_LIMIT)
memory_mb = 1024      # per bot container (env KWNT_AGENT_MEMORY_MB)
max_turns = 2         # isolated replies at once; the next one waits, and the chat says so (env KWNT_AGENT_MAX_TURNS)
```

## Claude on this computer, on a Mac (0.8.0)

Without the VM, a Claude turn on a Mac runs on the Mac itself. A bot talked into it by a web
page, a file or another bot could read a sign-in with its file tools or a shell command. kwnt
now passes Claude one more `--settings` layer for those turns (`src/drivers/mac_local.rs`):

- **Always on.** Claude's Read, Grep, Glob, Edit and Write tools are denied `~/.claude`,
  `~/.claude.json`, `~/.codex`, `~/.grok`, `~/Library/Keychains`, the profile's own sign-in
  home, and in kwnt's data folder (the one the server runs on, from `--data-dir`, config.toml's
  `data_dir`, `KWNT_DATA_DIR` or `~/.kwnt`) `profiles/`, `kwnt-vault/`, `backups/`, `mcp/`,
  `run/`, `kwnt-vault.key`, `api-token` and `kwnt.db` with its `-wal` and `-shm` files (plus
  `$CLAUDE_CONFIG_DIR`, `$CODEX_HOME` and `$GROK_HOME` when set). What kwnt loads or runs at its
  next start, `config.toml`, `laya/` and `bin/`, stays readable but can't be changed. Where a
  place is reached through a link, its resolved path is listed too. They're `Read(…)`/`Edit(…)`
  deny rules, which outrank anything the folder's or your own settings allow.
  - **Without `KWNT_MAC_SEATBELT` these rules cover Claude's file tools only.** A shell command
    you approve can still read these places and Keychain items.
  - Claude's auto memory is turned off for these turns (`autoMemoryEnabled: false`): it writes
    inside the denied `~/.claude`. Your personal skills and plugins under `~/.claude` still load,
    but Claude's file tools can't open their extra files in these turns.
- **Opt-in: Seatbelt for Bash** (`KWNT_MAC_SEATBELT=1` in kwnt's environment). Claude runs each
  shell command in macOS's sandbox with the same paths unreadable and unwritable (kwnt's start
  files: unwritable only). kwnt still
  approves every command as before (`autoAllowBashIfSandboxed: false`), and a bot can't ask to
  run a command outside the sandbox (`allowUnsandboxedCommands: false`). It's off by default
  until it has been tried on a real Mac, because with it on:
  - the shell's network goes through Claude's proxy, which asks before each new domain, and kwnt
    shows that as an approval card (`localhost` and `127.0.0.1` are allowed);
  - Unix sockets are blocked except `$SSH_AUTH_SOCK`, so Docker from a bot's shell fails;
  - writes outside your home folder fail.

What it doesn't do: the `claude` process itself still reads its own sign-in and the Keychain, as
it has to in order to run. This stops a tricked bot's tools, not the CLI. `~/.ssh` isn't denied,
so git over ssh keeps working. Only the VM keeps sign-ins off the machine the bot runs on.

**The Keychain is only half covered.** kwnt always stops Claude's file tools from reading the
`~/Library/Keychains` files, and with `KWNT_MAC_SEATBELT=1` it stops shell commands reading them
too. Claude keeps its sign-in in the Keychain itself ("Claude Code-credentials"), and a shell
command reaches items through macOS's security service, not the files. That isn't proven safe:
- On CI's Mac runner, a sandboxed `security find-generic-password` couldn't read the probe item
  ("does not exist").
- But that runner's keychain isn't a person's login or iCloud keychain, so this is untested on a
  real Mac.
- Without Seatbelt (the default), a shell command you approve can still read Claude's
  "Claude Code-credentials" item.

**Checked on CI's Mac** (`.github/workflows/mac-local.yml`, `scripts/mac_local_probe.sh`): the unit
tests run on macOS; the real Claude Code reads kwnt's settings (its debug log reports the deny
rules); sandbox-runtime, which Claude's Bash sandbox is built on, gets the same sections and has
to block reads and writes of the denied places, still allow an ordinary home file, and block an
unlisted domain; and the job reports whether a sandboxed `security find-generic-password` reads a
test Keychain item. No Claude sign-in exists on CI, so no real turn runs there.

## Who refreshes a sign-in

While the setting is on, the broker is the only thing that refreshes a bot's own Claude sign-in
(a profile's sign-in home). It refreshes once at a time per file, re-reads the file under a lock
and writes the new token with mode 600. It never refreshes this computer's own `~/.claude`
sign-in: when that has expired, the reply stops with `sign_in_expired`, and you sign in again on
this computer.

## Files

- `src/sandbox/agent_vm/`: the VM (`mod.rs`), the broker (`broker.rs`), the folder rule
  (`folder_rule.rs`) and the chat notices (`notice.rs`).
- `src/sandbox/safe_fs.rs`: the no-follow folder reader.
- `src/db/isolated_folders.rs`: the record of folders isolated replies used.
- `src/server/isolated_folders_routes.rs`: the person's list and Forget
  (`release-080/forget-folder-contract.md`).
- `src/drivers/mac_local.rs`: the Mac hardening for Claude turns on this computer; its CI check is
  `.github/workflows/mac-local.yml` with `scripts/mac_local_probe.sh`.
- `docker/agent.Dockerfile`, `docker/agent-entry.sh`: the image (`kwnt-agent:v1-claude-<version>`).
- `proto/agents-in-vm/`: the Python spike, `gate0.sh` and the evidence it wrote.
