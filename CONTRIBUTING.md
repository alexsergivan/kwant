# Contributing to kwant

Thanks for helping. Bug reports, ideas, docs fixes and code are all welcome.

- **Found a bug?** Open an issue with the *Bug report* template.
- **Have an idea?** Open an issue with the *Feature request* template, or start a discussion if it's still vague.
- **Found a security problem?** Don't open an issue. Follow [SECURITY.md](SECURITY.md).

Everyone taking part follows the [Code of Conduct](CODE_OF_CONDUCT.md).

## Set up

You need **Rust** (stable, from [rustup](https://rustup.rs)) and **Node.js 20 or newer**. Docker is only needed for VM mode, the shared browser and a few tests that skip themselves without it.

```bash
git clone https://github.com/alexsergivan/kwant.git
cd kwant
make run                 # builds the web app, then runs `cargo run -- serve` on http://127.0.0.1:8799
```

Add `--no-laya` (`cargo run -- serve --no-laya`) to skip installing the Laya router while you work. Bots are then picked by keyword.

For the web app with hot reload, run kwant on port 8799 in one terminal and Vite in another:

```bash
cd frontend
npm install
npm run dev              # http://localhost:5199, proxies /api to kwant on 8799
```

## Project layout

| Path | What's there |
|---|---|
| `src/server/` | Axum HTTP and WebSocket routes |
| `src/drivers/` | Engines: API drivers and the `claude`, `codex` and `grok` CLI drivers |
| `src/router/` | Laya routing and the keyword fallback |
| `src/db/` | SQLite storage and migrations |
| `src/profiles/` | Accounts (LLM profiles), failover and sign-in |
| `src/sandbox/` | VM mode, the shared browser and saved passwords |
| `frontend/src/` | The React web app, embedded into the binary at build time |
| `tests/` | Rust integration tests |
| `frontend/tests/` | Web app tests (`node --test`) |
| `docs/` | Feature and API docs |

## Test

```bash
make frontend            # once: cargo test needs frontend/dist, because the binary embeds it
cargo test               # Rust unit and integration tests
cd frontend && npm test  # web app tests
cd frontend && npm run build   # type-checks with tsc, then builds
```

Tests that need Docker or a network service skip themselves or are marked `#[ignore]` when it isn't there. `cargo test -- --ignored` runs those if you have what they need.

CI runs the same commands on every pull request: see [.github/workflows/ci.yml](.github/workflows/ci.yml).

## Style

- **Rust:** keep to the style of the file you're editing. `cargo clippy --all-targets -- -D warnings` must pass: the code has no Clippy warnings today, and CI keeps it that way. The codebase isn't fully `rustfmt`-formatted yet, so format only the code you change rather than whole files; that keeps diffs reviewable.
- **TypeScript:** `npm run build` must pass with no type errors. Tailwind classes, no new CSS frameworks.
- **Interface text:** short, plain sentences. Name things the way the app already does (bots, spaces, accounts, routines).
- **Tests:** add or update a test for every behaviour change. Use neutral sample data: names like "Alex", `example.com` addresses, `/Users/alex/project` paths, and obviously fake keys. Never commit a real key, token, password or personal email.
- **Security-sensitive changes** (approvals, tokens, the vault, the sandbox, person-only routes) need a test that shows the old hole is closed.

## Send a pull request

1. Fork the repository and make a branch from `main`.
2. Keep the change focused: one fix or feature per pull request.
3. Make sure `cargo test`, `npm test` and `npm run build` pass.
4. Update the docs (`README.md` or `docs/`) if the change is visible to users.
5. Open the pull request and fill in the template. Add screenshots for interface changes, at desktop and phone width.

A maintainer will review it. Small follow-up commits are fine; we squash or rebase when merging.

## License

kwant is licensed under the [GNU Affero General Public License v3.0](LICENSE) and is also offered under commercial licenses. By submitting a contribution, you license it to the kwant maintainer under the Apache License 2.0, and you agree that it may be distributed as part of kwant under the AGPL-3.0 and under commercial licenses. You confirm that you have the right to do so.
