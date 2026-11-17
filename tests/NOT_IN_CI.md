# Tests the rust job doesn't run

CI runs strict (`CI=true`, and the rust job also sets `KWNT_TEST_REQUIRE=1`): a test whose
precondition is missing fails through `skip::skip_or_fail` instead of passing having run nothing
(`tests/support/skip.rs`). What follows is every test the rust job's `cargo test` leaves out, why,
and the exact command that runs it. `scripts/check_test_skips.py` fails CI when an `#[ignore]`d test
isn't listed here.

## Run by CI's `docker-tests` job

The rust job has no kwant images, so it skips these by name (`scripts/ci_docker_tests.sh list`) or
they are `#[ignore]`d. The `docker-tests` job builds the executor, agent and browser images
(`scripts/ci_docker_tests.sh images`) and runs them all strict (`scripts/ci_docker_tests.sh run`).
Locally, with Docker: `scripts/ci_docker_tests.sh images && scripts/ci_docker_tests.sh run`.

| Test | Needs | Command |
|---|---|---|
| `the_three_attacks_fail_in_a_shell_and_in_the_agents_own_bash` | Docker, the agent image | `KWNT_TEST_REQUIRE=1 cargo test --test agents_in_vm_adversarial_test` |
| `nothing_the_bot_started_outlives_the_containers_last_turn` | Docker, the agent image | same |
| `an_approval_goes_through_kwnt_and_both_answers_work` | Docker, the agent image | same |
| `every_failure_is_a_notice_and_nothing_runs_on_this_computer` | Docker, the agent image (its broker-down case) | `KWNT_TEST_REQUIRE=1 cargo test --test agents_in_vm_fail_closed_test` |
| `in_a_real_container_a_hook_cant_be_written_but_git_commit_works` | Docker, the agent image | `KWNT_TEST_REQUIRE=1 cargo test --lib -- --exact sandbox::agent_vm::tests::in_a_real_container_a_hook_cant_be_written_but_git_commit_works` |
| `a_fresh_kwants_container_has_no_restart_and_labels_stops_at_shutdown_and_is_reaped_once_its_folder_is_gone` | Docker, the browser image; starts a container | `cargo test --test browser_container_test -- --ignored` |
| `a_vm_bots_download_from_the_browser_container_lands_in_its_own_folder` | Docker, the browser image; starts a container | `cargo test --test browser_container_download_test -- --ignored` |
| `live_chromium_stream_and_isolated_sessions` | Docker, the browser image | `cargo test --test browser_stream_test -- --ignored --exact live_chromium_stream_and_isolated_sessions` |
| `harness_measures_a_synthetic_app_and_leaves_nothing_behind` | Docker, `kwnt-executor:v1`, the oha image | `cargo test --test parallel_test -- --ignored` |
| `an_app_that_never_answers_is_cleaned_up_too` | same | same |
| `the_internal_network_reaches_neither_the_internet_nor_the_host` | same | same |
| `the_build_reaches_the_network_only_when_asked` | same | same |
| `a_real_run_through_the_routes_leaves_nothing_behind` | same | same |

## Not in CI: run by hand before a release

These need the open internet, a person, or something CI can't build. They are on the release
checklist (QA).

| Test | Why not in CI | Command |
|---|---|---|
| `a_public_page_quote_opens_at_it` | needs the network (gnu.org) and Chrome | `cargo test --test browser_cite_test -- --ignored a_public_page_quote_opens_at_it` |
| `saucedemo_checkout_asks_before_finish` | needs the network (saucedemo.com) and Chrome | `cargo test --test browser_pay_gate_test -- --ignored saucedemo_checkout_asks_before_finish` |
| `saucedemo_says_what_changed` | same | `cargo test --test browser_progress_test -- --ignored saucedemo_says_what_changed` |
| `saucedemo_reads_are_cheaper` | same | `cargo test --test browser_reads_test -- --ignored saucedemo_reads_are_cheaper` |
| `saucedemo_by_refs` | same | `cargo test --test browser_refs_test -- --ignored saucedemo_by_refs` |
| `a_browser_container_from_an_older_image_is_replaced` | needs two older browser images, `kwnt-browser:v1` and `:v2`, built by hand | build both from `docker/browser.Dockerfile`, then `cargo test --test browser_stream_test -- --ignored --exact a_browser_container_from_an_older_image_is_replaced` |
| `live_registry_smoke` | talks to registry.modelcontextprotocol.io | `cargo test --test mcp_suggest_test -- --ignored live_registry_smoke` |
| `captcha_handoff_ui_harness` | manual: a live UI for Playwright or a phone (see the file comment) | `cargo test --test captcha_ui_harness -- --ignored --nocapture` |
| `inbox_timing` | a measurement, not a check; prints the median for a report | `cargo test --test inbox_away_b_test inbox_timing -- --ignored --nocapture` |
| `router_eval` | expected concierge/ask routes fail until the router overhaul lands | `cargo test --test router_eval_test -- --ignored router_eval` |
| `write_settings_for_the_mac_ci_probe` | not a check: `.github/workflows/mac-local.yml` runs it on macOS to write settings for the Seatbelt probe | run by that workflow |

## Opt-in live tests

They need a real service or binary named by an environment variable. Without it they print
`skipped: … runs only with VAR set` (`skip::opt_in`) and pass, in CI too. They never fail for a
missing variable.

| Test | Variable | Command |
|---|---|---|
| `a_real_turn_and_a_resume_pass_through_kwnts_broker` | `KWNT_GATE0_KEY_FILE`: a file with a real Anthropic API key | `KWNT_GATE0_KEY_FILE=/path/to/key cargo test --test agents_in_vm_real_api_test -- --nocapture` |
| `bitwarden_end_to_end`, `wrong_master_passwords_lock_the_vault` | `KWNT_TEST_VAULTWARDEN`: a Vaultwarden URL with a test account | `KWNT_TEST_VAULTWARDEN=http://127.0.0.1:<port> cargo test --test bitwarden_test` |
| `the_imap_source_reads_without_changing_anything` | `KWNT_TEST_IMAP`: an IMAP test server address | `KWNT_TEST_IMAP=127.0.0.1:<port> cargo test --test zombie_imap_e2e` |
| `j_s2_real_gitleaks_finds_runtime_built_aws_github_stripe_and_private_keys` | `KWANT_TEST_GITLEAKS`: the gitleaks 8.30.1 binary (CI can't fetch it) | `KWANT_TEST_GITLEAKS=/path/to/gitleaks-8.30.1 cargo test --lib real_gitleaks -- --ignored` |
| `j_s3_real_gitleaks_ignores_checksums_integrity_uuids_shas_and_base64_images` | same | same |
| `f_a_real_gitleaks_finds_and_hides_keys_in_paths_its_allowlist_skips` | same | same |
| `real_chrome_keeps_its_identity` | `KWNT_REAL_IDENTITY_DRIVER`: the driver script that checks pages, popups and workers | `KWNT_REAL_IDENTITY_DRIVER=/path/to/driver cargo test --lib real_chrome_keeps_its_identity -- --ignored` |
| `real_capped_browser_keeps_the_driven_page_working` | `KWNT_REAL_CDP`: a kwnt-browser container started for this run, under a memory cap | `KWNT_REAL_CDP=http://127.0.0.1:<port> cargo test --test browser_recovery_test -- --ignored real_capped_browser_keeps_the_driven_page_working` |
