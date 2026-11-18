// 28z (autonomy/details-summary-fits-phone/brief.md): the Details summary wraps to two lines on a phone instead of
// cutting off its last count, and the saved summary counts steps with the same rule as the live "Working · N steps"
// (shownStepCount). Tests marked "fails on 770b27ee" fail there.
process.env.TZ = 'UTC';
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { SavedActivityList } from './src/components/ActivityList';
  export { parseActivityMeta, activitySummary, shownStepCount } from './src/activity';
  export const saved = (props) => renderToStaticMarkup(React.createElement(SavedActivityList, props));
`);

const at = (min) => new Date(Date.parse('2026-11-17T18:00:00Z') + min * 60000).toISOString();
const AP = 'Waiting for your approval: Running a command';
const ap = (id, status, t, tool = 'Bash') => ({ id, kind: 'approval', status, title: AP, tool, started_at: at(t), ended_at: at(t + 0.2), duration_ms: 12000 });
const tool = (id, status, title, t, name = 'Bash', detail) => ({ id, kind: 'tool', status, title, tool: name, started_at: at(t), ended_at: at(t + 0.5), duration_ms: 30000, ...(detail ? { detail } : {}) });
const DENIED = 'The user denied this tool call.';
const note = (id, t) => ({ id, kind: 'note', status: 'ok', title: 'Claude Code doesn’t share its thinking.', started_at: at(t), ended_at: at(t) });
const parse = (raw, dropped = 0) => m.parseActivityMeta({ activity: { steps: raw, dropped } });
const summaryCount = (a) => Number(/(\d+) steps?/.exec(m.activitySummary(a))[1]);

test('the summary’s step count is shownStepCount + dropped: folded pair, unpaired card, note, dropped', () => {
  const cases = {
    'folded pair': parse([ap('tk1', 'ok', 0), tool('t1', 'ok', 'Running `npm run build`', 0.2)]),
    'declined pair': parse([tool('t1', 'failed', 'Running `rm -rf dist`', 0, 'Bash', DENIED), ap('tk1', 'declined', 0.01)]),
    'unpaired card': parse([ap('tk1', 'unanswered', 0), tool('t1', 'ok', 'Read a.ts', 1, 'Read')]),
    'card for another tool': parse([ap('tk1', 'ok', 0, 'Write'), tool('t1', 'ok', 'Running `ls`', 0.2)]),
    note: parse([tool('t1', 'ok', 'Read a.ts', 0, 'Read'), note('n1', 0.5), tool('t2', 'ok', 'Edit a.ts', 1, 'Edit')]),
    dropped: parse([ap('tk1', 'ok', 0), tool('t1', 'ok', 'Running `npm run build`', 0.2), note('n1', 0.5)], 7),
  };
  const want = { 'folded pair': 1, 'declined pair': 1, 'unpaired card': 2, 'card for another tool': 2, note: 2, dropped: 8 };
  for (const [name, a] of Object.entries(cases)) {
    assert.equal(summaryCount(a), m.shownStepCount(a.steps) + a.dropped, name);
    assert.equal(summaryCount(a), want[name], name);
  }
});

test('activitySummary counts steps through shownStepCount, not its own copy of the rule (fails on 770b27ee)', () => {
  const src = readFileSync(new URL('../src/activity.ts', import.meta.url), 'utf8');
  const body = src.slice(src.indexOf('export function activitySummary'), src.indexOf('\n}\n', src.indexOf('export function activitySummary')));
  assert.match(body, /shownStepCount\(a\.steps\) \+ a\.dropped/);
  assert.doesNotMatch(body, /kind !== 'note'/, 'the note rule lives only in shownStepCount');
});

test('on a phone the summary wraps to two lines between its parts, left-aligned, one line from sm up; icons sit on the first line (fails on 770b27ee)', () => {
  const a = parse([tool('t1', 'failed', 'Running `rm -rf dist`', 0, 'Bash', DENIED), ap('tk1', 'declined', 0.01), tool('t2', 'ok', 'Read a.ts', 26, 'Read')]);
  const html = m.saved({ activity: a, idBase: 'r1' });
  const span = /<span class="([^"]*)" data-steps-summary="">(.*?)<\/span><svg/.exec(html);
  assert.ok(span, 'the summary span is there');
  const cls = span[1].split(' ');
  for (const c of ['min-w-0', 'text-left', 'break-words', 'max-sm:line-clamp-2', 'sm:truncate']) assert.ok(cls.includes(c), c);
  assert.ok(!cls.includes('truncate'), 'no one-line cut below sm');
  assert.equal(span[2].replace(/<[^>]+>/g, ''), m.activitySummary(a), 'the whole summary is in the DOM, same text');
  // a line breaks only between parts: each " · " part is one no-wrap piece
  const parts = [...span[2].matchAll(/<span class="whitespace-nowrap">([^<]*)<\/span>/g)].map((x) => x[1]);
  assert.deepEqual(parts, m.activitySummary(a).split(' · '));
  assert.ok(parts.includes('you said no to 1'));
  const button = /<button type="button" aria-expanded="false" class="([^"]*)"/.exec(html)[1].split(' ');
  assert.ok(button.includes('max-sm:items-start'), 'chevron and icon top-aligned on a phone');
  assert.ok(button.includes('items-center'), 'centred from sm up');
  assert.equal((html.match(/max-sm:mt-\[1\.5px\]/g) ?? []).length, 2, 'both icons line up with the first line');
});
