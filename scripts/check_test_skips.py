#!/usr/bin/env python3
"""Fails when a Rust test skips outside tests/support/skip.rs, or when an ignored test isn't listed.

A skip and a pass have the same exit code, and `cargo test` hides a passing test's output, so a
test that prints "skipped" and returns looks green in CI having run nothing. Every early return
for a missing precondition goes through `skip::skip_or_fail` (fails in CI) or `skip::opt_in`
(live tests, never in CI). This check finds, in test code:

  1. a print!/println!/eprint!/eprintln! whose text says "skip";
  2. a direct read of the CI variable or KWNT_AGENT_TEST_REQUIRE (use `skip::strict()`);
  3. an #[ignore]d test whose name isn't in tests/NOT_IN_CI.md.

Test code is everything under tests/, and in src/ the part of a file from its first
`#[cfg(test)]` on (plus files named tests.rs or under a tests/ folder).

  python3 scripts/check_test_skips.py [repo root]
"""
import re
import sys
from pathlib import Path

HELPER = Path("tests/support/skip.rs")
HELPER_TEST = Path("tests/skip_helper_test.rs")
NOT_IN_CI = Path("tests/NOT_IN_CI.md")
PRINT = re.compile(r'\b(?:e?print(?:ln)?)!\s*\(\s*"((?:[^"\\]|\\.)*)"', re.S)
CI_READ = re.compile(r'\bvar(?:_os)?\s*\(\s*"CI"\s*\)|KWNT_AGENT_TEST_REQUIRE')
IGNORE = re.compile(r'^\s*#\[ignore\b')
FN = re.compile(r'\bfn\s+(\w+)')


def test_code(root: Path):
    """(path, first test line index, lines) for every Rust file with test code."""
    for p in sorted((root / "tests").rglob("*.rs")):
        if p.relative_to(root) not in (HELPER, HELPER_TEST):
            yield p, 0, p.read_text(encoding="utf-8").split("\n")
    for p in sorted((root / "src").rglob("*.rs")):
        lines = p.read_text(encoding="utf-8").split("\n")
        if p.name == "tests.rs" or "tests" in p.relative_to(root).parts[:-1]:
            yield p, 0, lines
            continue
        start = next((i for i, l in enumerate(lines) if l.strip() == "#[cfg(test)]"), None)
        if start is not None:
            yield p, start, lines


def ignored_names(lines, start):
    """(line number, test name) for each #[ignore] attribute from `start` on."""
    for i in range(start, len(lines)):
        if IGNORE.match(lines[i]):
            for nxt in lines[i + 1:i + 6]:
                m = FN.search(nxt)
                if m:
                    yield i + 1, m.group(1)
                    break


def check(root: Path):
    problems = []
    listed = (root / NOT_IN_CI).read_text(encoding="utf-8") if (root / NOT_IN_CI).exists() else ""
    for path, start, lines in test_code(root):
        rel = path.relative_to(root)
        body = "\n".join(lines[start:])
        offset = start + 1
        for m in PRINT.finditer(body):
            if "skip" in m.group(1).lower():
                line = offset + body.count("\n", 0, m.start())
                problems.append(f"{rel}:{line}: a test prints a skip itself; call skip::skip_or_fail(what, fix) "
                                f"(fails in CI) or skip::opt_in(var, what) for a live test (see {HELPER})")
        for m in CI_READ.finditer(body):
            line = offset + body.count("\n", 0, m.start())
            problems.append(f"{rel}:{line}: reads CI or KWNT_AGENT_TEST_REQUIRE directly; use skip::strict() ({HELPER})")
        for line, name in ignored_names(lines, start):
            if f"`{name}`" not in listed:
                problems.append(f"{rel}:{line}: #[ignore]d test `{name}` is not listed in {NOT_IN_CI}; "
                                f"add it with why it can't run in CI and the command that runs it")
    return problems


def main(argv):
    root = Path(argv[1]) if len(argv) > 1 else Path(__file__).resolve().parent.parent
    problems = check(root)
    for p in problems:
        print(p, file=sys.stderr)
    if problems:
        print(f"check_test_skips.py: {len(problems)} problem(s)", file=sys.stderr)
        return 1
    print("check_test_skips.py: every test skip goes through tests/support/skip.rs; every #[ignore] is listed")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
