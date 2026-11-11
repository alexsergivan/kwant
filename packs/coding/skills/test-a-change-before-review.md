---
name: "Test a change before review"
description: "Checks a branch or pull request against its done-when lines, riskiest cases first, and says if it's ready for the person."
owner: "QA"
---
Goal
Tell the person, with evidence, whether a change does what it should and what it might break.

When to use
- An engineer on the team says a change is ready.
- The person asks "is this ready?" or "test this branch".

Inputs
- change: the branch or pull request.
- done_when: the lines from the plan. If there's no plan, ask TechLead for them.

Steps
1. Read the diff and the plan. List what changed in 3 lines or fewer.
2. List the riskiest cases first, at most 8: invalid or empty input, permissions, two things happening at once, slow or failed network, phone widths for screens, and nearby features that use the same code.
3. Run the project's own test commands (from the README, package.json, Makefile or CI file). Record the exact commands and results.
4. For each done-when line, check it and mark it met, not met, or unclear, with the evidence (a test name, an output, or the steps you followed).
5. Add missing tests where they belong, on the same branch, in a separate commit.
6. Report:
   - Verdict: Ready for review, or Not ready, with the reason in one line.
   - Done-when lines with their marks.
   - Risky cases tried and what happened.
   - Commands run and results.
   - Bugs found, each with steps to reproduce.

Checks
- Every verdict line has evidence.
- Failures quote the real output, shortened, not a paraphrase.
- You didn't change the feature code itself, only tests.

What can go wrong
- Tests need a secret, a paid service or production data: skip those, say which, and ask the person.
- The test suite is already failing on the main branch: say so and compare against it instead of blaming the change.
- Flaky test: run it 3 times and report all 3 results.

Never
- Merge, approve on the person's behalf, or push to the main branch.
