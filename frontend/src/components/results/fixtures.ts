// One sample bot reply and its `results` for every block type in plan §A.3, for building renderers and
// for tests. Names, prices and numbers are made up. Every value is copied from the reply text, the way
// the server copies it (or, for verification and files, from typed kwant data).

import type { ResultSet, SmartBlock } from '../../smartResults';
import { recipe } from './recipeFixtures';

export interface Fixture {
  /** The reply's markdown, as streamed and stored. */
  reply: string;
  results: ResultSet;
}

const table = (head: string[], rows: string[][]) =>
  [`| ${head.join(' | ')} |`, `|${head.map(() => '---').join('|')}|`, ...rows.map((r) => `| ${r.join(' | ')} |`)].join('\n');
const set = (block: SmartBlock): ResultSet => ({ version: 1, blocks: [block] });
const fromReply = { kind: 'reply_text' } as const;

// comparison: a pick, plus pros and cons columns.
const cmpHead = ['Host', 'Price per month', 'Build minutes', 'Pros', 'Cons'];
const cmpRows = [
  ['Pagecraft', 'EUR 19', '500', 'Cheapest; simple setup', 'No preview builds'],
  ['**Northdeck**', 'EUR 49', '3,000', 'Preview builds; EU region; chat support', 'Two seats only'],
  ['Fernlane', 'EUR 129', '10,000', 'SSO; audit log', 'Pricey under 10 people'],
];
const comparison: Fixture = {
  reply: `Three hosts fit the docs site.\n\n${table(cmpHead, cmpRows)}\n\n**Northdeck** is the one I'd pick for five of us.`,
  results: set({
    id: 'b-cmp',
    type: 'comparison',
    title: 'Docs hosting',
    source: { kind: 'markdown_table', index: 0 },
    provenance: fromReply,
    columns: [
      { key: 'host', label: 'Host', numeric: false, unit: null, role: 'label' },
      { key: 'price', label: 'Price per month', numeric: true, unit: 'EUR', role: 'value' },
      { key: 'minutes', label: 'Build minutes', numeric: true, unit: null, role: 'value' },
      { key: 'pros', label: 'Pros', numeric: false, unit: null, role: 'pros' },
      { key: 'cons', label: 'Cons', numeric: false, unit: null, role: 'cons' },
    ],
    rows: cmpRows.map((cells, i) => ({ id: `r${i}`, cells })),
    highlight_row: 'r1',
    highlight_reason: 'named as the pick in this reply',
  }),
};

// chart (line): a time-like first column, numeric series.
const lineRows = [
  ['Jul 2026', '1,240', '310'],
  ['Aug 2026', '1,515', '362'],
  ['Sep 2026', '1,890', '401'],
  ['Oct 2026', '2,260', '455'],
];
const chartLine: Fixture = {
  reply: `Signups kept climbing through October.\n\n${table(['Month', 'Signups', 'Paid'], lineRows)}\n\nOctober had 2,260 signups and 455 paid.`,
  results: set({
    id: 'b-line',
    type: 'chart',
    title: 'Signups by month',
    source: { kind: 'markdown_table', index: 0 },
    provenance: fromReply,
    chart_kind: 'line',
    x: { label: 'Month', kind: 'time' },
    series: [
      { key: 'signups', label: 'Signups', unit: null },
      { key: 'paid', label: 'Paid', unit: null },
    ],
    points: lineRows.map(([x, a, b], i) => ({ id: `p${i}`, x, cells: [a, b], values: [Number(a.replace(/,/g, '')), Number(b)] })),
    summary: 'October had 2,260 signups and 455 paid.',
  }),
};

// chart (bar): categories, one series, no pick.
const barRows = [
  ['Search', '4,120'],
  ['Newsletter', '1,980'],
  ['Social', '1,310'],
  ['Referral', '640'],
  ['Direct', '520'],
];
const chartBar: Fixture = {
  reply: `Where October's visits came from:\n\n${table(['Channel', 'Visits'], barRows)}`,
  results: set({
    id: 'b-bar',
    type: 'chart',
    title: null,
    source: { kind: 'markdown_table', index: 0 },
    provenance: fromReply,
    chart_kind: 'bar',
    x: { label: 'Channel', kind: 'category' },
    series: [{ key: 'visits', label: 'Visits', unit: null }],
    points: barRows.map(([x, v], i) => ({ id: `p${i}`, x, cells: [v], values: [Number(v.replace(/,/g, ''))] })),
    summary: null,
  }),
};

// metrics: "**Label:** value" lines, the rest is the note.
const metrics: Fixture = {
  reply: `October in numbers:\n\n- **Revenue:** EUR 18,400, up 8% vs September\n- **Active teams:** 312\n- **Churn:** 2.1%, down from 2.6%`,
  results: set({
    id: 'b-metrics',
    type: 'metrics',
    title: null,
    source: { kind: 'markdown_list', index: 0, ordered: false },
    provenance: fromReply,
    items: [
      { id: 'm0', label: 'Revenue', value_text: 'EUR 18,400', value: 18400, unit: 'EUR', note: 'up 8% vs September' },
      { id: 'm1', label: 'Active teams', value_text: '312', value: 312, unit: null, note: null },
      { id: 'm2', label: 'Churn', value_text: '2.1%', value: 2.1, unit: '%', note: 'down from 2.6%' },
    ],
  }),
};

// facts: mostly non-numeric "**Label:** value" lines.
const facts: Fixture = {
  reply: `Here's the domain as it stands:\n\n- **Registrar:** Porkfish\n- **Renews:** 14 March 2027\n- **DNS host:** Northdeck\n- **Owner contact:** ops@example.com`,
  results: set({
    id: 'b-facts',
    type: 'facts',
    title: 'docs.trailhead.shop',
    source: { kind: 'markdown_list', index: 0, ordered: false },
    provenance: fromReply,
    items: [
      { id: 'f0', label: 'Registrar', value: 'Porkfish' },
      { id: 'f1', label: 'Renews', value: '14 March 2027' },
      { id: 'f2', label: 'DNS host', value: 'Northdeck' },
      { id: 'f3', label: 'Owner contact', value: 'ops@example.com' },
    ],
  }),
};

// steps: a numbered list.
const stepTexts = [
  'Lower the TTL on the current `docs` record to 300 seconds, a day ahead.',
  'In the new host, add `docs.trailhead.shop` as a custom domain.',
  'Copy the CNAME target it shows you.',
  'At the registrar, point the `docs` CNAME at that target.',
  'Wait for the certificate to show **Active**, then open the site to check.',
];
const steps: Fixture = {
  reply: `Moving DNS takes about 20 minutes.\n\n${stepTexts.map((t, i) => `${i + 1}. ${t}`).join('\n')}\n\nTell me when step 4 is done.`,
  results: set({
    id: 'b-steps',
    type: 'steps',
    title: null,
    source: { kind: 'markdown_list', index: 0, ordered: true },
    provenance: fromReply,
    variant: 'steps',
    items: stepTexts.map((text, i) => ({ id: `s${i}`, text, done: null, when: null, owner: null })),
  }),
};

// checklist: a task list; `done` comes from the reply's [x].
const checks: [boolean, string][] = [
  [true, 'Export the old site as static files'],
  [true, 'Create the Northdeck project'],
  [false, 'Move the `docs` CNAME (owner: Sam)'],
  [false, 'Turn on preview builds'],
];
const checklist: Fixture = {
  reply: `Where the move stands:\n\n${checks.map(([d, t]) => `- [${d ? 'x' : ' '}] ${t}`).join('\n')}`,
  results: set({
    id: 'b-check',
    type: 'steps',
    title: 'Docs move',
    source: { kind: 'markdown_list', index: 0, ordered: false },
    provenance: fromReply,
    variant: 'checklist',
    items: checks.map(([done, text], i) => ({ id: `c${i}`, text, done, when: null, owner: text.includes('owner: Sam') ? 'Sam' : null })),
  }),
};

// timeline: each item starts with a time-like label and a separator; the label is `when`.
const events: [string, string][] = [
  ['Mon 9 Nov', 'Freeze content on the old site'],
  ['Tue 10 Nov', 'Switch DNS (Owner: Sam)'],
  ['Wed 11 Nov', 'Check search console for errors'],
  ['Fri 13 Nov', 'Cancel the old plan'],
];
const timeline: Fixture = {
  reply: `Proposed schedule:\n\n${events.map(([w, t]) => `- ${w}: ${t}`).join('\n')}`,
  results: set({
    id: 'b-time',
    type: 'steps',
    title: null,
    source: { kind: 'markdown_list', index: 0, ordered: false },
    provenance: fromReply,
    variant: 'timeline',
    items: events.map(([when, text], i) => ({ id: `t${i}`, text, done: null, when, owner: text.includes('Owner: Sam') ? 'Sam' : null })),
  }),
};

// cards: links from the reply, each with a description.
const links: [string, string, string][] = [
  ['Northdeck docs hosting', 'https://northdeck.example/docs', 'Preview builds on every pull request and an EU region.'],
  ['Pagecraft pricing', 'https://pagecraft.example/pricing', 'The cheapest plan, with 500 build minutes a month.'],
  ['Fernlane security', 'https://fernlane.example/security', 'SSO and audit log, priced for teams past ten people.'],
];
const cards: Fixture = {
  reply: `Worth a read before we decide:\n\n${links.map(([t, u, d]) => `- [${t}](${u}) - ${d}`).join('\n')}`,
  results: set({
    id: 'b-cards',
    type: 'cards',
    title: null,
    source: { kind: 'markdown_list', index: 0, ordered: false },
    provenance: fromReply,
    items: links.map(([title, url, description], i) => ({ id: `k${i}`, title, url, description, meta: [] })),
  }),
};

// findings: severity tokens, refs from code spans.
const findings: Fixture = {
  reply: [
    'Review of the deploy script:',
    '',
    '- **High:** the token is written to the log in `scripts/deploy.sh:42`',
    '- **Medium:** no timeout on the health check in `scripts/deploy.sh:88`',
    '- **Nit:** unused variable `RETRIES`',
  ].join('\n'),
  results: set({
    id: 'b-find',
    type: 'findings',
    title: null,
    source: { kind: 'markdown_list', index: 0, ordered: false },
    provenance: fromReply,
    items: [
      { id: 'x0', severity: 'high', severity_text: 'High', title: 'the token is written to the log', detail: 'the token is written to the log in `scripts/deploy.sh:42`', refs: [{ path: 'scripts/deploy.sh', line: 42 }] },
      { id: 'x1', severity: 'medium', severity_text: 'Medium', title: 'no timeout on the health check', detail: 'no timeout on the health check in `scripts/deploy.sh:88`', refs: [{ path: 'scripts/deploy.sh', line: 88 }] },
      { id: 'x2', severity: 'info', severity_text: 'Nit', title: 'unused variable', detail: 'unused variable `RETRIES`', refs: [] },
    ],
  }),
};

// verification: from the task's stored check results; drawn after the reply.
const verification: Fixture = {
  reply: 'The migration task is done. I ran the outcome checks; one is still failing.',
  results: set({
    id: 'b-verify',
    type: 'verification',
    title: null,
    source: { kind: 'slot' },
    provenance: { kind: 'kwant_data', ref: 'task:7f3a9c' },
    target: { kind: 'task', id: '7f3a9c', title: 'Move docs to Northdeck' },
    checks: [
      { id: 'v0', label: 'docs.trailhead.shop answers 200', kind: 'http', status: 'passed', excerpt: 'HTTP 200 in 182 ms', checked_at: '2026-11-02T21:10:00Z' },
      { id: 'v1', label: 'Certificate is valid', kind: 'command', status: 'passed', excerpt: 'notAfter=Feb  1 2027', checked_at: '2026-11-02T21:10:02Z' },
      { id: 'v2', label: 'Old host redirects', kind: 'http', status: 'failed', excerpt: 'HTTP 404, expected 301', checked_at: '2026-11-02T21:10:03Z' },
      { id: 'v3', label: 'Sitemap lists 120 pages', kind: 'command', status: 'pending', excerpt: null, checked_at: null },
    ],
    summary: '2 of 4 checks passed',
  }),
};

// files: the message's own attachments; drawn after the reply.
const files: Fixture = {
  reply: "Here's the export and the redirect map.",
  results: set({
    id: 'b-files',
    type: 'files',
    title: null,
    source: { kind: 'slot' },
    provenance: { kind: 'kwant_data', ref: 'attachment:att-redirects' },
    items: [
      {
        id: 'a0',
        attachment_id: 'att-redirects',
        name: 'redirects.csv',
        mime: 'text/csv',
        size_bytes: 2310,
        preview: { kind: 'table', columns: ['from', 'to'], rows: [['/guide', '/docs/guide'], ['/faq', '/docs/faq']] },
      },
      { id: 'a1', attachment_id: 'att-notes', name: 'move-notes.md', mime: 'text/markdown', size_bytes: 840, preview: { kind: 'text', excerpt: '# Move notes\nFreeze content on Monday.' } },
      {
        id: 'a3',
        attachment_id: 'att-big',
        name: 'urls-all.csv',
        mime: 'text/csv',
        size_bytes: 91200,
        preview: { kind: 'table', columns: ['url', 'status'], rows: [['/a', '200'], ['/b', '301']], total_rows: 143, total_cols: 2 },
      },
      { id: 'a4', attachment_id: 'att-log', name: 'crawl.log', mime: 'text/plain', size_bytes: 40960, preview: { kind: 'text', excerpt: 'start\nfetch /a\nfetch /b', total_lines: 512 } },
      { id: 'a5', attachment_id: 'att-dump', name: 'dump.txt', mime: 'text/plain', size_bytes: 3145728, preview: { kind: 'text', excerpt: 'header', truncated: true } },
      { id: 'a2', attachment_id: 'att-shot', name: 'dns.png', mime: 'image/png', size_bytes: 48200, preview: { kind: 'image' } },
    ],
  }),
};


// pocket (pa4): the Pocket card in its 7 states (pa0a/design.md §1) plus paused and the results card (§6).
// A slot block after Quizmaster's reply. Names, scores, addresses and costs are made up, after Mira's mockups.
const POCKET_WIFI = 'http://192.168.1.20:7480/p/fq7k2m';
const POCKET_ANYWHERE = 'https://quiet-otter-lamp.trycloudflare.com/p/fq7k2m';
const ROUNDS = [
  { label: 'Round 1', topic: 'Music', written: 10, total: 10 },
  { label: 'Round 2', topic: 'Sport', written: 10, total: 10 },
  { label: 'Round 3', topic: 'Film', written: 4, total: 10 },
  { label: 'Round 4', topic: 'Picture round', written: 0, total: 10 },
];
const pocketBase = {
  id: 'b-pocket',
  type: 'pocket',
  source: { kind: 'slot' },
  provenance: { kind: 'kwant_data', ref: 'pocket:fq7k2m' },
  code: 'fq7k2m',
  kit: 'quiz',
  name: 'Friday Pub Quiz',
  when: 'Fri, Nov 27 · 8:00 PM · 40 questions · teams of up to 4',
  state: 'writing',
  paused: false,
  progress: null,
  rounds: [],
  checks: null,
  dry_run: null,
  eta: null,
  opens_line: null,
  live: null,
  join: null,
  ended: null,
  budget: null,
  results: null,
  santa: null,
} as const;
const passedRun = { players: 30, passed: true, stopped_at: null, what_happened: null, what_im_doing: null };
const pocketFixture = (reply: string, block: Record<string, unknown>): Fixture => ({
  reply,
  results: set({ ...pocketBase, ...block } as unknown as SmartBlock),
});
export const POCKET_FIXTURES = {
  writing: pocketFixture("I'm writing your pub quiz now. Here's how it's going.", {
    state: 'writing',
    progress: { done: 24, total: 40 },
    rounds: ROUNDS,
    dry_run: { ...passedRun, passed: false },
    eta: 'About 10 minutes.',
    budget: { calls: 9, usd: 0.02 },
  }),
  checking: pocketFixture('All 40 questions are written. Checking every answer against a source now.', {
    state: 'checking',
    progress: { done: 31, total: 40 },
    checks: { matched: 29, total: 40, swapped: ['Q7', 'Q22'] },
    eta: 'About 4 minutes.',
    budget: { calls: 13, usd: 0.03 },
  }),
  dryRunStopped: pocketFixture("The dry run stopped, so there's no QR code yet.", {
    state: 'dry_run_stopped',
    checks: { matched: 40, total: 40, swapped: [] },
    dry_run: {
      players: 30,
      passed: false,
      stopped_at: 'Round 4, question 3',
      what_happened: "the picture for Q33 didn't load on 12 of 30 pretend phones, so their timers ran out.",
      what_im_doing: "swapping in a smaller copy of the picture and running the dry run again. I'll say here when it passes.",
    },
    budget: { calls: 16, usd: 0.04 },
  }),
  ready: pocketFixture('Your quiz is ready. Every answer has a source and the dry run passed.', {
    state: 'ready',
    checks: { matched: 40, total: 40, swapped: [] },
    dry_run: passedRun,
    opens_line: 'Joining opens Fri at 7:30 PM, 30 minutes before the start',
    join: { mode: 'wifi', wifi_url: POCKET_WIFI, anywhere_url: POCKET_ANYWHERE },
    budget: { calls: 15, usd: 0.04 },
  }),
  readyAnywhere: pocketFixture('Your quiz is ready. Every answer has a source and the dry run passed.', {
    state: 'ready',
    checks: { matched: 40, total: 40, swapped: [] },
    dry_run: passedRun,
    opens_line: 'Joining opens Fri at 7:30 PM, 30 minutes before the start',
    join: { mode: 'anywhere', wifi_url: POCKET_WIFI, anywhere_url: POCKET_ANYWHERE },
    budget: { calls: 15, usd: 0.04 },
  }),
  live: pocketFixture('The quiz is on.', {
    state: 'live',
    dry_run: passedRun,
    live: {
      players: 23,
      teams: 6,
      leader: { name: 'Quizzly Bears', points: 1840 },
      now: 'Round 2 of 4 · question 6 of 10',
      palettes: ['cyan', 'coral', 'mint', 'yellow', 'blue', 'violet'],
    },
    join: { mode: 'wifi', wifi_url: POCKET_WIFI, anywhere_url: POCKET_ANYWHERE },
    budget: { calls: 17, usd: 0.05 },
  }),
  paused: pocketFixture('Paused for the break.', {
    state: 'live',
    paused: true,
    dry_run: passedRun,
    live: {
      players: 23,
      teams: 6,
      leader: { name: 'Quizzly Bears', points: 1840 },
      now: 'Round 2 of 4 · paused after question 6',
      palettes: ['cyan', 'coral', 'mint', 'yellow', 'blue', 'violet'],
    },
    join: { mode: 'wifi', wifi_url: POCKET_WIFI, anywhere_url: POCKET_ANYWHERE },
    budget: { calls: 17, usd: 0.05 },
  }),
  ended: pocketFixture('That was the Friday Pub Quiz. Well played, everyone.', {
    state: 'ended',
    dry_run: passedRun,
    ended: { at: '10:12 PM', winner: { name: 'Quizzly Bears', points: 3420 }, stats: '23 players in 6 teams · 40 questions', deletes_on: 'Fri, Dec 4' },
    budget: { calls: 19, usd: 0.05 },
  }),
  results: pocketFixture("Here are the results of the Friday Pub Quiz.", {
    state: 'ended',
    dry_run: passedRun,
    ended: { at: '10:12 PM', winner: { name: 'Quizzly Bears', points: 3420 }, stats: '23 players in 6 teams · 40 questions', deletes_on: 'Fri, Dec 4' },
    budget: { calls: 19, usd: 0.05 },
    results: {
      teams: [
        { name: 'Quizzly Bears', players: 4, points: 3420, palette: 'cyan', members: ['Ana', 'Ben', 'Chloe', 'Dev'] },
        { name: 'Les Quizerables', players: 4, points: 3180, palette: 'coral', members: [] },
        { name: 'Agatha Quiztie', players: 4, points: 2960, palette: 'mint', members: [] },
        { name: 'The Know-It-Ales', players: 4, points: 2710, palette: 'yellow', members: [] },
        { name: 'Trivia Newton-John', players: 4, points: 2390, palette: 'blue', members: [] },
        { name: 'Table 6', players: 3, points: 1980, palette: 'violet', members: [] },
      ],
      margin: 240,
      missed: [
        { label: 'Q33', question: 'Name this bridge', answer: 'Pont du Gard', pct_right: 9 },
        { label: 'Q18', question: 'Year the first iPod came out', answer: '2001', pct_right: 17 },
        { label: 'Q27', question: 'Smallest planet', answer: 'Mercury', pct_right: 26 },
      ],
      near_misses: 5,
      near_misses_accepted: 3,
    },
  }),
} satisfies Record<string, Fixture>;

// Secret Santa (pa10a/design.md §1): a family of 9, host Oleks. Names and creatures are made up.
const SANTA_FUNNEL = 'https://oleks-mac.tail4b2e.ts.net/p/sn4tq9';
const SANTA_WIFI = 'http://192.168.1.20:7480/p/sn4tq9';
const SANTA_PEOPLE = [
  ['p1', 'Oleks', 'beret'], ['p2', 'Mia', 'antenna'], ['p3', 'Sam', 'visor'], ['p4', 'Priya', 'bulb'], ['p5', 'Jonas', 'spikes'],
  ['p6', 'Ana', 'goggles'], ['p7', 'Kai', 'horns'], ['p8', 'Leo', 'ears'], ['p9', 'Zoe', 'antenna'],
] as const;
const santaPeople = (f: (id: string, i: number) => Record<string, unknown>, upTo = 9) =>
  SANTA_PEOPLE.slice(0, upTo).map(([id, name, avatar], i) => ({ id, name, avatar, you: id === 'p1', locked: false, looked: null, gift_ready: null, ...f(id, i) }));
const santaBase = {
  joined: 9,
  expected: 9,
  waiting_for: [],
  keep_apart: [['Mia', 'Sam'], ['Priya', 'Jonas']],
  draw_rule: 'when all 9 are in, or Sun, Dec 6 at 6 PM, whichever comes first',
  drawn_at: null,
  reveal: 'scheduled',
  reveal_at: '7:00 PM',
  revealed_at: null,
  deletes_on: 'Fri, Dec 25',
  ended_on: null,
  looked: null,
  gifts_ready: null,
  asks_asked: null,
  asks_answered: null,
  can_draw: false,
  blocked: null,
  join: { mode: 'anywhere', anywhere_kind: 'funnel', anywhere_url: SANTA_FUNNEL, wifi_url: SANTA_WIFI },
};
const santaFixture = (reply: string, state: 'live' | 'ended', santa: Record<string, unknown>): Fixture =>
  pocketFixture(reply, {
    provenance: { kind: 'kwant_data', ref: 'pocket:sn4tq9' },
    code: 'sn4tq9',
    kit: 'santa',
    name: 'Family Secret Santa',
    when: '€25 each · gifts on Fri, Dec 18 · reveal at 7 PM',
    state,
    budget: { calls: 3, usd: 0.01 },
    santa: { ...santaBase, ...santa },
  });
const collecting = {
  phase: 'collecting',
  joined: 7,
  waiting_for: ['Leo', 'Zoe'],
  people: santaPeople(() => ({}), 7),
  can_draw: true,
};
const drawnPeople = santaPeople((id, i) => ({ looked: id !== 'p8', gift_ready: i < 5 }));
export const SANTA_FIXTURES = {
  collecting: santaFixture("Your Secret Santa is open. Send the invite to the family chat; I'll draw names when everyone's in.", 'live', collecting),
  noAddress: santaFixture('Your Secret Santa is open on your Wi-Fi. To let the family join from anywhere, set up Tailscale Funnel (the How to has the steps).', 'live', {
    ...collecting,
    join: { mode: 'wifi', anywhere_kind: null, anywhere_url: null, wifi_url: SANTA_WIFI },
  }),
  drawn: santaFixture("Names are drawn. Everyone sees only their own match; here's how it's going.", 'live', {
    phase: 'drawn',
    people: drawnPeople,
    drawn_at: 'Sun, Dec 6 at 6:00 PM',
    looked: 8,
    gifts_ready: 5,
    asks_asked: 6,
    asks_answered: 5,
  }),
  revealDay: santaFixture("It's gift day. Every Santa ticked their gift.", 'live', {
    phase: 'reveal_day',
    people: santaPeople(() => ({ looked: true, gift_ready: true })),
    drawn_at: 'Sun, Dec 6 at 6:00 PM',
    looked: 9,
    gifts_ready: 9,
    asks_asked: 11,
    asks_answered: 11,
  }),
  revealDayNotAll: santaFixture("It's gift day. Two Santas haven't ticked their gift yet.", 'live', {
    phase: 'reveal_day',
    people: santaPeople((_id, i) => ({ looked: true, gift_ready: i < 7 })),
    drawn_at: 'Sun, Dec 6 at 6:00 PM',
    looked: 9,
    gifts_ready: 7,
    asks_asked: 11,
    asks_answered: 10,
  }),
  ended: santaFixture('Your Secret Santa is over. Everything is deleted on Fri, Dec 25.', 'ended', {
    phase: 'ended',
    people: santaPeople(() => ({ looked: true, gift_ready: true })),
    drawn_at: 'Sun, Dec 6 at 6:00 PM',
    reveal: 'shown',
    revealed_at: 'Fri, Dec 18 at 7:00 PM',
    ended_on: 'Dec 18',
    looked: 9,
    gifts_ready: 9,
    asks_asked: 11,
    asks_answered: 11,
    join: null,
  }),
  // People and PINs after the draw: Leo hasn't looked; Ana is locked out after 5 wrong PINs.
  people: santaFixture("Here's who's in. Matches never show here.", 'live', {
    phase: 'drawn',
    people: santaPeople((id, i) => ({ looked: id !== 'p8', gift_ready: i < 5, locked: id === 'p6' })),
    drawn_at: 'Sun, Dec 6 at 6:00 PM',
    looked: 8,
    gifts_ready: 5,
    asks_asked: 6,
    asks_answered: 5,
  }),
} satisfies Record<string, Fixture>;
/** What host/reset-pin answers for the shots and tests: a one-time code and its expiry (24 h). */
export const SANTA_RESET_CODE = { code: 'K7QX', expires_at: '2026-12-08T21:41:00+01:00' };

// Pocket apps (pa10b/design.md): Riverside Runners, host Oleks, 22 members, euros. Names, places and creatures are
// made up. One engine, three faces: RSVP, Sign-up and Vote, states 1-9 plus People, Nudges and the closed results.
// `gather` is Lin's host view as the box sends it (pa10b/server-shape.md, src/pocket/gather.rs) plus the pocket's
// title and expires_at: RFC 3339 times (+01:00, Berlin in December), food keys, `{link}` in the drafts.
const GATHER_FUNNEL = 'https://oleks-mac.tail4b2e.ts.net/p/runclub';
const GATHER_WIFI = 'http://192.168.1.20:7480/p/runclub';
const CREATURES = ['beret', 'antenna', 'visor', 'bulb', 'spikes', 'goggles', 'horns', 'ears'];
const av = (i: number) => CREATURES[i % CREATURES.length];
const at = (day: number, hm: string) => `2026-12-${String(day).padStart(2, '0')}T${hm}:00+01:00`;
type P = [name: string, answer: 'yes' | 'maybe' | 'no' | null, plus: number, food: string[], paid: boolean];
const person = ([name, answer, plus_ones, food, paid]: P, i: number) => ({
  id: `g${i + 1}`, name, avatar: av(i), answer, plus_ones, food, food_other: null, answered_at: at(1, '18:00'), paid_at: answer === 'yes' && paid ? at(3, '12:00') : null,
});
/** RSVP counts as gather.rs builds them from the rows. */
function rsvpParts(rows: P[]) {
  const yes = rows.filter((r) => r[1] === 'yes');
  const plus = yes.reduce((n, r) => n + r[2], 0);
  const food: Record<string, number> = {};
  for (const r of yes) for (const k of r[3]) food[k] = (food[k] ?? 0) + 1;
  return {
    counts: { yes: yes.length, maybe: rows.filter((r) => r[1] === 'maybe').length, no: rows.filter((r) => r[1] === 'no').length, plus_ones: plus, coming: yes.length + plus },
    paid: { ticked: yes.filter((r) => r[4]).length, of: yes.length, not_yet: yes.filter((r) => !r[4]).map((r) => r[0]) },
    food: { counts: food, other: [] },
  };
}
// State 1-3: Yes 14 (+3) · Maybe 4 · No 2 · Paid 10 of 14 (not yet: Ana, Kai, Ravi, Tom) · 2 of 22 haven't answered.
const RSVP_OPEN_PEOPLE: P[] = [
  ['Oleks', 'yes', 1, ['vegetarian'], true], ['Mia', 'yes', 1, [], true], ['Priya', 'yes', 0, ['vegan'], true], ['Ana', 'yes', 0, ['nut_allergy'], false],
  ['Kai', 'yes', 0, [], false], ['Ravi', 'yes', 0, ['vegetarian'], false], ['Tom', 'yes', 0, [], false], ['Leo', 'maybe', 0, [], false], ['Finn', 'no', 0, [], false],
  ['Jonas', 'yes', 0, [], true], ['Elif', 'yes', 1, ['vegetarian'], true], ['Marta', 'yes', 0, ['gluten_free'], true], ['Ben', 'yes', 0, [], true],
  ['Lucia', 'yes', 0, [], true], ['Omar', 'yes', 0, [], true], ['Zoe', 'maybe', 0, [], false], ['Nina', 'maybe', 0, [], false], ['Sam', 'maybe', 0, [], false], ['Ida', 'no', 0, [], false], ['Hana', 'yes', 0, [], true],
];
const idOf = (rows: P[], name: string) => `g${rows.findIndex((r) => r[0] === name) + 1}`;
const maybeNote = (n: string) => `Hi ${n}! Still a maybe for the club dinner on Sat at The Old Mill? Answers close Thu at 8 PM so Oleks can book the table. Tap to change it: {link}`;
const unpaidNote = (n: string) => `Hi ${n}, a quick one: the €35 for Saturday's dinner is still open. Pay Oleks whenever suits you before the dinner. Thanks!`;
const draft = (rows: P[], to: string, kind: string, text: string, copied_at: string | null = null, next_copy_at: string | null = null) => ({
  to, player_id: idOf(rows, to), kind, text, copied_at, next_copy_at, can_copy: next_copy_at === null,
});
const RSVP_DRAFTS = [
  draft(RSVP_OPEN_PEOPLE, 'Leo', 'maybe', maybeNote('Leo')),
  draft(RSVP_OPEN_PEOPLE, 'Zoe', 'maybe', maybeNote('Zoe'), at(9, '21:12'), at(11, '21:12')),
  draft(RSVP_OPEN_PEOPLE, 'Nina', 'maybe', maybeNote('Nina')),
  draft(RSVP_OPEN_PEOPLE, 'Sam', 'maybe', maybeNote('Sam')),
  draft(RSVP_OPEN_PEOPLE, 'Ana', 'not_paid', unpaidNote('Ana')),
  draft(RSVP_OPEN_PEOPLE, 'Kai', 'not_paid', unpaidNote('Kai'), at(8, '18:40'), at(10, '18:40')),
  draft(RSVP_OPEN_PEOPLE, 'Ravi', 'not_paid', unpaidNote('Ravi')),
  draft(RSVP_OPEN_PEOPLE, 'Tom', 'not_paid', unpaidNote('Tom')),
];
const gatherBase = {
  host_name: 'Oleks',
  show_names: true,
  members: 22,
  not_answered: [],
  people: [],
  drafts: [],
  join: { mode: 'anywhere', anywhere_kind: 'funnel', anywhere_url: GATHER_FUNNEL, wifi_url: GATHER_WIFI },
};
const gatherFixture = (reply: string, name: string, state: 'live' | 'ended', gather: Record<string, unknown>): Fixture =>
  pocketFixture(reply, {
    provenance: { kind: 'kwant_data', ref: 'pocket:runclub' },
    code: 'runclub',
    kit: 'gather',
    name,
    when: null,
    state,
    budget: { calls: 3, usd: 0.01 },
    gather: { ...gatherBase, ...gather },
  });
const rsvpOpen = {
  face: 'rsvp',
  phase: 'open',
  title: 'End-of-season dinner',
  when: at(12, '19:30'),
  place: 'The Old Mill',
  price: '€35 each, paid to Oleks',
  closes_at: at(10, '20:00'),
  event_at: at(12, '19:30'),
  expires_at: at(19, '19:30'),
  not_answered: ['Rosa', 'Theo'],
  ...rsvpParts(RSVP_OPEN_PEOPLE),
  people: RSVP_OPEN_PEOPLE.map(person),
  drafts: RSVP_DRAFTS,
};
type Who = [id: string, name: string, note: string | null];
const slot = (i: number, name: string, cap: number, who: Who[], waiting: Array<[string, string]> = []) => ({
  slot: i, name, cap,
  people: who.map(([id, n, note], j) => ({ id, name: n, avatar: av(i + j), note })),
  waiting: waiting.length,
  waiting_people: waiting.map(([id, n], j) => ({ id, name: n, avatar: av(i + j + 3) })),
});
const POTLUCK_SLOTS = [
  slot(0, 'Mains', 3, [['g3', 'Priya', 'lentil stew']]),
  slot(1, 'Sides', 4, [['g4', 'Ana', null], ['g5', 'Kai', null], ['g7', 'Tom', null]]),
  slot(2, 'Desserts', 2, [['g2', 'Mia', 'apple cake'], ['g13', 'Ben', 'brownies']], [['g20', 'Hana'], ['g8', 'Leo']]),
  slot(3, 'Drinks', 2, [['g15', 'Omar', null]]),
  slot(4, 'Washing-up crew', 3, []),
];
const signupOpen = {
  face: 'signup',
  phase: 'open',
  title: 'Christmas potluck',
  when: at(20, '13:00'),
  place: 'the clubhouse',
  closes_at: at(19, '18:00'),
  event_at: at(20, '13:00'),
  expires_at: at(27, '13:00'),
  slots: POTLUCK_SLOTS,
  moved_up: [],
};
const OPTIONS = ['The Old Mill', 'Casa Lupa', 'Golden Lotus', 'Pizza at the clubhouse'].map((name, option) => ({ option, name }));
const VOTERS = ['Mia', 'Priya', 'Jonas', 'Ana', 'Kai', 'Oleks', 'Elif', 'Marta', 'Ben', 'Lucia', 'Omar', 'Hana', 'Tom', 'Ravi', 'Zoe'].map((name, i) => ({ id: `v${i + 1}`, name, avatar: av(i + 1), at: at(1, '19:00') }));
const c = (...votes: number[]) => votes.map((v, option) => ({ option, votes: v })).filter((x) => x.votes >= 0);
const ROUND1 = c(6, 5, 3, 1);
const voteBase = {
  face: 'vote',
  title: "Where's the end-of-season dinner?",
  closes_at: at(2, '20:00'),
  expires_at: at(9, '20:00'),
  not_answered: ['Leo', 'Finn', 'Nina', 'Sam', 'Ida', 'Rosa', 'Theo'],
  voted: 15,
  voters: VOTERS,
};
const voteOpen = { ...voteBase, phase: 'open', ballot: { kind: 'ranked', options: OPTIONS, in_play: [0, 1, 2, 3] }, live: ROUND1 };
const ROUNDS_DONE = [
  { round: 1, counts: ROUND1, exhausted: 0, dropped: [{ option: 3 }] },
  { round: 2, counts: c(6, 6, 3), exhausted: 0, dropped: [{ option: 2 }] },
  { round: 3, counts: c(8, 7), exhausted: 0, dropped: [] },
];
// Closed RSVP (results-1440 left): 19 coming (15 + 4), 4 no, 3 never answered, paid 14 of 15.
const RSVP_CLOSED_PEOPLE: P[] = [
  ['Oleks', 'yes', 1, ['vegetarian'], true], ['Mia', 'yes', 1, [], true], ['Priya', 'yes', 0, ['vegan'], true], ['Jonas', 'yes', 0, [], true], ['Ana', 'yes', 0, ['nut_allergy'], true],
  ['Kai', 'yes', 0, [], true], ['Ravi', 'yes', 0, ['vegetarian'], true], ['Tom', 'yes', 0, [], true], ['Elif', 'yes', 1, ['vegetarian'], true], ['Marta', 'yes', 0, ['gluten_free'], true],
  ['Ben', 'yes', 0, [], true], ['Lucia', 'yes', 0, ['vegetarian'], true], ['Omar', 'yes', 0, [], false], ['Hana', 'yes', 0, [], true], ['Leo', 'yes', 1, [], true],
  ['Finn', 'no', 0, [], false], ['Ida', 'no', 0, [], false], ['Nina', 'no', 0, [], false], ['Sam', 'no', 0, [], false],
];
const rsvpClosed = {
  ...rsvpOpen,
  phase: 'closed',
  not_answered: ['Zoe', 'Rosa', 'Theo'],
  ...rsvpParts(RSVP_CLOSED_PEOPLE),
  people: RSVP_CLOSED_PEOPLE.map(person),
  drafts: [draft(RSVP_CLOSED_PEOPLE, 'Omar', 'not_paid', unpaidNote('Omar'))],
  join: null,
};
const signupClosed = {
  ...signupOpen,
  phase: 'closed',
  slots: [
    slot(0, 'Mains', 3, [['g3', 'Priya', 'lentil stew'], ['g11', 'Elif', 'lasagne'], ['g15', 'Omar', 'curry']]),
    slot(1, 'Sides', 4, [['g4', 'Ana', null], ['g5', 'Kai', null], ['g7', 'Tom', null], ['g14', 'Lucia', null]]),
    slot(2, 'Desserts', 2, [['g2', 'Mia', 'apple cake'], ['g20', 'Hana', 'gingerbread']], [['g8', 'Leo']]),
    slot(3, 'Drinks', 2, [['g15', 'Omar', null], ['g10', 'Jonas', null]]),
    slot(4, 'Washing-up crew', 3, [['g12', 'Marta', null], ['g6', 'Ravi', null]]),
  ],
  join: null,
};
const voteClosed = {
  ...voteBase,
  phase: 'closed',
  join: null,
  ballot: { kind: 'ranked', options: OPTIONS, in_play: [0, 1, 2, 3] },
  result: { outcome: 'winner', winner: { option: 0 }, tied: [], ballots_counted: 15, rounds: ROUNDS_DONE },
};
const HANA_NOTE = "Hi Hana! A spot opened up in Desserts for Sunday's potluck, so you're in now. See you at 1 PM at the clubhouse!";
export const GATHER_FIXTURES = {
  rsvpOpen: gatherFixture("Your dinner RSVP is open. Share the link in the club chat; I'll keep count and draft nudges for you to send.", 'Dinner RSVP', 'live', rsvpOpen),
  rsvpPeople: gatherFixture("Here's who answered. Paid ticks are yours only.", 'Dinner RSVP', 'live', rsvpOpen),
  rsvpNudges: gatherFixture('8 nudges are drafted. Nothing sends by itself: copy one and send it from your own chat.', 'Dinner RSVP', 'live', rsvpOpen),
  rsvpNoAddress: gatherFixture('Your dinner RSVP is open on your Wi-Fi. To let members answer from home, set up Tailscale Funnel (the How to has the steps).', 'Dinner RSVP', 'live', {
    ...rsvpOpen,
    join: { mode: 'wifi', anywhere_kind: null, anywhere_url: null, wifi_url: GATHER_WIFI },
  }),
  signupOpen: gatherFixture('Your potluck sign-up is open. Desserts is full; Hana and Leo are on its waiting list.', 'Potluck sign-up', 'live', signupOpen),
  signupMoved: gatherFixture('Ben left Desserts, so Hana moved up from the waiting list.', 'Potluck sign-up', 'live', {
    ...signupOpen,
    slots: POTLUCK_SLOTS.map((s) => (s.slot === 2 ? slot(2, 'Desserts', 2, [['g2', 'Mia', 'apple cake'], ['g20', 'Hana', 'gingerbread']], [['g8', 'Leo']]) : s)),
    moved_up: [{ id: 'g20', name: 'Hana', slot: 2, at: at(14, '16:12'), seen: false }],
    drafts: [{ to: 'Hana', player_id: 'g20', kind: 'moved_up', text: HANA_NOTE, copied_at: null, next_copy_at: null, can_copy: true }],
  }),
  voteOpen: gatherFixture("The dinner vote is open. 15 of 22 have voted; it closes Wed at 8 PM.", 'Dinner vote', 'live', voteOpen),
  voteClosed: gatherFixture('The vote is closed: The Old Mill won, 8 to 7 in the final round.', 'Dinner vote', 'ended', voteClosed),
  voteTie: gatherFixture("It's a tie between The Old Mill and Casa Lupa. You pick, or open the vote again for 24 hours.", 'Dinner vote', 'live', {
    ...voteBase,
    phase: 'tie',
    join: null,
    voted: 14,
    voters: VOTERS.slice(0, 14),
    ballot: { kind: 'ranked', options: OPTIONS, in_play: [0, 1] },
    result: { outcome: 'tie', winner: null, tied: c(7, 7), ballots_counted: 14, rounds: [{ ...ROUNDS_DONE[0], counts: c(6, 5, 2, 1), dropped: [{ option: 2 }, { option: 3 }] }, { round: 2, counts: c(7, 7), exhausted: 0, dropped: [] }] },
  }),
  rsvpResult: gatherFixture('Your dinner RSVP is closed. 19 are coming; the list for The Old Mill is ready to copy.', 'Dinner RSVP', 'ended', rsvpClosed),
  signupResult: gatherFixture('Your potluck sign-up is closed. Every slot is full except Washing-up crew.', 'Potluck sign-up', 'ended', signupClosed),
} satisfies Record<string, Fixture>;

export const FIXTURES = {
  comparison,
  chartLine,
  chartBar,
  metrics,
  facts,
  steps,
  checklist,
  timeline,
  cards,
  findings,
  verification,
  files,
  recipe,
  pocket: POCKET_FIXTURES.ready,
} satisfies Record<string, Fixture>;

// ---------------------------------------------------------------------------------------------------
// pa4c: a quiz draft for Edit questions (Juno's GET /api/quiz-drafts/:id shape) and the card it sends back.
// The sample questions and sources are made up for the fixtures.

const DRAFT_ROUNDS = [
  { n: 1, title: 'Music' },
  { n: 2, title: 'Sport' },
  { n: 3, title: 'Pictures' },
];
type DQ = [round: number, question: string, answer: string, accepted: string[], source: string];
const DRAFT_QS: DQ[] = [
  [1, 'Which band released the album "Rumours" in 1977?', 'Fleetwood Mac', [], 'https://en.wikipedia.org/wiki/Rumours_(album)'],
  [1, 'How many strings does a standard cello have?', '4', ['four'], 'https://www.britannica.com/art/cello'],
  [1, 'Which singer is known as the "Queen of Soul"?', 'Aretha Franklin', ['Aretha'], 'https://en.wikipedia.org/wiki/Aretha_Franklin'],
  [2, 'How many players does a rugby union team have on the pitch?', '15', ['fifteen'], 'https://www.world.rugby/the-game/laws'],
  [2, 'In which city were the 2012 Summer Olympics held?', 'London', [], 'https://olympics.com/en/olympic-games/london-2012'],
  [3, 'Which bird is on the New Zealand dollar coin?', 'Kiwi', ['Kiwis', 'Kiwi bird'], 'https://www.rbnz.govt.nz/money-and-cash/coins'],
  [3, 'Which landmark is in picture 2?', 'Eiffel Tower', ['Eiffel', 'Tour Eiffel'], 'https://www.toureiffel.paris/en'],
  [3, 'Which animal is in picture 3?', 'Red panda', ['Lesser panda'], 'https://www.britannica.com/animal/red-panda'],
];

/** A draft in the given state, every question checked unless `yours` lists question numbers. */
export function quizDraftFixture(over: { state?: string; yours?: number[]; updated_at?: number; questions?: DQ[] } = {}) {
  const qs = (over.questions ?? DRAFT_QS).map(([round, question, answer, accepted, source], i) => {
    const n = i + 1;
    const own = (over.yours ?? []).includes(n);
    return {
      n,
      round,
      at: `R${round}`,
      kind: /^\d+$/.test(answer) ? 'number' : 'text',
      question,
      answer,
      accepted,
      status: own ? 'yours' : 'checked',
      source_url: own ? null : source,
      why: null,
      picture: null,
    };
  });
  const yours = qs.filter((q) => q.status === 'yours').length;
  const checked = qs.length - yours;
  const state = over.state ?? 'ready';
  return {
    id: 'qd_fq7k2m',
    title: 'Friday Pub Quiz',
    state,
    chip: state === 'ready' ? 'Ready' : state,
    kwant_id: 'k-quizmaster',
    pocket_code: 'fq7k2m',
    updated_at: over.updated_at ?? 1_763_900_000_000,
    counts: { questions: qs.length, checked, yours, not_checked: 0 },
    header: `${qs.length} questions · ${checked} checked · ${yours} yours`,
    dry_run: { ok: true, summary: 'Dry run passed with 30 pretend players.', problems: [] },
    problems: [],
    rounds: DRAFT_ROUNDS,
    questions: qs,
  };
}

/** The pa3 card a draft reply carries. */
export function draftCardFixture(d: ReturnType<typeof quizDraftFixture>) {
  return {
    id: d.id,
    kit: 'quiz',
    title: d.title,
    when: null,
    state: d.state,
    chip: d.chip,
    rounds: d.rounds.length,
    questions: d.counts.questions,
    checked: d.counts.checked,
    total: d.counts.checked,
    yours: d.counts.yours,
    failed: [],
    problems: d.problems,
    dry_run: d.dry_run,
    lines: [],
    notes: [],
    budget: null,
    pocket: d.pocket_code,
    next: null,
  };
}
