# Saved passwords

Kwnt can fill website logins in a team's, channel's or bot's browser. The passwords live in one of two stores:

| Store | Where | Default |
|---|---|---|
| Built-in | encrypted files in `<data>/kwnt-vault/`, with the key next to them | yes |
| Bitwarden | the person's own Bitwarden (or Vaultwarden) account, folder `kwnt` | after the person unlocks it once |

The built-in store protects against copies of the data folder. It does **not** protect against a bot that runs as the same Linux user with approvals off. That bot can read the key file, and the API token next to it, and with them every built-in saved password (audit F-10, 2026-10-20). Kwnt's name filter on those files is not enough: shell wildcards get past it.

## Approvals off needs Bitwarden

Since 2026-10-20, approvals off and passwords in the built-in store are not allowed together. The server enforces this:

- Switching approvals off returns 409 `{error, needs_bitwarden: true, local_count}` while the built-in store holds any password. This covers a space, a bot, and a bot set to `inherit` from a space that is off. It also applies in Bitwarden mode when built-in passwords were never moved: they still open with the built-in key. Move them with `POST /api/passwords/migrate` first.
- While any space or bot has approvals off, saving into the built-in store (`POST …/passwords` or `…/passwords/pending`) returns 409 `{error, needs_bitwarden: true}`. A captured login keeps waiting.
- While any space or bot has approvals off, going back from Bitwarden to the built-in store (`DELETE /api/passwords/bitwarden`) returns 409 `{error, needs_bitwarden: true, local_count}`, unless the built-in store is empty.
- A space that is off counts even if every bot in it overrides to `ask`, because a new bot there would inherit `off`.
- An install that was already in this state keeps its approvals as they are. `GET /api/passwords/store` shows `exposed: true`, and kwnt logs one warning at start (a count, no secrets). Moving the passwords to Bitwarden or turning approvals back on clears it.

With Bitwarden, the API token alone doesn't show a password: that needs the master password. The DevTools gap below still applies in Bitwarden mode.

## Bitwarden: what holds and what doesn't

- Kwnt talks to Bitwarden itself (an in-process Rust client), not through the `bw` CLI and not through `bw serve`.
  - The CLI puts `BW_SESSION` in a child's environment and leaves the protected key in `data.json`. In the spike, another process run as the same user recovered a password from them.
  - `bw serve` is an unauthenticated local listener.
- The master password, the session token and the decrypted vault key exist only in kwnt's memory. Nothing secret is written to disk, put in a child's env or argv, or logged.
  - `<data>/kwnt-vault/bitwarden.json` holds only the server URL, email, device id and optional API-key client id.
  - Key buffers are zeroed on drop.
- Kwnt calls `prctl(PR_SET_DUMPABLE, 0)` at start. This makes `/proc/<kwnt>/{environ,mem,maps,…}` root-owned, so a same-user process can't read kwnt's memory or attach with ptrace. Children (Chrome, CLIs) get the normal setting back on exec.
- After a restart the vault is **locked**:
  - nothing is filled;
  - the per-owner password routes answer 423;
  - the UI shows "Locked" until the person unlocks it again.
- Only items in the `kwnt` folder, named `kwnt · <host>` and carrying the custom field `kwnt-owner`, are used. Organisation items, trashed items and items in other folders are never read.
- Showing a password needs the master password, not only the API token. The token sits in kwnt's data folder, where a bot with approvals off can read it. Five wrong master passwords in a row lock the vault.
- Every vault route is person-only. A bot's turn token gets 403; no token gets 401.

**Known gap (open, accepted 2026-10-20):** autofill stays on while approvals are off; Oleks decided this. Chrome's DevTools port listens on loopback without authentication, so with approvals off a bot with a shell can attach to it and read a field kwnt has just filled. This holds for both the built-in store and Bitwarden, because the leak is in the browser, not the store. Planned fix, on its own branch: start Chrome with `--remote-debugging-pipe` so there is no port to attach to.

## HTTP API (person only)

Per owner (`:id` is a team, channel or bot id):

- `GET    /api/sandbox/:id/passwords` returns `{passwords: [{id, origin, domain, username, created_at, last_used}], pending: null | {origin, username, captured_at}}`.
- `POST   /api/sandbox/:id/passwords {origin | domain, username, password}` returns `{status: "saved", credential}`.
- `POST   /api/sandbox/:id/passwords/:cid/reveal` returns `{password}`.
  - In Bitwarden mode the body must be `{master_password}`.
  - A wrong one returns 401 `{wrong_password: true}`.
- `DELETE /api/sandbox/:id/passwords/:cid` returns `{status: "deleted" | "not_found"}`.
- `POST|DELETE /api/sandbox/:id/passwords/pending` saves or discards the captured login.
- While Bitwarden is locked, each of these returns 423 `{error, locked: true}`.

Vault:

- `GET /api/passwords/store` returns:

  ```json
  {"store": "local" | "bitwarden", "locked": bool, "local_count": n,
   "approvals_off": bool, "exposed": bool,
   "bitwarden": {"server_url", "email", "login": "password" | "api_key", "client_id",
                 "unlocked", "unlocked_at", "item_count", "last_sync"} | null}
  ```

  `approvals_off`: any space or bot has approvals off. `exposed`: approvals are off and `local_count > 0`.

- `POST /api/passwords/bitwarden/unlock` returns the store status.
  - Body: `{server_url?, email?, master_password, client_id?, client_secret?, two_factor_provider?, two_factor_token?, new_device_otp?}`.
  - `server_url` and `email` are needed only the first time. That first unlock also makes Bitwarden the store.
  - `server_url` must be https; plain http is accepted only for loopback.
  - Errors:
    - 401 `{wrong_password: true}`
    - 401 `{needs: "two_factor", providers}`
    - 401 `{needs: "new_device_otp"}`
    - 409 when another account is already set up
    - 400 for bad input
    - 502 when Bitwarden is unreachable or refuses
- `POST /api/passwords/bitwarden/lock` returns the store status.
- `DELETE /api/passwords/bitwarden {master_password}` goes back to the built-in store. The vault must be unlocked, and the items stay in Bitwarden. While approvals are off anywhere and the built-in store isn't empty, it returns 409 (see above).
- `POST /api/passwords/migrate` returns `{moved, failed}`. It runs once, when the person triggers it. Each built-in password is deleted locally only after Bitwarden has it.

## Tests

- `cargo test --test approvals_passwords_test` checks the approvals-off rules above.
- `cargo test bitwarden` runs the unit tests and route checks, with known-answer vectors for the key derivation and the cipher format.
- The end-to-end tests run against a throwaway Vaultwarden:

```sh
docker run -d --rm --name kwnt-bwtest -p 127.0.0.1:18990:80 \
  -e SIGNUPS_ALLOWED=true -e SIGNUPS_VERIFY=false -e I_REALLY_WANT_VOLATILE_STORAGE=true \
  vaultwarden/server:latest
KWNT_TEST_VAULTWARDEN=http://127.0.0.1:18990 cargo test --test bitwarden_test
docker rm -f kwnt-bwtest
```
