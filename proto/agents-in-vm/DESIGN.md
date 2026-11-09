# Agents in the VM: CLI isolation + credential broker (design spike, Lin, 2026-11-08)

Branch `lin-078-agents-in-vm`, prototype in `proto/agents-in-vm/` (Python broker and mock for the spike; the real one is Rust).
One command re-runs the attack check: `proto/agents-in-vm/adversarial.sh` (3.4 s, prints `RESULT: PASS`, cleans up after itself).
Evidence: `proto/agents-in-vm/evidence/` (adversarial-run.txt, bench.txt, q2-q3-q6-notes.md).

## Decision

**Build it, Claude first, opt-in.** In vm mode, run the CLI inside the executor container with `--network none`, so only
the task folder and a private CLI home are mounted. The container's single way out is a host-side broker on a
bind-mounted Unix socket. The broker swaps a dummy credential for the real one, and only for an allow-list of
(method, path) pairs. Everything else gets a 403, and the real token never enters the container. This worked
end to end for Claude with both an API key and a subscription OAuth token (the host refreshes it), for Codex with
both an API key and ChatGPT sign-in, and up to the request layer for Grok. We don't rely on the CLIs' own sandboxes,
because those protect writes, not the token.

## Answers

**Q1 Claude without a real credential.** Proven against a header-logging mock.
- API key: the container gets `ANTHROPIC_API_KEY=sk-ant-api03-kwnt-dummy` and `ANTHROPIC_BASE_URL=http://127.0.0.1:8080/anthropic`. The broker drops `x-api-key` and `authorization` and adds the real key. The upstream logged `api_key_ok`, and the dummy never reached it.
- OAuth: the container gets `CLAUDE_CODE_OAUTH_TOKEN=sk-ant-oat01-kwnt-dummy`. The broker sends `Bearer <real access token>`, and `anthropic-beta: oauth-2025-04-20` passes through.
- Who refreshes: the host broker does, never the CLI. It refreshes before forwarding when the token expires within 60 s, and again once on a 401. Refresh runs single-flight under a lock and writes the credentials file atomically at 0600. Both paths were proven (`pre_refresh` and `on_401`), and the turn succeeded each time. The container is also denied the refresh endpoint itself (`deny_path /anthropic/v1/oauth/token`).
- `--resume` works across turns because the session store lives in the mounted CLI home. The MCP permission prompt (`mcp__kwnt__approve`) reaches the gateway through the same broker.

**Q2 Codex and Grok.**
- **Codex (API key and ChatGPT): yes.** Codex ignores `OPENAI_BASE_URL`, so kwant writes a custom `[model_providers]` entry into the container's `CODEX_HOME/config.toml`. For ChatGPT sign-in, use `requires_openai_auth=true` plus a dummy `auth.json` (unsigned JWTs). The broker sets the real bearer and `chatgpt-account-id`. Full turns passed in both modes. Side calls (plugins, wham/*, analytics) were denied and the turn still finished.
- **Grok, API key: very likely.** The real bearer was injected on /models, /chat/completions and /responses through `GROK_MODELS_BASE_URL` and `GROK_CLI_CHAT_PROXY_BASE_URL`. A full turn was not run because the mock doesn't speak xAI.
- **Grok, sign-in: partly.** `GROK_AUTH_PROVIDER_COMMAND` makes it send a dummy session token to our base URL, but I never reached its inference path.
- **Unproven for both:** host-side refresh of ChatGPT and Grok sign-ins against real endpoints.
- **Fallback (any CLI or mode that won't go through the broker):** keep that CLI on the host, as today, and show "not isolated" for it in Settings. We never copy a real token into the container.

**Q3 Default-deny network.**
- **Pick `--network none` + Unix socket + an in-container `socat` on 127.0.0.1:8080.** Proven: no DNS (curl rc 6), no IP route to 1.1.1.1 or the docker bridge (rc 7), no `host.docker.internal`, and a 403 for any host not on the allow-list.
- **`--internal` is rejected.** It blocks the internet, but on Linux the container could still open the host's bridge gateway (172.19.0.1:22 was open), so every host service listening on 0.0.0.0 is reachable.
- **MCP gateway:** reached as `http://127.0.0.1:8080/mcp`. The broker passes the per-turn gateway token through unchanged.
- **Docker Desktop on Mac:** bind-mounting a Unix socket into a container works there in principle (via the VM's file sharing), but it is **untested**. If it fails, the fallback is a loopback TCP port published only to the broker plus a per-container shared secret.
- **The person's allow-list** (npm, PyPI, GitHub) lives in the broker as extra routes in the same (host, method, path) table, edited in Settings › Computer and stored per bot. In v1 they are pass-through only, with no credentials added. Package installs then go through an `HTTPS_PROXY=http://127.0.0.1:8080` CONNECT route that the broker limits to those hostnames. That route is **not built** in the spike.

**Q4 Image.**
- **Layers:** `kwnt-executor:v1` + `socat` + the three CLIs. They are native binaries (claude is bun/glibc, codex and grok are static), so **no Node** is needed.
- **Size:** 1.46 GB on disk, 421 MB compressed; the executor alone is 376 MB. The build took 56 s.
- **Versions:** build once per kwant release with pinned CLI versions, as one image layer per CLI so an update re-downloads one layer. Turn the CLIs' auto-updaters off inside (`DISABLE_AUTOUPDATER=1`). They couldn't reach the internet anyway.
- **Startup cost:** a warm turn adds about 110 ms over the host (833 vs 708 ms median), and a cold start adds about 460 ms. Idle memory is 3.4 MiB, so keep one container per bot warm while vm mode is on.

**Q5 Files and approvals.**
- **Mounts:** `/workspace` is the task folder (rw). The CLI home is a per-bot host dir mounted at `/home/agent`: session store, `CLAUDE_CONFIG_DIR`, `CODEX_HOME` and settings, with no credentials in it. `/run/kwnt` is read-only: the socket and the per-turn MCP config.
- **Hardening:** `--read-only` root, tmpfs `/tmp`, `--cap-drop ALL`, `no-new-privileges`, a dedicated non-root uid.
- **Unchanged:** checkpoints and undo (`server/missions/checkpoints.rs` snapshots the host folder, and the files land there owned by the container uid, so kwant has to chown or map uids). Approvals and Flight School also stay the same: the permission prompt still arrives as an MCP call, and a `touch` was gated through the broker in the spike.
- **Known gap (same as today):** Claude's built-in read-only allow-list (cat, ls, echo) runs without asking kwant. In the container that only exposes the task folder, so isolation makes the gap harmless.

**Q6 The cheaper alternative (CLI sandboxes).**
- **They don't keep sign-in files safe.** The CLI process itself holds the real token in memory and reads its own credential file. The sandboxes limit what its tools write and where they connect; by default reads stay broad. Claude has `denyRead` settings, which help against a prompt-injected `cat ~/.claude/.credentials.json` but not against the process itself.
- **Codex's Linux sandbox couldn't start on this host.** bwrap failed with "RTM_NEWADDR: Operation not permitted" because Ubuntu 24.04 restricts unprivileged user namespaces. Expect the same on many users' Linux machines.
- **For Mac "local" mode:** it's a good hardening layer (Seatbelt works, plus `denyRead` on `~/.claude`, `~/.codex`, `~/.grok` and the Keychain). It is not a substitute for the broker. Recommend it as a separate small follow-up.

**Q7 First slice.** Opt-in setting, vm mode only, Claude only, off by default, Linux hosts first.
- **Backend, about 7 days (Lin):**
  - broker in Rust: hyper on a UnixListener, the route table, inject and refresh, logs hashed only; 2.5 d
  - agent image + build in CI; 1 d
  - `cli.rs` runs `docker exec` with the dummy env when the setting is on, and `computer.rs` adds `cli_agents` to `VM_ISOLATES`; 1.5 d
  - uid and chown for checkpoints, and container lifecycle via `sandbox/manager.rs`; 1 d
  - tests, including `adversarial.sh` ported to an integration test; 1 d
- **Frontend, about 1 day (Pixel):**
  - in Settings › Computer, under vm mode, one toggle, "Run Claude inside the VM too", with the hint "Your Claude sign-in stays on this computer; the bot only gets a stand-in."
  - a status chip: Isolated / Not isolated (shown for Codex and Grok until they ship)
  - an error state for "Docker not available"
  - Mira: one review of the copy
- **QA, about 1 day (Rigel):** run `adversarial.sh` and a real-key smoke test on Linux and Docker Desktop.

## Risks
- The bot can still spend the person's tokens: the broker hides the key, it doesn't ration it. Slice 1 caps each
  reply at 200 model requests (configurable), then answers 429 with `x-should-retry: false`; a day of replies is not capped.
- **Refresh ownership.** A Claude OAuth refresh token is single-use: when two processes refresh the same sign-in, the
  loser's next refresh fails and the person has to sign in again. While isolation is on, the broker must be the only
  refresher of a bot's sign-in home. It refreshes single-flight under a per-file lock inside kwnt, re-reads the file under that lock
  (so a token another process already rotated is used, not refreshed again) and writes it atomically at 0600. It never
  refreshes this computer's own `~/.claude` sign-in, which host Claude (and the person's own terminal) also refresh:
  when that one has expired, the reply stops with `sign_in_expired` and the person signs in on this computer.
  Residual risk: a host-mode turn of the same bot profile running at the same time could still refresh in parallel;
  the lock is inside kwnt (one per file), and the host CLI doesn't take it. Refresh was tested only against the mock, never with
  the team's real sign-ins (by rule).
- The OAuth refresh endpoint and the Codex and Grok side endpoints are private CLI internals and can change with any CLI release. Pin versions and keep a CI smoke test against the real CLIs with the mock.
- On a Mac, Claude keeps its credentials in the Keychain, so the broker needs a Keychain read (one permission prompt) or kwant's own sign-in.
- Docker Desktop socket forwarding is unproven, and so is the image size on slow links (about 860 MB on disk for the
  slice-1 image, built locally from the executor image plus one pinned Claude Code).
- Files written by the container uid need ownership fixed on the host.

## Still unproven
Proven since Gate 0 (2026-11-08): a real Claude turn and a `--resume` turn with an API key, through the Python spike broker
and through kwnt's Rust broker (`tests/agents_in_vm_real_api_test.rs`, runs only with `KWNT_GATE0_KEY_FILE`).
Still unproven: a Claude sign-in (OAuth) against the real API, and token refresh (never tested with the team's sign-ins on
this server). Long-session compaction (`count_tokens` is refused until it's seen). The approval round trip end to end.
Mac and Docker Desktop. A full Grok turn, and Grok sign-in inference. Refresh for ChatGPT and Grok. The allow-list CONNECT
proxy. The Claude sandbox's read policy in practice.

## Open questions for Oleks
1. Ship the first slice for Linux only and add Mac after a Docker Desktop test on your Mac? *Default: yes.*
2. Is a 421 MB one-time image pull acceptable, or should the CLIs download on first use? *Default: ship in the image.*
3. Should vm mode's isolated Claude become the default once proven, or stay opt-in? *Default: opt-in for 0.7.7.*

## Slice 1 as built (branch `lin-078-agents-in-vm`)

Rust, in `src/sandbox/agent_vm/` (broker, folder rule, notices, VM), wired in `drivers/cli.rs`, `server/computer.rs`
and `sandbox/manager.rs`. Off by default, Claude sign-ins only, Linux only. User-facing summary and error codes:
`docs/agents-in-vm.md`. Binding decisions D1-D5 from Ada's review are implemented as written there; the PR lists the
field names, statuses, error codes and the host git calls.

## Gate 0: the real API

`proto/agents-in-vm/gate0.sh` runs the spike broker against `https://api.anthropic.com` with Haiku: a normal turn, a
`--resume` turn and one Bash tool call (about six calls; it took four), with `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` and
`DISABLE_AUTOUPDATER=1`. It writes `evidence/real-upstream.txt`: every (method, path, status) the CLI sent, denied ones
included, with hashes only, plus the tool types and `anthropic-beta` values seen and the container's peak memory.

**Status: passed, 2026-11-08 16:30 UTC** (CLI 2.1.316, `claude-haiku-5-5`, API-key mode, Oleks's key).

| Turn | Result | Calls |
|---|---|---|
| 1, normal | success, reply "gate0 ok" | 1 |
| 2, `--resume` | success, recalled turn 1's reply | 1 |
| 3, Bash tool | success; `probe.txt` written in the folder, owned by the host uid:gid (1003:1003) | 2 |

- **Calls.** 4 in all, every one `POST /anthropic/v1/messages?beta=true` → 200. No other path, no other host
  (the proxy variables point at the broker, so a call elsewhere would have been logged as a refused `CONNECT`), no
  `count_tokens`, no telemetry.
- **Derived.** `ANTHROPIC_ALLOWED` is now only `POST /v1/messages` (`count_tokens` is dropped: never seen).
  `ALLOWED_TOOL_TYPES` stays `["custom"]` (the only type seen; 11 tools per call). `KNOWN_BETAS` is the 9 values
  seen (`advanced-tool-use-2025-11-20`, `claude-code-20250219`, `context-management-2025-06-27`,
  `effort-2025-11-24`, `interleaved-thinking-2025-05-14`, `mid-conversation-tool-changes-2026-07-01`,
  `per-turn-control-2026-07-01`, `thinking-display-updates-2026-08-18`, `thinking-token-count-2026-05-13`) plus
  `oauth-2025-04-20`, which only a Claude sign-in sends. The forwarded-header list dropped `x-client-request-id`
  (never sent). The broker test `the_anthropic_lists_match_the_gate_0_evidence` fails if the lists and the
  evidence drift apart.
- **Body.** Top-level keys seen: `context_management, max_tokens, messages, metadata, model, output_config, stream,
  system, thinking, tools`; content blocks `text, tool_use, tool_result`. No `mcp_servers`, no `container`, no
  server tool, so the broker's 400s cost nothing.
- **Memory.** The container's `memory.peak` over the three turns was 124.5 MiB, far under 716.8 MB (70% of 1024), so
  `memory_mb` stays 1024.
- **Key.** The only copy the script made (the spike broker's 0600 creds file) was shredded on exit. A scan of the
  worktree, the logs, this folder and the branch's git history for the key or its first or last 24 characters
  found nothing, and it never appeared in the process list.
- **Not covered:** a Claude sign-in (OAuth) turn and the broker's refresh against the real token endpoint (we don't
  refresh-test with the team's sign-ins on this server), and long sessions that compact.

## Mac (design only, not in slice 1)

Settings shows the switch disabled with "Coming to Mac later". What a Mac version needs:

- **Docker Desktop.** Bind-mounting the broker's Unix socket into a container goes through Docker Desktop's file
  sharing (virtiofs). Unproven. If it fails: publish a loopback-only TCP port for the broker and give each container
  a per-turn shared secret in the URL path (the `/t/<key>/` prefix already does this), never a credential.
- **Sign-in source.** Claude keeps its Mac sign-in in the Keychain ("Claude Code-credentials"), not in
  `~/.claude/.credentials.json`. The broker reads it with one Keychain prompt the person approves, or uses a kwnt-owned
  sign-in home per bot profile (preferred: no Keychain access, and the broker owns refresh as on Linux).
- **Ownership.** Docker Desktop maps file ownership itself, so `--user uid:gid` is not needed for files to land as the
  person; it still keeps the CLI non-root.
- **Image.** arm64 and x86_64 builds of the same pinned CLI; the agent image is built locally from the executor image,
  as on Linux.
- **Hardening for Mac "local" mode** (separate follow-up, no broker): Claude's Seatbelt sandbox plus `denyRead` on
  `~/.claude`, `~/.codex`, `~/.grok` and the Keychain. It doesn't keep the token from the CLI process itself.
- **Test before shipping:** the adversarial test and Gate 0 on Oleks's Mac with Docker Desktop.
