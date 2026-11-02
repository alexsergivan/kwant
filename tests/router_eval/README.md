# Router eval

47 messages with the expected routing, across 3 fixture spaces (`kitchen` mirrors the corn-dog screenshot,
`software`, `duo`). 15 expect `concierge`, 2 expect `ask`. The runner is `tests/router_eval_test.rs`.

    cargo test --test router_eval_test router_eval -- --ignored --nocapture

Prints the misroute rate per space, the no-match rate and every failure. `ROUTER_EVAL_MAX_MISROUTE=0.05`
turns it into a gate. A *misroute* is a wrong destination; a *basis mismatch* is the right destination
for the wrong stated reason, reported separately.

`expect.route` is a member name, `concierge` or `ask`. `expect.basis` lists accepted bases (omitted = any).
`laya` fixes a confident titles-only Laya guess (case 1 reproduces the screenshot). `channel` is `general`,
a members' channel name or `dm`: the runner picks, then settles on fit as the server does, with a Concierge
in #general; a DM goes to its pick (see `route_case`).

`router_eval_must_pass` runs in CI and fails if any case in its `MUST_PASS` list misroutes.
