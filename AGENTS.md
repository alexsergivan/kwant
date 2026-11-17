# AGENTS.md

Rules for anyone, person or agent, who adds or changes kwant's tests. CONTRIBUTING.md covers the rest.

## Tests

- **A new test fails without the change.** Run it on the code before your change and see it fail;
  a test that passes either way checks nothing.
- **No fixed sleeps.** Wait for the condition (a file, a port, a message, a state) with a deadline,
  never `sleep(…)` and hope.
- **Run each new test 3 times before the PR**, e.g. `for i in 1 2 3; do cargo test --test x_test name || break; done`.
- **A missing precondition goes through `skip::skip_or_fail(what, fix)`** (`tests/support/skip.rs`).
  It skips on your computer and fails in CI (`CI=true` or `KWNT_TEST_REQUIRE=1`), naming what's
  missing and how to provide it. A live test that needs a real service uses `skip::opt_in(var, what)`.
  Never print "skipped" and return: `scripts/check_test_skips.py` fails CI on it.
- **An `#[ignore]`d test is listed in `tests/NOT_IN_CI.md`** with why it can't run in CI and the
  exact command that runs it.
- **Fail loudly, no silent fallback.** A test, or the code it checks, that can't do what it was
  asked says so and fails; it doesn't quietly do something else and pass.
- **Find the root cause before fixing a symptom.** A retry, a longer timeout or a skipped case is
  not a fix until you know why it failed.

Adapted from agent-substrate's AGENTS.md and docs/ci-fail-closed.md (Apache-2.0).
