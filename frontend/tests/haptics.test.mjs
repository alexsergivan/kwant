import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

const h = await loadBundled(`export * from './src/haptics';`);
const src = (path) => readFileSync(new URL(`../src/${path}`, import.meta.url), 'utf8');

/** A phone: records every vibrate call, with `this` checked like Chrome does. */
function phone({ stored = null, reduced = false, active = true, visible = true, coarse = true, vibrate } = {}) {
  const calls = [];
  const store = new Map(stored === null ? [] : [[h.HAPTICS_KEY, stored]]);
  const navigator = {
    userActivation: { hasBeenActive: active },
    vibrate:
      vibrate ??
      function (pattern) {
        if (this !== navigator) throw new TypeError('Illegal invocation');
        calls.push(pattern);
        return true;
      },
  };
  return {
    calls,
    store,
    env: {
      navigator,
      matchMedia: (q) => ({
        matches: (reduced && q === '(prefers-reduced-motion: reduce)') || (coarse && q === '(pointer: coarse)'),
      }),
      localStorage: { getItem: (k) => store.get(k) ?? null, setItem: (k, v) => store.set(k, v) },
      document: { visibilityState: visible ? 'visible' : 'hidden' },
    },
  };
}

test('patterns stay short and distinct', () => {
  assert.equal(h.HAPTIC_PATTERNS.tap, 10);
  assert.deepEqual(h.HAPTIC_PATTERNS.success, [15, 40, 15]);
  assert.deepEqual(h.HAPTIC_PATTERNS.warning, [30, 60, 30]);
  const kinds = Object.keys(h.HAPTIC_PATTERNS);
  assert.equal(new Set(kinds.map((k) => JSON.stringify(h.HAPTIC_PATTERNS[k]))).size, kinds.length);
  for (const k of kinds) {
    const total = [h.HAPTIC_PATTERNS[k]].flat().reduce((a, b) => a + b, 0);
    assert.ok(total <= 150, `${k} runs ${total} ms`);
  }
  const p = phone();
  for (const k of kinds) assert.equal(h.haptic(k, p.env), true);
  assert.deepEqual(p.calls, kinds.map((k) => h.HAPTIC_PATTERNS[k]));
});

test('unsupported devices (iOS Safari, desktop) are a silent no-op', () => {
  assert.equal(h.canVibrate({ navigator: {} }), false);
  assert.equal(h.haptic('tap', { navigator: {} }), false);
  assert.equal(h.haptic('tap', {}), false);
  assert.equal(h.haptic('tap', { navigator: { vibrate: 'nope' } }), false);
  // Node has no navigator.vibrate either: the default env must not throw.
  assert.equal(h.haptic('tap'), false);
  assert.equal(h.messageHaptic(), false);
  assert.equal(h.canVibrate(phone().env), true);
});

test('the Settings switch is offered on touch phones only', () => {
  assert.equal(h.hapticsAvailable(phone().env), true);
  assert.equal(h.hapticsAvailable(phone({ coarse: false }).env), false, 'desktop Chrome: vibrate exists, no motor');
  assert.equal(h.hapticsAvailable({ navigator: {}, matchMedia: () => ({ matches: true }) }), false, 'iPhone: no vibrate');
  assert.equal(h.hapticsAvailable(), false);
});

test('a throwing or refusing vibrate never throws', () => {
  const boom = phone({ vibrate: () => { throw new Error('blocked'); } });
  assert.equal(h.haptic('tap', boom.env), false);
  const refused = phone({ vibrate: () => false });
  assert.equal(h.haptic('tap', refused.env), false);
  const badStore = phone();
  badStore.env.localStorage = { getItem: () => { throw new Error('denied'); }, setItem: () => { throw new Error('denied'); } };
  assert.doesNotThrow(() => h.setHapticsEnabled(false, badStore.env));
  assert.equal(h.haptic('tap', badStore.env), true, 'falls back to the default (on)');
});

test('switched off in Settings: nothing buzzes', () => {
  const p = phone();
  h.setHapticsEnabled(false, p.env);
  assert.equal(p.store.get(h.HAPTICS_KEY), 'off');
  assert.equal(h.hapticsEnabled(p.env), false);
  assert.equal(h.haptic('tap', p.env), false);
  assert.deepEqual(p.calls, []);
  h.setHapticsEnabled(true, p.env);
  assert.equal(h.haptic('tap', p.env), true);
});

test('reduced motion defaults off, but an explicit choice wins', () => {
  assert.equal(h.hapticsEnabled(phone({ reduced: true }).env), false);
  assert.equal(h.haptic('tap', phone({ reduced: true }).env), false);
  assert.equal(h.hapticsEnabled(phone({ reduced: true, stored: 'on' }).env), true);
  assert.equal(h.hapticsEnabled(phone({ reduced: false }).env), true);
  assert.equal(h.hapticsEnabled(phone({ reduced: false, stored: 'off' }).env), false);
});

test('no vibrate call before the first tap on the page', () => {
  const p = phone({ active: false });
  assert.equal(h.haptic('tap', p.env), false);
  assert.deepEqual(p.calls, []);
});

test('reply buzz: only on screen, one per burst', () => {
  h.resetMessageHaptic();
  const hidden = phone({ visible: false });
  assert.equal(h.messageHaptic(hidden.env, 1000), false, 'background: the push notification vibrates instead');
  const p = phone();
  assert.equal(h.messageHaptic(p.env, 1000), true);
  assert.equal(h.messageHaptic(p.env, 1000 + h.MESSAGE_GAP_MS - 1), false, 'several bots at once give one buzz');
  assert.equal(h.messageHaptic(p.env, 1000 + h.MESSAGE_GAP_MS), true);
  assert.deepEqual(p.calls, [h.HAPTIC_PATTERNS.message, h.HAPTIC_PATTERNS.message]);
  h.setHapticsEnabled(false, p.env);
  assert.equal(h.messageHaptic(p.env, 99999), false);
});

test('wiring: meaningful actions buzz, typing and scrolling do not', () => {
  const app = src('App.tsx');
  const body = (name) => {
    const start = app.indexOf(`const ${name} = async`);
    assert.ok(start >= 0, name);
    return app.slice(start, app.indexOf('\n  };\n', start));
  };
  for (const name of ['handleSendMessage', 'handleResolveApproval', 'handleAnswerQuestion', 'answerHumanStep']) {
    const b = body(name);
    assert.match(b, /haptic\('tap'\)/, `${name} taps`);
    assert.match(b, /haptic\('warning'\)/, `${name} warns on failure`);
  }
  assert.match(body('answerHumanStep'), /if \(which === 'done'\) haptic\('success'\)/);
  assert.match(app, /if \(solve\) \{\s*setSolveRequest\(\(v\) => v \+ 1\);\s*haptic\('tap'\);/, 'Solve it taps');
  // Replies: on TurnComplete for the open chat and when a discussion ends.
  assert.equal(app.match(/messageHaptic\(\)/g)?.length, 2);
  assert.doesNotMatch(app, /onScroll[^\n]*haptic|onChange[^\n]*haptic|onInput[^\n]*haptic/);

  const overlay = src('components/BrowserControlOverlay.tsx');
  assert.match(overlay, /if \(solved\) haptic\('success'\)/);
  assert.match(overlay, /haptic\('tap'\);\s*onHandBack\(\);/);

  const settings = src('components/SettingsModal.tsx');
  assert.match(settings, /hapticsAvailable\(\) && \(/, 'the switch only shows where the phone can vibrate');
  assert.match(settings, /role="switch"[\s\S]*?aria-checked=\{vibration\}/);
  assert.match(settings, />Vibration</);
});
