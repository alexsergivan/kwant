// 28h (autonomy/answered-card-says-so/brief.md): once the person answers an approval card, its step says what they
// chose ("You said OK: running a command"), in Details and in the "What <bot> did" trail, not "Waiting for your OK".
// The tests marked "fails on main" fail on c6598d24; the guards pass on both.
process.env.TZ = 'UTC';
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { SavedActivityList } from './src/components/ActivityList';
  export { readableStepTitle } from './src/toolLabel';
  export { trailFrom } from './src/doneAway';
  export const steps = (props) => renderToStaticMarkup(React.createElement(SavedActivityList, props));
`);

const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/&amp;/g, '&').replace(/&#x27;/g, "'").replace(/\s+/g, ' ').trim();

const CMD = 'Waiting for your approval: Running a command';
const PR = 'Waiting for your approval: Using create_pull_request (github)';
const HA = 'Waiting for your approval: Using intent__HassTurnOff (home-assistant)';

test('an answered approval says what you chose, with the -ing phrase (fails on main)', () => {
  assert.equal(m.readableStepTitle(CMD, 'ok'), 'You said OK: running a command');
  assert.equal(m.readableStepTitle(CMD, 'declined'), 'You said no: running a command');
  assert.equal(m.readableStepTitle(CMD, 'unanswered'), 'Asked for your OK: running a command');
  assert.equal(m.readableStepTitle(CMD, 'stopped'), 'Asked for your OK: running a command');
  assert.equal(m.readableStepTitle(PR, 'ok'), 'You said OK: opening a pull request on GitHub');
  assert.equal(m.readableStepTitle(PR, 'declined'), 'You said no: opening a pull request on GitHub');
  assert.equal(m.readableStepTitle(HA, 'ok'), 'You said OK: turning something off');
  assert.equal(m.readableStepTitle(HA, 'unanswered'), 'Asked for your OK: turning something off');
});

test('guard: no status, waiting or running read exactly as today', () => {
  for (const st of [undefined, 'waiting', 'running']) {
    assert.equal(m.readableStepTitle(CMD, st), 'Waiting for your OK: running a command', String(st));
    assert.equal(m.readableStepTitle(PR, st), 'Waiting for your OK to open a pull request on GitHub', String(st));
    assert.equal(m.readableStepTitle(HA, st), 'Waiting for your OK to turn something off', String(st));
  }
});

test('guard: a title that is not an approval is unchanged by any status', () => {
  for (const st of [undefined, 'ok', 'declined', 'unanswered', 'stopped', 'failed']) {
    assert.equal(m.readableStepTitle('Using `web_search`', st), 'Searching the web', String(st));
    assert.equal(m.readableStepTitle('Running `ls`', st), 'Running `ls`', String(st));
    assert.equal(m.readableStepTitle('Running `echo Waiting for your approval: x`', st), 'Running `echo Waiting for your approval: x`', String(st));
  }
});

const T0 = '2026-11-17T18:00:00Z';
const ap = (id, status) => ({ id, kind: 'approval', status, title: CMD, tool: 'Bash', started_at: T0, ended_at: T0 });

test('Details: one reply with an allowed, a denied and an unanswered card (fails on main)', () => {
  const html = m.steps({ activity: { steps: [ap('a1', 'ok'), ap('a2', 'declined'), ap('a3', 'unanswered')], dropped: 0 }, idBase: 'r1', defaultOpen: true });
  const t = text(html);
  assert.match(t, /You said OK: running a command/);
  assert.match(t, /You said no: running a command/);
  assert.match(t, /Asked for your OK: running a command/);
  assert.doesNotMatch(t, /Waiting for your OK/);
});

const msg = (status) => ({ id: 'm1', text: 'Done.', created_at: '2026-11-17T18:01:00Z', meta: { activity: { steps: [ap('a1', status)], dropped: 0 } } });

test('trail: an answered card reads as Details does (fails on main)', () => {
  assert.equal(m.trailFrom(msg('ok'), null)[0].what, 'You said OK: running a command');
  assert.equal(m.trailFrom(msg('declined'), null)[0].what, 'You said no: running a command');
});

test('trail: a card still waiting reads as Details does (fails on main: raw server words)', () => {
  assert.equal(m.trailFrom(msg('waiting'), null)[0].what, 'Waiting for your OK: running a command');
});
