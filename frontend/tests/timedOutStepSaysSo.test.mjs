// 28n (autonomy/timed-out-step-says-so/brief.md): a human step nobody finished in time (server `timed_out`) gets the
// unanswered look: the ⏱ amber icon, the amber chip and "Not done in time", not ✓ "Done". `status` stays `ok`.
// Tests marked "fails on c561af2e" fail there.
process.env.TZ = 'UTC';
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { SavedActivityList } from './src/components/ActivityList';
  export { parseActivityMeta, statusLabel, activitySummary } from './src/activity';
  export const steps = (props) => renderToStaticMarkup(React.createElement(SavedActivityList, props));
`);

const T0 = '2026-11-17T18:00:00Z';
const HS = 'Waiting for you: Solve the CAPTCHA on reviews.example.org in the browser panel.';
const hs = (id, status) => ({ id, kind: 'human_step', status, title: HS, tool: 'ask_human', ref_id: `h-${id}`, started_at: T0, ended_at: T0, duration_ms: 600000 });
const ap = (id, status) => ({ id, kind: 'approval', status, title: 'Waiting for your approval: Running a command', tool: 'Bash', started_at: T0, ended_at: T0 });
const parse = (raw) => m.parseActivityMeta({ activity: { steps: raw, dropped: 0 } });

/** The rendered <li> of step `id`, as HTML. */
const row = (html, id) => {
  const start = html.indexOf(`id="r1-steps-${id}"`);
  assert.ok(start >= 0, `row ${id}`);
  return html.slice(start, html.indexOf('</li>', start));
};
const look = (r) => ({
  label: r.match(/role="img" aria-label="([^"]+)"/)?.[1],
  mark: r.match(/data-step-mark="([^"]+)"/)?.[1] ?? null,
  chip: r.match(/data-step-mark="[^"]+"[^>]*>([^<]+)</)?.[1] ?? null,
  timer: /lucide-timer-off/.test(r),
  check: /lucide-check/.test(r),
  amberChip: /text-amber-300 border-amber-500\/30/.test(r),
});

test('a timed-out human step reads "Not done in time" (fails on c561af2e)', () => {
  const [s] = parse([hs('t', 'timed_out')]).steps;
  assert.equal(s.status, 'ok');
  assert.equal(s.outcome, 'timed_out');
  assert.equal(m.statusLabel(s), 'Not done in time');
});

test('Details: timed out gets the timer icon and the amber "Not done in time" chip (fails on c561af2e)', () => {
  const a = parse([hs('s0', 'done'), hs('s1', 'resolved_auto'), hs('s2', 'declined'), hs('s3', 'timed_out'), hs('s4', 'stopped')]);
  const html = m.steps({ activity: a, idBase: 'r1', defaultOpen: true });
  const t = look(row(html, 's3'));
  assert.deepEqual(t, { label: 'Not done in time', mark: 'unanswered', chip: 'Not done in time', timer: true, check: false, amberChip: true });
});

test('guard: done and resolved_auto keep ✓ Done, declined and stopped as before (28s: declined has no chip)', () => {
  const a = parse([hs('s0', 'done'), hs('s1', 'resolved_auto'), hs('s2', 'declined'), hs('s4', 'stopped')]);
  const html = m.steps({ activity: a, idBase: 'r1', defaultOpen: true });
  for (const id of ['s0', 's1']) {
    const r = look(row(html, id));
    assert.equal(r.label, 'Done', id);
    assert.equal(r.check, true, id);
    assert.equal(r.mark, null, id);
  }
  assert.equal(look(row(html, 's2')).label, 'You said no');
  // 28s (Mira's follow-up 2): a declined human step drops its chip; its title says "You said no: …"
  assert.equal(look(row(html, 's2')).chip, null);
  assert.equal(look(row(html, 's4')).label, 'Stopped');
  assert.equal(look(row(html, 's4')).mark, null);
});

test('guard: an unanswered approval is unchanged', () => {
  const html = m.steps({ activity: parse([ap('a1', 'unanswered')]), idBase: 'r1', defaultOpen: true });
  const r = look(row(html, 'a1'));
  assert.equal(r.label, 'Not approved in time');
  assert.equal(r.timer, true);
  assert.equal(r.chip, 'Not approved in time');
});

test('guard: counts read `status`, so the summary is as before (no "didn\'t run" for a human step)', () => {
  const a = parse([hs('t', 'timed_out')]);
  assert.doesNotMatch(m.activitySummary(a), /didn’t run/);
  const tool = parse([{ ...hs('x', 'timed_out'), kind: 'tool' }]);
  assert.notEqual(m.statusLabel(tool.steps[0]), 'Not done in time');
});
