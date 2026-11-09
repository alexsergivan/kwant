// 0.8.0 B3 demo data: pre-push cards shaped exactly like Juno's `card_json` (ba2157a5,
// src/server/dev/prepush.rs) and fresh-eyes labels from fresh-eyes-audit.md §3. Demo names only
// (bot Pixel, reviewer Iris, branch pixel/inbox-empty-state, a fake key in a fixture file).

export const PP_T0 = Date.UTC(2026, 10, 9, 14, 0);
const min = 60_000;

const reviewer = { name: 'Iris', engine: 'claude', model: 'Claude Opus' };
const fe = { checked: true, saw: ['diff'], spec_source: 'pr', reviewer_kwant_id: 'k_iris' };
const diffstat = { files: 4, add: 86, del: 21 };

const KEY_FINDING = {
  id: 'g1', source: 'gitleaks', severity: 'critical', title: 'A key-shaped string in a test fixture',
  file: 'frontend/tests/fixtures/inbox.json', line: 14, detail: 'Looks like an API key (sk-demo…). Use a placeholder.', state: 'open',
};
const NIT = { id: 'r1', source: 'review', severity: 'low', title: 'The empty state copy says "No items" twice', file: 'frontend/src/components/InboxPage.tsx', line: 88, detail: null, state: 'open' };

const pass = (key: string, text: string) => ({ key, state: 'pass', text, required: key !== 'semgrep' });
const row = (key: string, state: string, text: string) => ({ key, state, text, required: key !== 'semgrep' });

const ALL_PASS = [
  pass('secrets', 'No keys or tokens in the diff'),
  pass('pkgs', "No new package that doesn't exist"),
  pass('tests', 'No tests removed, skipped or weakened'),
  pass('risk', 'Low risk'),
  pass('review', 'Nothing to fix, 1 note'),
  row('semgrep', 'not_installed', 'Not installed. Optional, never blocks a push.'),
];

const KEY_HIT = [row('secrets', 'hit', '1 possible secret in the diff'), ...ALL_PASS.slice(1)];

const inner = (findings: unknown[], extra: Record<string, unknown> = {}) => ({
  kind: 'prepush',
  reviewer,
  fresh_eyes: fe,
  findings,
  diffstat,
  security: { secrets: { state: 'pass', count: 0 }, packages: { state: 'pass', count: 0 }, semgrep: { state: 'not_run', count: 0, reason: "semgrep isn't installed on the server" } },
  error: null,
  ...extra,
});

const base = {
  space_id: 's_demo',
  kwant_id: 'k_pixel',
  branch: 'pixel/inbox-empty-state',
  head_sha: '3f9c2a1b7e5d4c3b2a1908f7e6d5c4b3a2918070',
  base_sha: '1a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d',
  tries: 1,
  needs_you: false,
  no_change: false,
  diffstat,
  blockers: [] as unknown[],
  override: null as unknown,
  overridden_by: null as string | null,
  overridden_at: null as number | null,
  created_at: PP_T0,
  updated_at: PP_T0 + 2 * min,
};

export const PP_CHECKING = {
  ...base, id: 'pp_check', status: 'checking', diffstat: null, card: null, updated_at: PP_T0,
  checks: ['secrets', 'pkgs', 'tests', 'risk', 'review', 'semgrep'].map((k) => row(k, 'running', 'Checking…')),
};

export const PP_NO_CHANGE = {
  ...base, id: 'pp_none', status: 'clear', no_change: true, diffstat: { files: 0, add: 0, del: 0 }, checks: [],
  card: { kind: 'prepush', no_change: true, diffstat: { files: 0, add: 0, del: 0 }, blockers: [] },
};

export const PP_CLEAR = { ...base, id: 'pp_clear', status: 'clear', checks: ALL_PASS, card: inner([NIT]) };

const keyBlocker = { source: 'gitleaks', severity: 'critical', title: KEY_FINDING.title, file: KEY_FINDING.file, line: KEY_FINDING.line };

export const PP_BLOCKED = {
  ...base, id: 'pp_blocked', status: 'blocked', checks: KEY_HIT, blockers: [keyBlocker],
  card: inner([KEY_FINDING, NIT], { security: { secrets: { state: 'hit', count: 1 }, packages: { state: 'pass', count: 0 }, semgrep: { state: 'not_run', count: 0, reason: "semgrep isn't installed on the server" } } }),
};

export const PP_NEEDS_YOU = { ...PP_BLOCKED, id: 'pp_needs', tries: 2, needs_you: true, created_at: PP_T0 + 6 * min, updated_at: PP_T0 + 8 * min };

/** Several rows hit: the tests row and the review row, plus semgrep with the repo's own rules. */
export const PP_MANY = {
  ...base, id: 'pp_many', status: 'blocked',
  checks: [
    pass('secrets', 'No keys or tokens in the diff'),
    pass('pkgs', "No new package that doesn't exist"),
    row('tests', 'hit', '1 test removed, skipped or weakened'),
    row('risk', 'hit', 'High risk: touches sign-in'),
    row('review', 'hit', '1 finding to fix'),
    row('semgrep', 'hit', '1 match'),
  ],
  blockers: [
    { source: 'review', severity: 'high', title: 'Sign-in redirect drops the return path', file: 'frontend/src/auth.ts', line: 42 },
  ],
  card: inner([
    { id: 't1', source: 'tamper', severity: 'medium', title: 'A test now skips the empty inbox case', file: 'frontend/tests/inbox.test.mjs', line: 31, state: 'open' },
    { id: 'r2', source: 'review', severity: 'high', title: 'Sign-in redirect drops the return path', file: 'frontend/src/auth.ts', line: 42, detail: 'After sign-in the person lands on Home, not the Inbox they opened.', state: 'open' },
    { id: 's1', source: 'semgrep', severity: 'low', title: 'innerHTML set from a variable', file: 'frontend/src/components/InboxPage.tsx', line: 120, state: 'open' },
  ]),
};

const why = 'Didn\'t run: the checks took longer than 12 minutes';
export const PP_FAILED = {
  ...base, id: 'pp_failed', status: 'failed', diffstat: null,
  checks: ['secrets', 'pkgs', 'tests', 'risk', 'review', 'semgrep'].map((k) => row(k, 'error', why)),
  blockers: [{ source: 'kwant', severity: 'high', title: "kwant couldn’t check this push: the checks took longer than 12 minutes", file: null, line: null }],
  card: { kind: 'prepush', error: 'the checks took longer than 12 minutes', blockers: [{ source: 'kwant', severity: 'high', title: "kwant couldn’t check this push: the checks took longer than 12 minutes", file: null, line: null }] },
};

/** The reviewer's turn didn't finish: one error row, the rest passed (a check that couldn't run never passes). */
export const PP_REVIEW_ERROR = {
  ...base, id: 'pp_rerr', status: 'failed',
  checks: [...ALL_PASS.slice(0, 4), row('review', 'error', "Didn't finish: the reviewer's turn didn't answer"), ALL_PASS[5]],
  blockers: [{ source: 'review', severity: 'high', title: "The fresh-eyes review didn't finish: the reviewer's turn didn't answer", file: null, line: null }],
  card: inner([], { error: "the reviewer's turn didn't answer", fresh_eyes: { checked: false, saw: [], spec_source: 'pr', reviewer_kwant_id: 'k_iris' } }),
};

export const PP_FAILED_NEEDS_YOU = { ...PP_FAILED, id: 'pp_failed2', tries: 2, needs_you: true };

const REASON = 'The key is a revoked test key from the fixture generator.';
export const PP_OVERRIDDEN = {
  ...PP_NEEDS_YOU, id: 'pp_over', status: 'overridden', needs_you: false,
  override: { by: 'person', reason: REASON, at: PP_T0 + 9 * min },
  overridden_by: 'person', overridden_at: PP_T0 + 9 * min, updated_at: PP_T0 + 9 * min,
};

export const PP_RETRIED = { ...PP_NEEDS_YOU, id: 'pp_retried', status: 'retried', needs_you: false };

export const PP_ALL = { PP_CHECKING, PP_NO_CHANGE, PP_CLEAR, PP_BLOCKED, PP_NEEDS_YOU, PP_MANY, PP_FAILED, PP_REVIEW_ERROR, PP_FAILED_NEEDS_YOU, PP_OVERRIDDEN, PP_RETRIED };

// Fresh-eyes labels (contract §3.2)
export const FE_TICKET = { checked: true, saw: ['spec', 'diff'], spec_source: 'ticket', reviewer_kwant_id: 'k_iris' };
export const FE_TASK = { checked: true, saw: ['brief', 'diff'], spec_source: 'brief', reviewer_kwant_id: 'k_iris' };
export const FE_PR = fe;
export const FE_AUDIT_WRITTEN = { checked: true, saw: ['spec', 'result'], spec_source: 'written', reviewer_kwant_id: 'k_iris' };
export const FE_SAW_CHAT = { checked: false, saw: ['brief', 'diff'], spec_source: 'brief', reviewer_kwant_id: 'k_iris' };
export const FE_FAILED = { checked: false, saw: [], spec_source: 'none', reviewer_kwant_id: 'k_iris' };
