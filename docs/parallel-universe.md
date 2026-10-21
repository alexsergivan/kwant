# Parallel Universe

Compares up to three ways to deploy one app ("stay on managed hosting, move to a VPS, or go serverless?") without
a cloud account or any spending. The code is `src/parallel/`, `src/db/parallel.rs` and `src/server/parallel_routes.rs`.

**Status: slice 2.** This covers the scenario store, the container harness, the price table, runs and the comparison report.
The multi-bot advisor (a bot per option) is slice 3, if wanted. The full design is in `/home/maus/shared/ship-software-safely/parallel-universe/design.md`.

## Where numbers come from

Every number is a `Figure` with a `tag`:

| tag | meaning | carries |
|---|---|---|
| `verified` | measured on this machine by the harness | `command` (exact docker/oha argv), `raw_output` (oha's JSON, ≤ 64 KB), `measured_at` |
| `estimate` | computed from the price table | `source_url`, `as_of`, `stale` (older than 180 days), `assumptions` |
| `speculation` | neither measured nor priced | `note` |

Latency measured here is this server's, not the provider's. A report must say so.

## Price table

The table is `src/parallel/prices.json`, compiled in. Each entry was read by hand from the provider's public page on its `as_of` date. The
server never fetches prices. To update an entry, edit the JSON in a commit and give it a new `as_of`. `prices::check_table` (a unit
test) refuses an entry without an https source, with a future date or with negative prices. Currencies are not converted.

The table was seeded on 2026-10-20 with Hetzner CPX02/CPX12 (EUR, excl. VAT), DigitalOcean Basic 1/2 GiB, Fly.io shared-cpu-1x 512 MB/1 GB
and AWS Lambda x86 us-east-1.

## Container harness (`parallel::harness::run_branch`)

1. It creates `docker network create --internal`, a network with no default route, so there's no internet, no host and no production. Ports 8799/8800 can't be reached.
2. It builds the app with `docker build --network none` (offline), or uses an image already on the machine. A scenario can set
   `build_network: true` to give **the build step only** the default bridge network (never `host`), for apps that fetch packages
   while building. The app and load containers stay on the internal network either way, and each branch of a run says
   `build_network_used`.
3. It runs the app with `--cpus`, `--memory` (= `--memory-swap`), `--pids-limit`, `--cap-drop ALL` plus a few file/uid caps, and
   `no-new-privileges`. **Nothing is ever published on a host port.**
4. It waits for the health path (an oha probe, below 50 % of the time limit).
5. It runs the load: oha 1.16.0, pinned by digest (`harness::OHA_IMAGE`; `docker pull` it once), in a second limited container on the
   same network. Every `docker run` has `--pull never`, so nothing is downloaded during a run.
6. It samples memory with `docker stats`.
7. It removes every container, the network and a built image on success, failure and timeout. Whatever can't be removed is
   returned in `cleanup_errors`. Every object carries the label `kwnt.pu.run=<run id>`, and `harness::sweep` removes leftovers by
   that label. It never touches `kwnt-browser-*` or `kwnt-executor-*`.

Limits per option: 0.1–2 CPUs, 64–2048 MB, 16–512 pids, 30–900 s. At most 3 options, reserving at most 3072 MB together.

## Runs (`parallel::run`, table `pu_runs`)

- One run at a time, server-wide (`Runner`). Starting another while one is going is a 409 `{code: "busy"}`.
- The options run one after another, so only one branch's containers exist at a time.
- A run copies the scenario when it starts. Editing or deleting the scenario later doesn't change a past run.
- A run is `running`, then `finished` (every option was tried; each branch is `ok` or `failed` with an `error_code` from
  `HarnessError::code`), or `interrupted` if the server stopped. It is saved after every step, so polling shows progress.
- At the end, the run's label is swept once more. Anything that sweep removes or can't remove goes in `cleanup_errors`.
- At startup, runs left `running` become `interrupted`, and their label (and that of runs with cleanup errors) is swept.
  Only runs in this database are swept, so another kwnt on the same Docker keeps its containers.

## Comparison report (`run::report`, in `GET /api/parallel/runs/:id`)

Plain JSON: `{run_id, title, question, status, build_network_used, criteria, options: [{key, label, plan, status, error_code,
measured: [verified Figure], cost: estimate|speculation Figure, notes: [{criterion: ops_burden|rollback, text, tag:
"speculation", note}]}], leaders: [{criterion, option, value, unit, basis}], caveats: [string]}`.

- `leaders` ranks only what can be compared: p95 among options that answered every request (verified), and cost among
  estimates in one currency. Ops and rollback are never ranked.
- The person's own `ops_notes`/`rollback_notes` are used when set. Otherwise kwant gives a general note for the plan's kind.
  Either way it's tagged speculation.
- `caveats` always says the numbers are from this server, not the provider's. It also says when the build had the network,
  when prices are stale, when currencies differ, and when options failed.
- The cost is computed when you read the report, so a price goes stale over time.

## API (person-only: no token 401 from another machine, bot or wrong token 403)

- `GET/POST /api/parallel/scenarios`, `GET/PUT/DELETE /api/parallel/scenarios/:id`. A body that fails validation gets a 400 `{error}`.
- `GET /api/parallel/prices` returns `{prices, stale_after_days}`.
- `POST /api/parallel/estimate {plan, traffic}` returns `{figures: [Figure]}`.
- `POST /api/parallel/scenarios/:id/run` returns 202 `{run}`, or 409 `{error, code: "busy"}`.
- `GET /api/parallel/runs` and `GET /api/parallel/scenarios/:id/runs` return `{runs, running}` (newest 50).
- `GET /api/parallel/runs/:id` returns `{run, report}`. Poll it until `run.status` isn't `running`.

A scenario is `{title, question, requirements, traffic: {monthly_requests, peak_rps, avg_response_kb, avg_duration_ms},
load: {requests, concurrency, path}, criteria: [{key: cost|p95_latency|ops_burden|rollback, weight 0–10}], options: [{key,
label, plan?, app: {image | context (+dockerfile), port, health_path}, limits, ops_notes, rollback_notes}], build_network: bool}`.

## Tests

`cargo test --test parallel_test` covers the API and the run lifecycle with a fake harness, so it needs no Docker. `-- --include-ignored`
also runs the harness against a synthetic Python app. That needs Docker, `kwnt-executor:v1` and the oha image. It covers measuring,
cleanup after an app that never answers, the network reaching neither the internet nor the host, the build network only when
asked, and a full run through the routes with 0 labelled leftovers.
