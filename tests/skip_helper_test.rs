//! `skip_or_fail` and `strict()` (tests/support/skip.rs). Its own binary, because it changes CI and
//! KWNT_TEST_REQUIRE for the whole process.

#[path = "support/skip.rs"]
mod skip;

use skip::{opt_in, skip_or_fail, strict};

const KEYS: [&str; 3] = ["CI", "KWNT_TEST_REQUIRE", "KWNT_AGENT_TEST_REQUIRE"];

fn panic_text(f: impl FnOnce() + std::panic::UnwindSafe) -> Option<String> {
    std::panic::catch_unwind(f).err().map(|e| *e.downcast::<String>().unwrap())
}

// One test for every branch: env vars are process-wide, so separate tests would race.
#[test]
fn a_missing_precondition_skips_without_ci_and_fails_with_the_fix_in_ci() {
    let saved: Vec<_> = KEYS.iter().map(std::env::var_os).collect();
    for k in KEYS {
        std::env::remove_var(k);
    }
    assert!(!strict());
    assert_eq!(panic_text(|| skip_or_fail("a thing", "run `make thing`")), None, "skips without CI");

    for k in KEYS {
        for (v, want) in [("true", true), ("1", true), ("TRUE", true), ("0", false), ("false", false), ("", false)] {
            std::env::set_var(k, v);
            assert_eq!(strict(), want, "{k}={v}");
        }
        std::env::set_var(k, "1");
        let msg = panic_text(|| skip_or_fail("a thing", "run `make thing`")).unwrap_or_else(|| panic!("{k}=1 must fail"));
        assert!(msg.contains("a thing is missing") && msg.contains("run `make thing`") && msg.contains("CI=true"), "{msg}");
        std::env::remove_var(k);
    }

    // Opt-in live tests stay skipped in CI
    std::env::set_var("CI", "true");
    assert_eq!(opt_in("KWNT_TEST_SKIP_HELPER_NEVER_SET", "x"), None);
    std::env::set_var("KWNT_TEST_SKIP_HELPER_SET", "v");
    assert_eq!(opt_in("KWNT_TEST_SKIP_HELPER_SET", "x").as_deref(), Some("v"));

    for (k, v) in KEYS.iter().zip(saved) {
        match v {
            Some(v) => std::env::set_var(k, v),
            None => std::env::remove_var(k),
        }
    }
}
