// One sample bot reply and its `results` for every block type in plan §A.3, for building renderers and
// for tests. Names, prices and numbers are made up. Every value is copied from the reply text, the way
// the server copies it (or, for verification and files, from typed kwant data).

import type { ResultSet, SmartBlock } from '../../smartResults';

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
      { id: 'a2', attachment_id: 'att-shot', name: 'dns.png', mime: 'image/png', size_bytes: 48200, preview: { kind: 'image' } },
    ],
  }),
};

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
} satisfies Record<string, Fixture>;
