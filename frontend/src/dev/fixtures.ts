// Fixtures for the Dev mode renderers: the route JSON shapes of impl/lin.md with the cards Vega's generator
// wrote for #290, #301 and #303 (fixtures/vega-examples.json, copied unchanged from impl/ at round 3), plus #288 and #289 from the design
// mock (screen 3) and #294 as the "Sample PR" review (screen 4). Tests and the screenshot page use these;
// the app never imports them. Names are kwant's own bots; nothing here is a real key, token or email.

import vega from './fixtures/vega-examples.json';

const row = (id: string, card: Record<string, unknown>, status = 'ready') => ({ id, status, created_at: '2026-11-05T01:00:00Z', ...card });

/** A fixed clock for tests and shots: 2026-11-05 01:00 UTC. */
export const FIXTURE_NOW = Date.parse('2026-11-05T01:00:00Z');

const ada = { kind: 'bot', name: 'Ada', engine: 'claude', model: 'Claude Opus' };
const pixel = { kind: 'bot', name: 'Pixel', engine: 'claude', model: 'Claude Sonnet' };
const rigel = { id: 'kw_demo_rigel', name: 'Rigel', engine: 'codex', model: 'Codex' };
const gh = (n: number) => `https://github.com/alexsergivan/kwant/pull/${n}`;

const [v290, v301, v303] = vega as { pr: number; landed: Record<string, unknown>; review: Record<string, unknown> }[];

/** #288 has an open follow-up (N3): never Verified. */
export const LANDED_288 = row('dvc_288', {
  kind: 'landed', pr: 288, sha: '9be1c40', merged_at: '2026-11-04T18:48:00Z', merged_by: 'Oleks', written_by: pixel,
  area: 'Live activity', headline: 'Live activity hides secrets split across chunks', meta: 'redaction code · 249 lines',
  for_you: "If a bot's thinking spells a password over two chunks, or puts it inside a credential object, the live view now shows dots instead. Ships to the apps in 0.6.11.",
  under_hood: { summary: 'The redactor now keeps a short tail of the last chunk and checks it together with the next one.', files: [
    { path: 'src/server/activity.rs', add: 141, del: 9 }, { path: 'src/server/redact.rs', add: 58, del: 4 },
    { path: 'tests/activity_redact_test.rs', add: 44, del: 0 }, { path: 'docs/live-activity.md', add: 6, del: 2 } ] },
  risk: { level: 'medium', reasons: ['Security path: redaction', '+249 −15 in 4 files'] },
  tests: { state: 'passed', summary: '3 checks passed' }, reviewed_by: 'Rigel',
  follow_ups: [{ text: 'password: X, Bearer/Basic headers and quoted values with spaces still show live. With Pixel.', ref: '#288 N3' }],
  verified: false, links: { pr: gh(288), commit: 'https://github.com/alexsergivan/kwant/commit/9be1c40', ci: null }, cut: null,
});

export const LANDED_289 = row('dvc_289', {
  kind: 'landed', pr: 289, sha: '2f6d0aa', merged_at: '2026-11-04T18:32:00Z', merged_by: 'Oleks', written_by: ada,
  area: 'Spaces rail', headline: "The open space's ring isn't cut off any more",
  for_you: 'The ring around the open space and its unread badge now show in full on the rail. Live on kwant.jumpin.games.',
  under_hood: { summary: 'The rail no longer clips its children; the ring moved inside the tile.', files: [{ path: 'frontend/src/components/Spaces.tsx', add: 5, del: 2 }] },
  risk: { level: 'low', reasons: ['+5 −2 in 1 file'] }, tests: { state: 'passed', summary: '3 checks passed' }, reviewed_by: 'Rigel',
  follow_ups: [], verified: true, links: { pr: gh(289), commit: null, ci: null }, cut: null,
});

/** B1: a commit pushed to the branch without a PR (poller.rs `pr: null`). The server's `verified: true` here is
 * deliberately wrong: with no PR review the card must never get the sticker. */
export const LANDED_DIRECT = row('dvc_c0ffee1', {
  kind: 'landed', pr: null, sha: 'c0ffee1', merged_at: '2026-11-04T19:02:00Z', merged_by: null, written_by: { kind: 'person', name: 'Oleks' },
  area: 'Docs', headline: 'Fix typo in README',
  for_you: 'The install line in the README now names the right command. Nothing in the app changes.',
  under_hood: { summary: 'One word fixed in the README.', files: [{ path: 'README.md', add: 1, del: 1 }] },
  risk: { level: 'low', reasons: ['+1 −1 in 1 file'] }, tests: { state: 'passed', summary: '1 check passed' }, reviewed_by: null,
  follow_ups: [], verified: true, links: { pr: null, commit: 'https://github.com/alexsergivan/kwant/commit/c0ffee1', ci: 'https://github.com/alexsergivan/kwant/actions/runs/1' }, cut: null,
});

export const LANDED_290 = row('dvc_290', v290.landed);
export const LANDED_301 = row('dvc_301', v301.landed);
export const LANDED_303 = row('dvc_303', v303.landed);

/** §3.6 item 12: no writer text. Headline = the PR title; area, for_you, summary are null. */
export const LANDED_NO_TEXT = row('dvc_311', {
  kind: 'landed', pr: 311, sha: 'a1b2c3d', merged_at: '2026-11-03T10:05:00Z', merged_by: 'Oleks', written_by: { kind: 'person', name: 'Oleks' },
  title: 'Bump vite to 6.4.4', headline: null, area: null, for_you: null,
  under_hood: { summary: null, files: [{ path: 'frontend/package-lock.json', add: 12, del: 12 }] },
  risk: { level: 'low', reasons: ['+12 −12 in 1 file'] }, tests: { state: 'passed', summary: '3 checks passed' }, reviewed_by: null,
  follow_ups: [], verified: false, links: { pr: gh(311), commit: null, ci: null }, cut: 'diff cut, 2 files not read',
}, 'failed');
(LANDED_NO_TEXT as Record<string, unknown>).error = 'The writer bot was over its daily cap';

export const REVIEW_290 = row('dvr_290', v290.review);
export const REVIEW_301 = row('dvr_301', v301.review);
export const REVIEW_303 = row('dvr_303', v303.review);
export const REVIEW_289 = row('dvr_289', { kind: 'review', pr: 289, title: 'Spaces rail ring', author: ada, reviewer: rigel, same_engine: false, findings: [] });

/** #294, sample review content (design §5, screen 4). Shown with the "Sample PR" chip. */
export const REVIEW_294 = row('dvr_294', {
  kind: 'review', pr: 294, title: 'Live redaction: catch password: X, Bearer/Basic and quoted values', branch: 'pixel-live-redact-n3', base: 'main', draft: false,
  author: pixel, reviewer: rigel, same_engine: false, note: null,
  risk: { level: 'high', reasons: ['Security path: redaction', 'A test was loosened', 'Changed a file outside the task', '+212 −40 in 5 files'] },
  intent: { source: 'N3', asked: 'hide password: X, Bearer/Basic headers and quoted values with spaces in live activity.', none: false, rows: [
    { state: 'ok', text: 'password: X is hidden' }, { state: 'ok', text: 'Bearer and Basic headers are hidden' },
    { state: 'missing', text: 'Quoted values with spaces still show the part after the space' },
    { state: 'extra', text: 'Also rewords a log line in src/drivers/browser.rs' } ] },
  tamper: { flags: [{ kind: 'loosened', file: 'src/server/activity.rs', line: 418,
    why: 'redacts_bearer_header used to check the exact hidden line. Now it only checks that "Bearer" appears, so a leak would still pass. A new #[ignore] skips redacts_quoted_value.',
    excerpt: '-    assert_eq!(out, "Authorization: Bearer ••••");\n+    assert!(out.contains("Bearer"));' }] },
  security: { secrets: { state: 'pass', count: 0 }, packages: { state: 'pass', count: 0 }, osv: { state: 'later' }, semgrep: { state: 'later' } },
  findings: [
    { id: 'f1', severity: 'high', title: 'Quoted values with spaces still leak in live activity', file: 'src/drivers/activity.rs', line: 212,
      detail: 'The new pattern stops at the first space, so token="abc def" shows def.', fix: 'match up to the closing quote, and un-ignore redacts_quoted_value.',
      source: 'review', state: 'open', sent_at: null, dismissed: null },
    { id: 'f2', severity: 'medium', title: 'A test was weakened instead of the code being fixed', file: 'src/server/activity.rs', line: 418,
      detail: 'redacts_bearer_header now passes even if the header leaks.', fix: 'Put the exact assert back.',
      source: 'tamper', state: 'sent', sent_at: '2026-11-05T00:59:55Z', sent_to: 'Pixel', dismissed: null },
    { id: 'f3', severity: 'low', title: 'Unrelated change in browser.rs', file: 'src/drivers/browser.rs', line: 88,
      detail: 'A log line was reworded. Move it to its own PR or drop it.', fix: null, source: 'review', state: 'open', sent_at: null, dismissed: null },
  ],
  cut: null, links: { pr: gh(294), commit: null, ci: null },
});

/** §3.6 item 10: no second bot in the space. */
export const REVIEW_NO_REVIEWER = row('dvr_312', {
  kind: 'review', pr: 312, title: 'Dev mode: landed and review cards', branch: 'vega-dev-cards', base: 'main', draft: true,
  author: { kind: 'bot', name: 'Vega', engine: 'claude', model: 'Claude Opus' }, reviewer: null, same_engine: false,
  note: 'No reviewer: add a second bot to this space',
  risk: { level: 'medium', reasons: ['Has a migration', '+1840 −12 in 14 files'] }, intent: null, tamper: { flags: [], unread: ['Cargo.lock', 'frontend/package-lock.json'] },
  security: { secrets: { state: 'not_run', count: 0 }, packages: { state: 'not_run', count: 0 } }, findings: [],
  cut: 'diff cut, 3 files not read', links: { pr: gh(312), commit: null, ci: null },
});

export const LANDED_ROUTE = {
  cards: [LANDED_303, LANDED_301, LANDED_290, LANDED_288, LANDED_289],
  rollup: { since: '2026-11-04T17:40:00Z', until: '2026-11-05T01:00:00Z', count: 5, needs_you: 2, main_green: true, areas: [
    { area: 'Chat', count: 1, first_headline: 'A 👍 or 👎 reaches the bot once' },
    { area: 'Sidebar', count: 2, first_headline: 'Long bot names wrap between words, not inside them' },
    { area: 'Live activity', count: 1, first_headline: 'Live activity hides secrets split across chunks' },
    { area: 'Spaces rail', count: 1, first_headline: "The open space's ring isn't cut off any more" } ] },
  last_tag: 'mac-v0.6.10',
};

export const ROLLUP_QUIET = {
  since: '2026-11-04T17:40:00Z', until: '2026-11-04T19:51:00Z', count: 3, needs_you: 0, main_green: true, areas: [
    { area: 'Sidebar', count: 1, first_headline: 'Bot rows stop jumping on hover' },
    { area: 'Live activity', count: 1, first_headline: 'Secrets split across chunks are hidden' },
    { area: 'Spaces rail', count: 1, first_headline: "The open space's ring isn't cut off" } ] };

export const ROLLUP_EMPTY = { since: '2026-11-05T00:30:00Z', until: '2026-11-05T01:00:00Z', count: 0, needs_you: 0, main_green: true, areas: [] };

export const ROLLUP_MANY = { ...ROLLUP_QUIET, count: 7, main_green: false, areas: [
  ...ROLLUP_QUIET.areas, { area: 'Chat', count: 2, first_headline: 'A reaction reaches the bot once' },
  { area: 'Settings', count: 1, first_headline: 'Backup command field' }, { area: 'Docs', count: 1, first_headline: 'Dev mode guide' } ] };

export const RELEASE_PENDING = row('dvc_rn1', { kind: 'release_notes' }, 'pending');
export const RELEASE_READY = row('dvc_rn1', {
  kind: 'release_notes', since_tag: 'mac-v0.6.12',
  whats_new: 'Sidebar rows hold still, long names wrap between words, and reactions reach a bot once.',
  markdown: '## What changed since mac-v0.6.12\n\n### Sidebar\n- Rows stop jumping on hover (#290)\n- Long bot names wrap between words, not inside them (#301)\n\n### Chat\n- A reaction reaches the bot once, even in the same second as its reply (#303)\n',
});

// ---- Production floor (approval cards) ----

const floorApproval = (id: string, bot: string, command: string, prod_floor: Record<string, unknown>, cwd = '~/work/kwant-deploy') => ({
  id, tool_name: 'execute_shell',
  details: { source: 'local_shell', kwant_name: bot, arguments: { command, cwd }, prod_floor: { space_id: 's_kwant', ...prod_floor } },
});

export const FLOOR_DEPLOY_FRESH = floorApproval('ap_1', 'Pixel', './scripts/deploy_test.sh 4049325', {
  kind: 'deploy', why: 'a deploy to the test server', needs_backup: false,
  blast: [['Where', 'kwant.jumpin.games · production'], ['What changes', 'App 51f9141 → 4049325 · 1 change: #290 sidebar hover'],
    ['Downtime', 'About 20 s while the container restarts'], ['Data', 'Nothing deleted. Database and uploads stay.'],
    ['If it fails', 'Health check fails → it goes back to 51f9141 on its own']],
  backup: { state: 'fresh', age_s: 14, bytes: 412_000_000, path: '/home/demo/backups/backup-2026-11-04-pre-4049325.tgz', error: null },
});

export const FLOOR_DESTRUCTIVE = floorApproval('ap_2', 'Lin', 'sqlite3 data/kwant.db "DROP TABLE dev_cards_v0;"', {
  kind: 'destructive', why: 'a database command that runs DROP TABLE', needs_backup: true,
  blast: [['Where', 'kwant.jumpin.games database · production'], ['Data', "Table dev_cards_v0 · 1,284 rows. Can't be undone without a backup."]],
  backup: { state: 'old', age_s: 3 * 3600, bytes: 412_000_000, path: '/home/demo/backups/backup-old.tgz', error: null },
}, '~/work/kwant');

export const FLOOR_MIGRATION = floorApproval('ap_3', 'Vega', './scripts/deploy_test.sh 7c41e0a', {
  kind: 'deploy_migration', why: 'a deploy that changes the database', needs_backup: true,
  blast: [['Where', 'kwant.jumpin.games · production'], ['What changes', 'App 4049325 → 7c41e0a · 1 change: #301 risk reasons'],
    ['Migration', '0031_landed_risk.sql · adds 1 column to landed_cards · has a way back'],
    ['If it fails', 'The app goes back on its own. The database goes back only from a backup.']],
  backup: { state: 'old', age_s: 41 * 60, bytes: 412_000_000, path: null, error: null },
});

export const FLOOR_DEPLOY_OLD = floorApproval('ap_4', 'Iris', './scripts/deploy_test.sh 9a2d5f3', {
  kind: 'deploy', why: 'a deploy to the test server', needs_backup: false,
  blast: [['Where', 'kwant.jumpin.games · production'], ['What changes', 'App 7c41e0a → 9a2d5f3 · 1 change: #292 reactions'],
    ['Data', 'No migration. Database and uploads stay.']],
  backup: { state: 'old', age_s: 3 * 3600, bytes: 412_000_000, path: null, error: null },
});

export const FLOOR_PAGES = floorApproval('ap_5', 'Pixel', 'npx wrangler pages deploy site/dist', {
  kind: 'deploy', why: 'a Cloudflare Pages deploy', needs_backup: false,
  blast: [['Where', 'kwant.si · production'], ['Key used', 'Cloudflare · production']],
  backup: { state: 'no_data', age_s: null, bytes: null, path: null, error: null },
});

export const FLOOR_NO_COMMAND = floorApproval('ap_6', 'Lin', 'psql -c "DELETE FROM sessions"', {
  kind: 'destructive', why: 'a database command that runs DELETE without WHERE', needs_backup: true,
  blast: [['Where', 'db.example.com · production']],
  backup: { state: 'no_command', age_s: null, bytes: null, path: null, error: null },
});

export const FLOOR_FAILED = floorApproval('ap_7', 'Lin', 'sqlite3 data/kwant.db "DROP TABLE old_x;"', {
  kind: 'destructive', why: 'a database command that runs DROP TABLE', needs_backup: true,
  blast: [['Where', 'kwant.jumpin.games database · production']],
  backup: { state: 'failed', age_s: null, bytes: null, path: null, error: 'disk full' },
});
