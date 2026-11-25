// 36c router-one-line (brief: autonomy/router-one-line/brief.md; Mira's simplify review rank 3): the router card
// is one line, "✦ Sent to Quant, the team lead · Why · Change". The role is said once, in words; the router's name
// and "Laya is offline" leave the chat (Details and Settings keep them); Why and Change are real buttons with
// aria-expanded; the deciding state is one line; under reduced motion nothing moves.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'DOMParser', 'localStorage', 'sessionStorage', 'getComputedStyle', 'requestAnimationFrame', 'cancelAnimationFrame', 'MutationObserver']) {
  if (k in dom.window) Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;
dom.window.matchMedia = () => ({ matches: false, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export { LayaRoutingCard } from './src/components/ChatView';
  export { routeLineWhy, routeRole, routeWhyText } from './src/routeReason';
  export { routingLine } from './src/spaces';
`);

const names = ['Quant', 'Backtester', 'Builder', 'Risk', 'DataAnalyst', 'Backend', 'QA', 'TechWriter', 'Scout'];
const candidates = names.map((n, i) => ({ id: 'k' + i, name: n, title: i === 0 ? 'Quant Researcher' : n }));
const OFFLINE = 'Laya is offline, so this was a keyword match';
const routed = (over = {}) => ({ phase: 'decided', contextId: 'c1', candidates, kwantId: 'k0', kwantName: 'Quant', kwantTitle: 'Quant Researcher', candidateCount: 9, ...over });

function mount(routing) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  const render = (r) => m.flushSync(() => root.render(m.React.createElement(m.LayaRoutingCard, { routing: r, spaceName: 'Binance desk' })));
  render(routing);
  return { host, render, unmount: () => (root.unmount(), host.remove()) };
}
// What the eye sees on a phone: the inline why is md-only, so it is left out here.
const phoneText = (host) => {
  const c = host.cloneNode(true);
  c.querySelectorAll('.md\\:inline, .hidden').forEach((e) => e.remove());
  return c.textContent.replace(/\s+/g, ' ').trim();
};
const click = (el) => m.flushSync(() => el.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true })));
const btn = (host, label) => [...host.querySelectorAll('button')].find((b) => b.textContent.trim() === label);

const LINES = [
  { case: 'lead', basis: 'lead', reason: "No one's role matched this, so it went to the channel lead, Quant (Quant Researcher)", line: 'Sent to Quant, the team lead', why: 'No one’s role matched this.' },
  { case: 'several (lead, multi-role)', basis: 'several', fallback: true, matchedTerms: ['backup', 'signals'], reason: `This touches several roles (Quant and Backend), so it went to the channel lead, Quant (Quant Researcher), to answer or hand parts on. ${OFFLINE}`, line: 'Sent to Quant, the team lead', why: 'It touches several roles, so the lead answers or hands parts on.' },
  { case: 'only', basis: 'only', reason: 'Quant is the only bot here that can answer', line: 'Sent to Quant, the only bot here', why: null },
  { case: 'skills', basis: 'skills', matchedTerms: ['backtest'], reason: 'Quant (Quant Researcher) does this kind of work: your message mentions backtest; Laya picked the same bot', line: 'Sent to Quant, Quant Researcher', why: 'Your message mentions backtest.' },
  { case: 'ask', basis: 'ask', line: 'Sent to Quant, Quant Researcher', why: 'You picked Quant.' },
  { case: 'handback', basis: 'handback', reason: 'Risk handed this back (not my area), so Quant takes it', line: 'Sent to Quant, Quant Researcher', why: 'Risk said it’s not their area, so Quant took it.' },
  { case: 'discuss', basis: 'discuss', mode: 'discuss', reason: 'This needs several perspectives — starting a 4-turn team discussion led by Quant', line: 'Team discussion led by Quant', why: 'This needs several points of view.' },
  { case: 'last', basis: 'last', reason: 'Nothing in your message named a role, so Quant (Quant Researcher), who spoke last here, carries on', line: 'Sent to Quant, Quant Researcher', why: 'No role was named, so the bot that spoke last carries on.' },
  { case: 'last + fallback', basis: 'last', fallback: true, reason: `Nothing in your message named a role, so Quant (Quant Researcher), who spoke last here, carries on. ${OFFLINE}`, line: 'Sent to Quant, Quant Researcher', why: 'No role was named, so the bot that spoke last carries on.' },
  { case: 'skills, named', basis: 'skills', matchedTerms: ['quant'], reason: 'Quant was named in your message', line: 'Sent to Quant, Quant Researcher', why: 'You named them in your message.' },
  { case: 'keywords fallback', basis: 'keywords', fallback: true, matchedTerms: ['strategy', 'go'], reason: `Quant (Quant Researcher) does this kind of work: your message mentions strategy, go. ${OFFLINE}`, line: 'Sent to Quant, Quant Researcher', why: 'Your message mentions strategy, go.' },
];

for (const c of LINES) {
  test(`one line, ${c.case}: "${c.line}", the role once, no router name, no offline`, () => {
    const { host, unmount } = mount(routed({ basis: c.basis, reason: c.reason, fallback: c.fallback, matchedTerms: c.matchedTerms, mode: c.mode }));
    try {
      const t = phoneText(host);
      assert.ok(t.startsWith(c.line), `${t} starts with ${c.line}`);
      assert.doesNotMatch(host.textContent, /Laya|offline|router|Picked by keywords/i, 'the router and its offline note stay out of the chat');
      assert.doesNotMatch(host.textContent, /Quant \(Quant Researcher\)/, 'never the name with its role in brackets');
      assert.equal((host.textContent.match(/Quant Researcher/g) ?? []).length, c.line.includes('Quant Researcher') ? 1 : 0, 'the role is said once');
      assert.equal(host.querySelectorAll('[data-route-line] [title]').length, 0, 'no chip wall until Change');
      assert.equal(m.routeLineWhy({ kwantName: 'Quant', ...c }), c.why);
      const inline = host.querySelector('.md\\:inline');
      assert.equal(inline ? inline.textContent.replace(/^ · /, '') : null, c.why, 'desktop shows the short why inline');
      assert.equal(!!btn(host, 'Why'), !!c.why, 'the phone gets "Why" only when there is one');
      assert.equal(host.querySelector('[data-route-line]').getAttribute('aria-live'), null, 'not on the outer box');
      assert.equal(host.querySelector('[data-route-line] > div').getAttribute('aria-live'), 'polite', 'only the first row is live');
    } finally {
      unmount();
    }
  });
}

test('"Laya is offline" leaves the chat line but stays in Details and the space line', () => {
  const r = { kwantName: 'Quant', basis: 'keywords', fallback: true, matchedTerms: ['strategy'] };
  assert.doesNotMatch(m.routeLineWhy(r) ?? '', /Laya|offline/);
  assert.match(m.routeWhyText(r), /Laya, who picks the bot, was offline/);
  assert.match(m.routingLine({ kwant_name: 'Quant', fallback: true, candidate_count: 9 }, 'Binance desk'), /Laya is offline/);
});

test('Why and Who else are buttons that toggle aria-expanded and open in place', () => {
  const { host, unmount } = mount(routed({ basis: 'lead' }));
  try {
    const why = btn(host, 'Why');
    const change = btn(host, 'Who else');
    assert.equal(why.getAttribute('aria-expanded'), 'false');
    assert.equal(change.getAttribute('aria-expanded'), 'false');
    assert.equal(document.getElementById(why.getAttribute('aria-controls')), null);
    click(why);
    assert.equal(why.getAttribute('aria-expanded'), 'true');
    assert.match(document.getElementById(why.getAttribute('aria-controls')).textContent, /No one’s role matched this\./);
    click(change);
    assert.equal(change.getAttribute('aria-expanded'), 'true');
    const group = document.getElementById(change.getAttribute('aria-controls'));
    assert.equal(group.getAttribute('role'), 'group');
    assert.equal(group.querySelectorAll(':scope > div > span').length, 9, 'Change shows all 9 candidates');
    assert.match(group.textContent, /After Quant replies, tap “Wrong bot\?” in its Details/, 'and points at the reply’s existing way to change');
    for (const chip of group.querySelectorAll(':scope > div > span')) {
      if (/Quant/.test(chip.textContent)) assert.match(chip.className, /bg-sky-500\/20/);
      else assert.match(chip.className, /border-transparent bg-transparent text-zinc-400 cursor-default/, 'view-only chips don’t look tappable');
    }
    click(change);
    click(why);
    assert.equal(change.getAttribute('aria-expanded'), 'false');
    assert.equal(why.getAttribute('aria-expanded'), 'false');
    assert.equal(host.querySelector('[role="group"]'), null);
    // 44 px touch area on phones, only the drawn size on desktop.
    assert.match(why.className, /min-h-\[44px\]/);
    assert.match(why.className, /md:hidden/);
    assert.match(change.className, /md:min-h-0/);
  } finally {
    unmount();
  }
});

test('Who else hides when there is no one else to pick', () => {
  const { host, unmount } = mount(routed({ basis: 'only', candidates: [candidates[0]] }));
  try {
    assert.equal(btn(host, 'Who else'), undefined);
    assert.equal(btn(host, 'Why'), undefined);
  } finally {
    unmount();
  }
});

test('deciding: one line with three dots, no chip wall', () => {
  const { host, unmount } = mount({ phase: 'deciding', contextId: 'c1', candidates, candidateCount: 9 });
  try {
    assert.equal(host.textContent.trim(), 'Choosing who answers');
    assert.equal(host.querySelectorAll('[title]').length, 0);
    assert.equal(host.querySelectorAll('button').length, 0);
    assert.equal(host.querySelectorAll('[aria-hidden="true"] > span').length, 3);
  } finally {
    unmount();
  }
});

test('reduced motion: every animation in the line is motion-safe only', () => {
  const src = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');
  const start = src.indexOf('export const LayaRoutingCard');
  const card = src.slice(start, src.indexOf('\n};\n', start) + 3);
  assert.ok(start > 0 && card.length < 12000, 'the slice is the card only');
  const anim = card.match(/[\w:-]*animate-[\w[\]-]+/g) ?? [];
  assert.ok(anim.length >= 3, 'pulse, dots and the 160 ms fade are there');
  for (const a of anim) assert.match(a, /^motion-safe:/, `${a} is static under reduced motion`);
  assert.match(card, /kw-opacity-in_160ms/, 'Change and Why open with a 160 ms opacity fade');
  assert.doesNotMatch(card, /transition-all|scale-105/);
});
