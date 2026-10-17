import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import ts from 'typescript';

const load = async (path) => {
  const source = await readFile(new URL(path, import.meta.url), 'utf8');
  const { outputText } = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext } });
  return import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);
};

const c = await load('../src/continuity.ts');

test('snippets escape everything except <mark>', () => {
  const raw = 'see <script>alert("x")</script> & <b>the</b> <mark>plan</mark> <img src=x onerror=alert(1)>';
  const html = c.sanitizeSnippet(raw);
  assert.equal(
    html,
    'see &lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt; &amp; &lt;b&gt;the&lt;/b&gt; <mark>plan</mark> &lt;img src=x onerror=alert(1)&gt;',
  );
  assert.ok(!/<(?!\/?mark>)/.test(html), 'no tag other than mark survives');
});

test('markup inside a hit stays text, and attributes on mark are not trusted', () => {
  assert.equal(c.sanitizeSnippet('<mark><i>x</i></mark>'), '<mark>&lt;i&gt;x&lt;/i&gt;</mark>');
  assert.equal(c.sanitizeSnippet('<mark onclick="x">y</mark>'), '&lt;mark onclick=&quot;x&quot;&gt;y');
  assert.deepEqual(c.snippetParts('a <mark>b</mark> c <mark>d</mark>'), [
    { text: 'a ', hit: false },
    { text: 'b', hit: true },
    { text: ' c ', hit: false },
    { text: 'd', hit: true },
  ]);
  assert.deepEqual(c.snippetParts(''), []);
});

test('groups hits by channel or direct chat, keeping rank order', () => {
  const hit = (id, channel_id, channel_name, kwant_id) => ({ message_id: id, channel_id, channel_name, kwant_id, author: 'a', snippet: '', created_at: null });
  const groups = c.groupHits(
    [hit('1', 'c1', 'general', null), hit('2', null, null, 'k1'), hit('3', 'c2', 'team-core', null), hit('4', 'c1', 'general', null)],
    (id) => (id === 'k1' ? 'Ada' : undefined),
  );
  assert.deepEqual(groups.map((g) => [g.label, g.hits.map((h) => h.message_id)]), [
    ['#general', ['1', '4']],
    ['Direct chat · Ada', ['2']],
    ['@core', ['3']],
  ]);
  assert.equal(groups[1].kwant_id, 'k1');
});

const form = (over) => ({ ...c.emptyScheduleForm(new Date('2026-10-17T08:00:00Z'), 'Europe/Kyiv'), ...over });
const NOW = Date.parse('2026-10-17T08:00:00Z');

test('schedule form → JSON for each type', () => {
  const local = '2026-10-18T09:30';
  assert.deepEqual(c.scheduleFromForm(form({ type: 'once', onceAt: local }), NOW), {
    schedule: { type: 'once', at: new Date(local).toISOString() },
  });
  assert.deepEqual(c.scheduleFromForm(form({ type: 'daily', time: '07:05' }), NOW), {
    schedule: { type: 'daily', time: '07:05', tz: 'Europe/Kyiv' },
  });
  assert.deepEqual(c.scheduleFromForm(form({ type: 'weekly', time: '18:00', weekdays: ['sun', 'mon', 'wed'] }), NOW), {
    schedule: { type: 'weekly', time: '18:00', weekdays: ['mon', 'wed', 'sun'], tz: 'Europe/Kyiv' },
  });
  assert.deepEqual(c.scheduleFromForm(form({ type: 'interval', everyMinutes: '90' }), NOW), {
    schedule: { type: 'interval', every_minutes: 90 },
  });
});

test('schedule form rejects what the server rejects', () => {
  const err = (over) => c.scheduleFromForm(form(over), NOW).error;
  assert.ok(err({ type: 'once', onceAt: '2026-10-01T09:00' }));
  assert.ok(err({ type: 'once', onceAt: '' }));
  assert.ok(err({ type: 'daily', time: '24:00' }));
  assert.ok(err({ type: 'daily', time: '9:00' }));
  assert.ok(err({ type: 'daily', tz: '  ' }));
  assert.ok(err({ type: 'weekly', weekdays: [] }));
  for (const n of ['4', '10081', '7.5', '', 'abc']) assert.ok(err({ type: 'interval', everyMinutes: n }), n);
  assert.equal(err({ type: 'interval', everyMinutes: '5' }), undefined);
  assert.equal(err({ type: 'interval', everyMinutes: '10080' }), undefined);
});

test('a saved schedule round-trips through the form', () => {
  for (const s of [
    { type: 'daily', time: '06:15', tz: 'America/New_York' },
    { type: 'weekly', time: '12:00', weekdays: ['tue', 'fri'], tz: 'UTC' },
    { type: 'interval', every_minutes: 45 },
    { type: 'once', at: '2026-12-01T10:00:00.000Z' },
  ]) {
    assert.deepEqual(c.scheduleFromForm(c.formFromSchedule(s), NOW).schedule, s);
  }
});

test('time zone defaults to the browser and falls back to UTC', () => {
  assert.equal(c.defaultTimeZone(), Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC');
  assert.equal(c.emptyScheduleForm().tz, c.defaultTimeZone());
  const original = Intl.DateTimeFormat;
  try {
    Intl.DateTimeFormat = () => ({ resolvedOptions: () => ({ timeZone: undefined }) });
    assert.equal(c.defaultTimeZone(), 'UTC');
    Intl.DateTimeFormat = () => {
      throw new Error('no Intl');
    };
    assert.equal(c.defaultTimeZone(), 'UTC');
  } finally {
    Intl.DateTimeFormat = original;
  }
  assert.ok(c.timeZoneOptions('Mars/Base').includes('Mars/Base'));
  try {
    Intl.DateTimeFormat = () => ({ resolvedOptions: () => ({ timeZone: 'Asia/Calcutta' }) });
    assert.equal(c.defaultTimeZone(), 'Asia/Kolkata');
  } finally {
    Intl.DateTimeFormat = original;
  }
  assert.ok(!c.timeZoneOptions('UTC').includes('Europe/Kiev'));
  assert.deepEqual(c.scheduleFromForm(form({ type: 'daily', tz: 'Europe/Kiev' }), NOW).schedule.tz, 'Europe/Kyiv');
  assert.ok(c.timeZoneOptions('').includes('UTC'));
});

test('OS notifications only when on, granted, hidden and a known kind', () => {
  const ok = { enabled: true, permission: 'granted', hidden: true, kind: 'turn_finished' };
  assert.equal(c.shouldShowOsNotification(ok), true);
  assert.equal(c.shouldShowOsNotification({ ...ok, kind: 'routine_failed' }), true);
  assert.equal(c.shouldShowOsNotification({ ...ok, enabled: false }), false);
  assert.equal(c.shouldShowOsNotification({ ...ok, permission: 'default' }), false);
  assert.equal(c.shouldShowOsNotification({ ...ok, permission: 'denied' }), false);
  assert.equal(c.shouldShowOsNotification({ ...ok, permission: 'unsupported' }), false);
  assert.equal(c.shouldShowOsNotification({ ...ok, hidden: false }), false);
  assert.equal(c.shouldShowOsNotification({ ...ok, kind: 'something_else' }), false);
});

test('browser toggle is read per browser and defaults off', () => {
  const store = (v) => ({ getItem: (k) => (k === c.BROWSER_NOTIFY_KEY ? v : null) });
  assert.equal(c.browserNotifyEnabled(store('on')), true);
  assert.equal(c.browserNotifyEnabled(store('off')), false);
  assert.equal(c.browserNotifyEnabled(store(null)), false);
  assert.equal(c.browserNotifyEnabled(undefined), false);
  assert.equal(c.browserNotifyEnabled({ getItem: () => { throw new Error('blocked'); } }), false);
});

test('webhook URL: https only, no credentials; mask and empty pass', () => {
  assert.equal(c.webhookUrlError(''), null);
  assert.equal(c.webhookUrlError(c.MASK), null);
  assert.equal(c.webhookUrlError('https://ntfy.sh/topic'), null);
  assert.ok(c.webhookUrlError('http://ntfy.sh/topic'));
  assert.ok(c.webhookUrlError('ftp://x'));
  assert.ok(c.webhookUrlError('ntfy.sh/topic'));
  assert.ok(c.webhookUrlError('https://user:pw@ntfy.sh/topic'));
});

test('continuity bus delivers to subscribers until they unsubscribe', () => {
  const seen = [];
  const off = c.onContinuity('memory_changed', (d) => seen.push(d.kwant_id));
  c.emitContinuity({ type: 'memory_changed', data: { kwant_id: 'k1' } });
  c.emitContinuity({ type: 'routine_run', data: { routine_id: 'r', run_id: 'x', status: 'ok' } });
  off();
  c.emitContinuity({ type: 'memory_changed', data: { kwant_id: 'k2' } });
  assert.deepEqual(seen, ['k1']);
});
