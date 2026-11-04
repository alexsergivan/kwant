// A bot's name in the sidebar wraps only between words (or after a hyphen), never inside a word:
// "Researche / r" next to the "Needs you" pill (Oleks, 0.6.11). The pill on the name line is the
// compact hand (plus a count past one); the words go on the status line. No row grows on hover, and
// a bot with no title keeps a status line so Edit and Remove never sit on a two-line name (#290 N1).
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'DOMParser', 'localStorage', 'sessionStorage', 'getComputedStyle']) {
  if (k in dom.window) Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}

const m = await loadBundled(`
  export { default as React } from 'react';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { Sidebar } from './src/components/Sidebar';
`);
const { React } = m;
const noop = () => {};
const space = { id: 'spc_default', name: 'Default', icon: '💼', color: '#f59e0b', sort: 0, is_default: true, archived_at: null,
  created_at: '', general_channel_id: null, counts: { bots: 0, ready_accounts: 0, unread: 0 }, approvals: 'on' };
const bot = (id, name, title = 'Role', over = {}) => ({ id, name, title, persona_prompt: '', avatar_sprite: 'mouse', driver_type: 'claude', preferred_model: '', ...over });
const NAMES = ['Researcher', 'Booktun.ing Marketer', 'Low content book trending and low competetive idea finder', 'Abcdefghijklmnopqrstuvwxyzabcd'];
const kwants = [...NAMES.map((n, i) => bot(`k${i}`, n)), bot('k9', 'Untitled Researcher Bot', '')];
const team = { id: 't1', name: 'Ship', icon: '🚢', lead_kwant_id: 'k0', member_kwants: ['k0', 'k1', 'k9'] };
const render = (over = {}) => m.renderToStaticMarkup(React.createElement(m.Sidebar, {
  space, spaces: [space], onSwitchSpace: noop, onCreateSpace: noop, onOpenSpaceSettings: noop, onOpenArchived: noop, onOpenSearch: noop,
  kwants, channels: [], teams: [team], selectedId: '', isChannel: true, onSelectKwant: noop, onSelectChannel: noop, onSelectTeam: noop,
  onSummonKwant: noop, onCreateTeam: noop, onOpenInbox: noop, onOpenParallel: noop, onOpenMissions: noop, missionBoard: true,
  onEditKwant: noop, onUpdateTeamMembers: noop, needsYou: { k0: 1, k1: 2, k2: 1, k3: 3 }, ...over,
}));
const doc = (html) => new DOMParser().parseFromString(`<body>${html}</body>`, 'text/html');
// Every class that lets a browser break inside a word
const MID_WORD = /overflow-wrap:anywhere|overflow-wrap:break-word|word-break:break-all|word-break:break-word|\bbreak-all\b|\bbreak-words\b|\bhyphens-auto\b/;


for (const phone of [false, true]) {
  test(`a name never breaks inside a word, on up to two lines, ending in … if a word is too long (phone=${phone})`, () => {
    const d = doc(render({ phone }));
    const names = [...d.querySelectorAll('[data-bot-name]')];
    assert.ok(names.length >= kwants.length, 'every bot row has a name');
    for (const el of names) {
      const cls = el.getAttribute('class');
      assert.doesNotMatch(cls, MID_WORD, `${el.textContent}: no mid-word break class`);
      assert.match(cls, /\[overflow-wrap:normal\]/, el.textContent);
      assert.match(cls, /\[word-break:normal\]/, el.textContent);
      assert.match(cls, /\bline-clamp-2\b/, el.textContent);
      assert.match(cls, /\btext-ellipsis\b/, el.textContent);
      assert.doesNotMatch(cls, /\btruncate\b|whitespace-nowrap/, `${el.textContent}: still wraps between words`);
      assert.match(el.getAttribute('title'), new RegExp(`^${el.textContent.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')} · `), 'the whole name is in the tooltip');
    }
    // Nothing on the name line lets the name's parents break words either
    for (const line of d.querySelectorAll('[data-bot-name-line]')) assert.doesNotMatch(line.getAttribute('class'), MID_WORD);
  });

  test(`the Needs you pill on the name line is the hand and a count, and the status line says it in words (phone=${phone})`, () => {
    const d = doc(render({ phone }));
    const pills = [...d.querySelectorAll('[data-bot-needs-you]')];
    assert.ok(pills.length >= 4, 'every bot with an open step has the pill');
    for (const p of pills) {
      const row = p.closest('[data-bot-row]');
      const name = row.querySelector('[data-bot-name]').textContent;
      const n = Number(p.getAttribute('data-bot-needs-you'));
      assert.equal(p.getAttribute('aria-label'), `Needs you (${n})`);
      assert.equal(p.getAttribute('title'), `${name} needs you in the browser`);
      assert.doesNotMatch(p.textContent, /Needs you/, 'no words on the name line');
      assert.equal(p.textContent, String(n), 'the hand and its count');
      assert.match(p.getAttribute('class'), /\bshrink-0\b/);
      for (const c of ['h-5', 'px-1.5', 'gap-1', 'rounded-full', 'text-[12px]', 'font-semibold', 'tabular-nums', 'text-[#fbbf24]', 'bg-[rgba(251,191,36,0.12)]', 'border-[rgba(251,191,36,0.35)]'])
        assert.ok(p.getAttribute('class').split(' ').includes(c), `pill spec: ${c}`);
      assert.doesNotMatch(p.getAttribute('class'), /holo/, 'never holographic');
      assert.match(p.innerHTML, /width="14"/, 'hand 14 px');
      const status = row.querySelector('[data-bot-busy]');
      assert.ok(status, `${name}: a status line`);
      assert.equal(status.textContent, 'Needs you in the browser');
      assert.match(status.getAttribute('class'), /\btext-warn\b/);
    }
  });
}

test('a bot that needs you because its chat waits on the browser keeps where (no doubled words)', () => {
  const d = doc(render({ busyKwants: { k0: 'Needs you in the browser in Engineering Room', k1: 'Working here' } }));
  const line = (id) => d.querySelector(`[data-bot-row="${id}"] [data-bot-busy]`).textContent;
  assert.equal(line('k0'), 'Needs you in the browser in Engineering Room');
  assert.equal(line('k1'), 'Needs you in the browser', 'an open step outranks "Working" while it is open');
  const free = doc(render({ needsYou: {}, busyKwants: { k1: 'Working here' } }));
  assert.equal(free.querySelector('[data-bot-row="k1"] [data-bot-busy]').textContent, 'Working here', 'back to the normal status once done');
  assert.equal(free.querySelectorAll('[data-bot-needs-you]').length, 0);
});

test('hover and focus never change the name line, and a bot with no title still reserves its status line (#290 N1)', () => {
  const d = doc(render({}));
  for (const line of d.querySelectorAll('[data-bot-name-line]')) assert.doesNotMatch(line.getAttribute('class'), /group-(hover|focus-within)\//, 'the name line has no hover/focus classes');
  for (const el of d.querySelectorAll('[data-bot-name], [data-bot-needs-you]')) assert.doesNotMatch(el.getAttribute('class'), /group-(hover|focus-within)\//);
  const row = d.querySelector('[data-bot-row="k9"]');
  const status = row.querySelector('[data-bot-name-line] + p');
  assert.equal(status.textContent, '');
  assert.match(status.getAttribute('class'), /min-h-\[1\.125rem\]/, 'one text-meta line high even with no title');
  const src = readFileSync(new URL('../tailwind.config.js', import.meta.url), 'utf8');
  assert.match(src, /meta: \['0\.75rem', \{ lineHeight: '1\.125rem' \}\]/, 'the reserved height is text-meta\'s line height');
});
