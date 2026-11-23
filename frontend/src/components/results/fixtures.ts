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
