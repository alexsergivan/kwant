// 0.9.6: each bot's AI model on its avatar, live when Laya picks (autonomy/model-badge/brief.md, Pixel).
// The rule (modelBadgeFor), the live store (turn_runs_on → turn_ended → last until the list refreshes),
// and the badge as ModernAvatar draws it: size, tooltip, reduced motion, can't run, and the dev-mode hat.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
let reduceMotion = false;
g.window.matchMedia = (q) => ({ matches: q.includes('reduce') ? reduceMotion : false, media: q, addEventListener() {}, removeEventListener() {} });
g.fetch = g.window.fetch = async () => new Response('{}', { status: 404 });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export * from './src/engines';
  export * from './src/kwantActivity';
  export * from './src/modelBadgeData';
  export { devStore, emptyState } from './src/devMode';
  export { ModernAvatar } from './src/components/ModernAvatar';
`);
const { React, createRoot, flushSync } = m;
const h = React.createElement;
const settle = async (ms = 0) => new Promise((r) => setTimeout(r, ms));

const ready = (id, extra = {}) => ({ id, name: id, kind: 'cli', status: 'ready', detail: null, version: null, models: [], checked_at: null, enabled: true, ready: true, ...extra });
const ENGINES = [
  ready('claude', { models: [{ id: 'sonnet', label: 'Sonnet', default: true }, { id: 'haiku', label: 'Haiku', default: false }] }),
  ready('codex', { models: [{ id: 'gpt-6.1-sol', label: 'GPT-6.1 Sol', default: true }] }),
  ready('grok', { models: [] }),
];
const run = (engine, model, model_label, picked_by) => ({ engine, family: engine, model, model_label, tier: null, picked_by });

// ---- The rule: the brief's table ----

test('pinned, not working: that engine\'s mark, "Runs on …"', () => {
  const b = m.modelBadgeFor({ preferred_model: 'claude/sonnet', last_ran_on: null }, null, ENGINES);
  assert.equal(b.mark, 'claude');
  assert.equal(b.resting, false);
  assert.equal(b.cantRun, null);
  assert.equal(b.label, 'Runs on Claude · Sonnet');
});

test('pinned, working: the same mark; on failover the mark it moved to', () => {
  const k = { preferred_model: 'claude/sonnet' };
  const live = m.modelBadgeFor(k, run('claude', 'sonnet', 'Sonnet', 'pinned'), ENGINES);
  assert.equal(live.mark, 'claude');
  assert.equal(live.label, 'Runs on Claude · Sonnet');
  const moved = m.modelBadgeFor(k, run('codex', 'gpt-6.1-sol', 'GPT-6.1 Sol', 'fallback'), ENGINES);
  assert.equal(moved.mark, 'codex');
  assert.equal(moved.label, 'Moved to Codex · GPT-6.1 Sol for this reply');
});

test('auto, resting: last reply\'s mark at 60% with the ✦ corner; never ran: Laya\'s ✦', () => {
  const rest = m.modelBadgeFor({ preferred_model: 'auto', last_ran_on: run('claude', 'sonnet', 'Sonnet', 'laya') }, null, ENGINES);
  assert.equal(rest.mark, 'claude');
  assert.equal(rest.resting, true);
  assert.equal(rest.auto, true);
  assert.equal(rest.label, 'Laya picks the model. Last reply: Claude · Sonnet');
  const never = m.modelBadgeFor({ preferred_model: 'auto', last_ran_on: null }, null, ENGINES);
  assert.equal(never.mark, 'laya');
  assert.equal(never.resting, false);
  assert.equal(never.label, 'Laya picks the model');
  // An older server without last_ran_on reads as never ran.
  assert.equal(m.modelBadgeFor({ preferred_model: '' }, null, ENGINES).mark, 'laya');
});

test('auto, working: Laya\'s pick for this reply at full opacity', () => {
  const b = m.modelBadgeFor({ preferred_model: 'auto', last_ran_on: run('claude', 'sonnet', 'Sonnet', 'laya') }, run('claude', 'haiku', 'Haiku', 'laya'), ENGINES);
  assert.equal(b.mark, 'claude');
  assert.equal(b.resting, false);
  assert.equal(b.label, 'Laya picked Claude · Haiku for this reply');
  assert.notEqual(b.key, m.modelBadgeFor({ preferred_model: 'auto', last_ran_on: run('claude', 'sonnet', 'Sonnet', 'laya') }, null, ENGINES).key);
});

test('can\'t run: greyed, and the tooltip says signed out, off or not set up', () => {
  const out = ENGINES.map((e) => (e.id === 'claude' ? { ...e, status: 'signed_out', ready: false } : e));
  const b = m.modelBadgeFor({ preferred_model: 'claude/sonnet' }, null, out);
  assert.equal(b.cantRun, 'signed_out');
  assert.equal(b.label, 'Runs on Claude · Sonnet · signed out');
  const off = ENGINES.map((e) => (e.id === 'grok' ? { ...e, enabled: false } : e));
  const o = m.modelBadgeFor({ preferred_model: 'grok' }, null, off);
  assert.equal(o.cantRun, 'off');
  assert.equal(o.label, 'Runs on Grok · off');
  // Not on this computer at all (no CLI, no key): not set up.
  const none = ENGINES.filter((e) => e.id !== 'grok').concat([{ ...ready('grok'), status: 'not_installed', ready: false }, { ...ready('xai_api'), kind: 'api', status: 'not_installed', ready: false }]);
  const ns = m.modelBadgeFor({ preferred_model: 'grok' }, null, none);
  assert.equal(ns.cantRun, 'not_set_up');
  assert.equal(ns.label, 'Runs on Grok · not set up');
  // Still checking, or not loaded, isn't can't-run yet.
  assert.equal(m.modelBadgeFor({ preferred_model: 'claude' }, null, null).cantRun, null);
  assert.equal(m.modelBadgeFor({ preferred_model: 'claude' }, null, out.map((e) => (e.id === 'claude' ? { ...e, status: 'checking' } : e))).cantRun, null);
});

test('every engine has its mark: Ollama, Custom API; the API engines read as their family', () => {
  assert.equal(m.modelBadgeFor({ preferred_model: 'local_ollama' }, null, null).mark, 'ollama');
  assert.equal(m.modelBadgeFor({ preferred_model: 'custom:qwen' }, null, null).mark, 'custom');
  assert.equal(m.modelBadgeFor({ preferred_model: 'auto' }, { engine: 'xai_api', family: null, model: 'grok-4.6', model_label: null, picked_by: 'laya' }, null).mark, 'grok');
  assert.equal(m.modelBadgeFor(null, null, null), null);
});

test('size: hidden under 24 px, 14 px at 28–34 px, larger on big avatars', () => {
  assert.equal(m.modelBadgeSize(20), null);
  assert.equal(m.modelBadgeSize(23), null);
  assert.equal(m.modelBadgeSize(24), 12);
  for (const s of [28, 30, 32, 34]) assert.equal(m.modelBadgeSize(s), 14);
  assert.ok(m.modelBadgeSize(72) > 14 && m.modelBadgeSize(96) >= m.modelBadgeSize(72));
});

// ---- The avatar ----

const host = document.createElement('div');
document.body.append(host);
const root = createRoot(host);
const draw = (props) => flushSync(() => root.render(h(m.ModernAvatar, { name: 'Ann', role: 'designer', kwantId: 'k1', size: 32, ...props })));
const badge = () => host.querySelector('[data-model-badge]');
const marks = () => [...host.querySelectorAll('[data-model-badge] [data-mark]')].map((e) => e.getAttribute('data-mark'));

test('a live change: Claude, then Laya picks Grok, then failover to Codex; resting on Codex after', async () => {
  reduceMotion = false;
  m.setBadgeEngines(ENGINES);
  m.setBadgeBots([{ id: 'k1', preferred_model: 'auto', last_ran_on: run('claude', 'sonnet', 'Sonnet', 'laya') }]);
  draw();
  assert.equal(badge().getAttribute('data-model-badge'), 'claude');
  assert.ok(badge().hasAttribute('data-badge-resting'));
  assert.ok(host.querySelector('[data-badge-spark]'), 'the ✦ corner while resting');
  assert.equal(badge().getAttribute('aria-label'), 'Laya picks the model. Last reply: Claude · Sonnet');
  assert.equal(badge().getAttribute('aria-live'), null, 'not announced on its own');

  flushSync(() => {
    m.turnStarted('k1', 'turn_1');
    m.turnRunsOn('k1', run('grok', 'grok-4.6', null, 'laya'));
  });
  await settle();
  assert.equal(badge().getAttribute('data-model-badge'), 'grok');
  assert.equal(badge().hasAttribute('data-badge-resting'), false);
  assert.equal(host.querySelector('[data-badge-spark]'), null);
  assert.equal(badge().getAttribute('title'), 'Laya picked Grok · grok-4.6 for this reply');
  // Mid-switch: both marks, the old fading out and the new fading in.
  assert.deepEqual(marks(), ['claude', 'grok']);
  assert.ok(host.querySelector('.kw-mb-out [data-mark="claude"]'));
  assert.ok(host.querySelector('.kw-mb-in [data-mark="grok"]'));
  await settle(200);
  assert.deepEqual(marks(), ['grok'], 'the fade is over after 150 ms');

  flushSync(() => m.turnRunsOn('k1', run('codex', 'gpt-6.1-sol', 'GPT-6.1 Sol', 'fallback')));
  await settle();
  assert.equal(badge().getAttribute('data-model-badge'), 'codex');
  assert.equal(badge().getAttribute('title'), 'Moved to Codex · GPT-6.1 Sol for this reply');

  flushSync(() => m.turnEnded('k1', 'turn_1'));
  await settle(200);
  assert.equal(badge().getAttribute('data-model-badge'), 'codex', 'rests on the engine it ended on');
  assert.ok(badge().hasAttribute('data-badge-resting'));
  assert.equal(badge().getAttribute('title'), 'Laya picks the model. Last reply: Codex · GPT-6.1 Sol');

  // The bot list is read again: its last_ran_on takes over.
  flushSync(() => {
    m.setBadgeBots([{ id: 'k1', preferred_model: 'auto', last_ran_on: run('codex', 'gpt-6.1-sol', 'GPT-6.1 Sol', 'fallback') }]);
    m.forgetLastRuns();
  });
  assert.equal(m.lastRunOf('k1'), undefined);
  assert.equal(badge().getAttribute('data-model-badge'), 'codex');
});

test('reduced motion: the mark changes at once, with no fade', async () => {
  reduceMotion = true;
  m.setBadgeBots([{ id: 'k1', preferred_model: 'auto', last_ran_on: run('claude', 'sonnet', 'Sonnet', 'laya') }]);
  draw();
  flushSync(() => {
    m.turnStarted('k1', 'turn_2');
    m.turnRunsOn('k1', run('grok', 'grok-4.6', null, 'laya'));
  });
  await settle();
  assert.deepEqual(marks(), ['grok']);
  assert.equal(host.querySelector('.kw-mb-in, .kw-mb-out'), null);
  const css = readFileSync(new URL('../src/styles/modern.css', import.meta.url), 'utf8');
  assert.match(css, /@media \(prefers-reduced-motion: reduce\) \{\s*\.kw-mb-in \{ animation: none; \}\s*\.kw-mb-out \{ display: none; \}/);
  flushSync(() => m.turnEnded('k1', 'turn_2'));
  reduceMotion = false;
});

test('tooltip: the badge and the bot list\'s name say the same', () => {
  m.setBadgeBots([{ id: 'k1', preferred_model: 'claude/sonnet', last_ran_on: null }]);
  draw();
  assert.equal(badge().getAttribute('title'), 'Runs on Claude · Sonnet');
  assert.equal(badge().getAttribute('role'), 'img');
  assert.equal(badge().getAttribute('aria-label'), badge().getAttribute('title'));
  const sidebar = readFileSync(new URL('../src/components/Sidebar.tsx', import.meta.url), 'utf8');
  assert.match(sidebar, /title=\{`\$\{k\.name\} · \$\{badge\?\.label \?\? engineTip\(k\.preferred_model\)\}`\}/);
});

test('hidden under 24 px, and on reply avatars in the chat', () => {
  m.setBadgeBots([{ id: 'k1', preferred_model: 'claude', last_ran_on: null }]);
  draw({ size: 20 });
  assert.equal(badge(), null);
  draw({ size: 23 });
  assert.equal(badge(), null);
  draw({ size: 24 });
  assert.ok(badge());
  draw({ size: 32, modelBadge: false });
  assert.equal(badge(), null);
  draw({ size: 32, kwantId: undefined });
  assert.equal(badge(), null, 'no bot, no badge');
  const chat = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');
  assert.match(chat, /size=\{20\} kwantId=\{replyKwantId\} modelBadge=\{false\}/);
});

test('dev mode: the hard hat (top-left) and the badge (bottom-left) never overlap', () => {
  m.devStore._reset();
  m.devStore.set('spc', { ...m.emptyState(), on: true });
  m.devStore.setCurrent('spc');
  m.setBadgeBots([{ id: 'k1', preferred_model: 'claude', last_ran_on: null }]);
  const css = readFileSync(new URL('../src/styles/modern.css', import.meta.url), 'utf8');
  const box = (sel) => {
    const rule = css.match(new RegExp(`\\${sel} \\{([^}]*)\\}`))[1];
    const n = (p) => { const v = rule.match(new RegExp(`(?:^|[\\s;])${p}:\\s*(-?\\d+)px`)); return v ? Number(v[1]) : null; };
    return { left: n('left'), top: n('top'), bottom: n('bottom'), w: n('width') };
  };
  const hat = box('.kw-dev-hat');
  const mb = box('.kw-model-badge');
  for (const size of [28, 30, 32, 34, 48, 72, 96]) {
    draw({ size });
    assert.ok(host.querySelector('[data-dev-hat]'), `hat at ${size}`);
    assert.ok(badge(), `badge at ${size}`);
    const d = m.modelBadgeSize(size);
    assert.equal(badge().style.width, `${d}px`);
    const hatBottom = hat.top + hat.w;
    const badgeTop = size - mb.bottom - d;
    assert.ok(badgeTop > hatBottom, `at ${size}px the hat ends at ${hatBottom} and the badge starts at ${badgeTop}`);
  }
  m.devStore._reset();
});

test('can\'t run: the avatar\'s badge is greyed out', () => {
  m.setBadgeEngines(ENGINES.map((e) => (e.id === 'claude' ? { ...e, status: 'signed_out', ready: false } : e)));
  m.setBadgeBots([{ id: 'k1', preferred_model: 'claude/sonnet', last_ran_on: null }]);
  draw();
  assert.equal(badge().getAttribute('data-badge-cant-run'), 'signed_out');
  assert.ok(host.querySelector('[data-model-badge] .text-zinc-500'));
  assert.equal(badge().getAttribute('title'), 'Runs on Claude · Sonnet · signed out');
  flushSync(() => m.setBadgeEngines(ENGINES));
  assert.equal(badge().hasAttribute('data-badge-cant-run'), false);
});

test('turn_runs_on for a turn whose start went unseen still lets go when it ends', () => {
  m.turnRunsOn('k9', run('claude', 'haiku', 'Haiku', 'laya'));
  m.turnEnded('k9', 'turn_x');
  assert.equal(m.liveRunOf('k9'), undefined);
  assert.equal(m.lastRunOf('k9').model, 'haiku');
});
