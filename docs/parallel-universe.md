# Parallel Universe

Compares up to three ways to deploy one app ("stay on managed hosting, move to a VPS, or go serverless?") without
a cloud account or any spending. The code is `src/parallel/`, `src/db/parallel.rs` and `src/server/parallel_routes.rs`.

**Status: slice 1.** This covers the scenario store, the container harness and the price table. The branch runner (one bot per option)
and the comparison report come next. The full design is in `/home/maus/shared/ship-software-safely/parallel-universe/design.md`.

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
2. It builds the app with `docker build --network none` (offline), or uses an image already on the machine.
3. It runs the app with `--cpus`, `--memory` (= `--memory-swap`), `--pids-limit`, `--cap-drop ALL` plus a few file/uid caps, and
   `no-new-privileges`. **Nothing is ever published on a host port.**
4. It waits for the health path (an oha probe, below 50 % of the time limit).
5. It runs the load: oha (`ghcr.io/hatoo/oha:1.16.0`, `docker pull` it once) in a second limited container on the same network.
6. It samples memory with `docker stats`.
7. It removes every container, the network and a built image on success, failure and timeout. Whatever can't be removed is
   returned in `cleanup_errors`. Every object carries the label `kwnt.pu.run=<run id>`, and `harness::sweep` removes leftovers by
   that label. It never touches `kwnt-browser-*` or `kwnt-executor-*`.

Limits per option: 0.1–2 CPUs, 64–2048 MB, 16–512 pids, 30–900 s. At most 3 options, reserving at most 3072 MB together.

## API (person-only: no token 401, bot or wrong token 403)

- `GET/POST /api/parallel/scenarios`, `GET/PUT/DELETE /api/parallel/scenarios/:id`. A body that fails validation gets a 400 `{error}`.
- `GET /api/parallel/prices` returns `{prices, stale_after_days}`.
- `POST /api/parallel/estimate {plan, traffic}` returns `{figures: [Figure]}`.

A scenario is `{title, question, requirements, traffic: {monthly_requests, peak_rps, avg_response_kb, avg_duration_ms},
load: {requests, concurrency, path}, criteria: [{key: cost|p95_latency|ops_burden|rollback, weight 0–10}], options: [{key,
label, plan?, app: {image | context (+dockerfile), port, health_path}, limits, ops_notes, rollback_notes}]}`.

## Tests

`cargo test --test parallel_test` covers the API. `-- --include-ignored` also runs the harness against a synthetic Python app. That needs
Docker, `kwnt-executor:v1` and the oha image. It covers measuring, cleanup after an app that never answers, and the network reaching
neither the internet nor the host.
