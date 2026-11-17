import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const h = await loadBundled(`export * from './src/humanSteps';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { HumanStepCard } from './src/components/HumanStepCard';
  export const render = (props) => renderToStaticMarkup(React.createElement(HumanStepCard, props));
`);

const meta = (over = {}) => ({
  kind: 'human_step', human_step_id: 'hs1', step_kind: 'sign_in', reason: 'Log in to GitHub so I can open the PR.',
  url: 'https://www.github.com/login?return_to=/secret/path', sandbox_target: 'vm', source: 'bot', status: 'open', ...over,
});
const rec = (over = {}) => ({
  id: 'hs1', kwant_id: 'pixel', channel_id: null, direct_kwant_id: 'pixel', thread_id: null, turn_id: 't1', kind: 'sign_in',
  reason: 'r', url: null, sandbox_target: null, source: 'bot', status: 'open', message_id: 'm1', created_at: '2026-10-22T10:00:00Z', closed_at: null, ...over,
});
const props = (over = {}) => ({ meta: h.parseHumanStepMeta(meta()), messageId: 'm1', kwantName: 'Pixel', waiting: true, onOpenBrowser: () => {}, onDone: async () => {}, onSkip: async () => {}, ...over });

test('meta parses, defaults odd values, ignores other kinds', () => {
  assert.equal(h.parseHumanStepMeta(JSON.stringify(meta())).step_kind, 'sign_in');
  assert.equal(h.parseHumanStepMeta(meta({ step_kind: 'weird', status: 'x' })).step_kind, 'other');
  assert.equal(h.parseHumanStepMeta(meta({ status: 'x' })).status, 'open');
  assert.equal(h.parseHumanStepMeta({ kind: 'ask_user' }), null);
  assert.equal(h.parseHumanStepMeta('nope'), null);
});

test('only the domain of the address is shown', () => {
  assert.equal(h.domainOf('https://www.github.com/login?x=1'), 'github.com');
  assert.equal(h.domainOf('not a url'), null);
  assert.equal(h.domainOf(null), null);
});

test('records: newest state, closed never reopens; badges and waits', () => {
  let map = h.applyHumanStep({}, rec());
  map = h.applyHumanStep(map, rec({ id: 'hs2', kwant_id: 'pixel', turn_id: 't2', created_at: '2026-10-22T11:00:00Z' }));
  assert.deepEqual(h.openStepsByKwant(map), { pixel: 2 });
  assert.equal(h.latestOpenStep(map, 'pixel').id, 'hs2');
  assert.deepEqual([...h.turnsAwaitingStep(map)].sort(), ['t1', 't2']);
  map = h.applyHumanStep(map, rec({ status: 'done' }));
  assert.equal(h.applyHumanStep(map, rec()).hs1.status, 'done');
  assert.deepEqual(h.openStepsByKwant(map), { pixel: 1 });
  assert.equal(h.latestOpenStep(map, 'other'), null);
  assert.equal(h.humanStepView(h.parseHumanStepMeta(meta()), rec({ status: 'declined' })).status, 'declined');
});

test('the Done label fits the kind', () => {
  assert.equal(h.KIND_TEXT.sign_in.done, 'Done, I’m signed in');
  assert.equal(h.KIND_TEXT.captcha.done, 'Done, I solved it');
  for (const [kind, t] of Object.entries(h.KIND_TEXT)) {
    const html = ui.render(props({ meta: h.parseHumanStepMeta(meta({ step_kind: kind })) }));
    assert.ok(html.includes(t.heading), kind);
    assert.ok(html.includes(t.done.replace('’', '&#x27;')) || html.includes(t.done), kind);
  }
});

test('open card: reason, domain only, three buttons, waiting line', () => {
  const html = ui.render(props());
  assert.match(html, /Pixel<\/span> needs you in the browser/);
  assert.match(html, /Log in to GitHub/);
  assert.match(html, />github\.com</);
  assert.doesNotMatch(html, /secret\/path|return_to/);
  assert.match(html, /Open browser/);
  assert.match(html, />Skip</);
  assert.match(html, /is paused and waiting for you/);
  assert.equal((html.match(/<button/g) || []).length, 3);
});

test('closed and stale cards', () => {
  assert.doesNotMatch(ui.render(props({ live: rec({ status: 'done' }) })), /<button/);
  assert.match(ui.render(props({ live: rec({ status: 'done' }) })), /Done\. Pixel is carrying on/);
  assert.match(ui.render(props({ live: rec({ status: 'declined' }) })), /Skipped/);
  assert.match(ui.render(props({ live: rec({ status: 'cancelled' }) })), /reply was stopped/);
  // 22m: a file sent while the bot waited closes the card, and it never reads "done"
  const left = ui.render(props({ live: rec({ status: 'left_for_message' }) }));
  assert.doesNotMatch(left, /<button/);
  assert.match(left, /Closed: Pixel is reading the file you sent\./);
  assert.doesNotMatch(left, /Done|carrying on|waiting for you/);
  assert.equal(h.parseHumanStepMeta(meta({ status: 'left_for_message' })).status, 'left_for_message');
  assert.match(ui.render(props({ waiting: false })), /stopped waiting/);
  assert.match(ui.render(props({ meta: h.parseHumanStepMeta(meta({ source: 'auto' })) })), /spotted automatically/);
});

test('a late Done or Skip on a closed step reads as a sentence, never the raw status (22m round 2)', () => {
  const closed = ['done', 'declined', 'cancelled', 'resolved_auto', 'timed_out', 'left_for_message'];
  for (const status of closed) {
    for (const which of ['done', 'decline']) {
      const text = h.closedStepText(status, which, 'Pixel');
      assert.notEqual(text, undefined, `${status}/${which}: a closed step is mapped, so the raw 409 text never shows`);
      if (text === null) continue;
      assert.doesNotMatch(text, /left_for_message|resolved_auto|timed_out|already (open|declined|cancelled)\b/, `${status}/${which}: ${text}`);
      assert.doesNotMatch(text, /_/, `${status}/${which}: ${text}`);
      assert.match(text, /^[A-Z].*\.$/, `${status}/${which}: a sentence`);
    }
  }
  // The file closed the card: Done says it didn't land and what to do; Skip says nothing
  assert.equal(
    h.closedStepText('left_for_message', 'done', 'Pixel'),
    "This closed when you sent a file, so Pixel didn't get your Done. If you finished, say so in the chat.",
  );
  assert.equal(h.closedStepText('left_for_message', 'decline', 'Pixel'), null);
  // Other closed statuses show nothing: the card's footer already says how it ended (22v)
  assert.equal(h.closedStepText('done', 'done', 'Pixel'), null);
  assert.equal(h.closedStepText('declined', 'decline', 'Pixel'), null);
  assert.equal(h.closedStepText('cancelled', 'done', 'Pixel'), null);
  // An open step isn't closed: the caller keeps the real error
  assert.equal(h.closedStepText('open', 'done', 'Pixel'), undefined);
});

test('meta line is one inline text run that wraps like a sentence', () => {
  const html = ui.render(props({ meta: h.parseHumanStepMeta(meta({ source: 'auto' })) }));
  const m = html.match(/<p data-hs-meta=""([^>]*)>([^]*?)<\/p>/);
  assert.ok(m, 'meta paragraph');
  // Not a flex row of columns
  assert.doesNotMatch(m[1], /\bflex\b|inline-flex/);
  // Icon inline, then name, then the rest as plain text in the same run
  assert.match(m[2], /^<svg[^>]*class="[^"]*inline-block[^"]*"[^]*?<\/svg><span class="text-zinc-200">Pixel<\/span> needs you in the browser · spotted automatically$/);
  const bot = ui.render(props()).match(/<p data-hs-meta=""[^>]*>([^]*?)<\/p>/)[1];
  assert.match(bot, /<span class="text-zinc-200">Pixel<\/span> needs you in the browser$/);
});

test('a closed card stops asking: quiet past-tense eyebrow in zinc, never "needs you" (22p)', () => {
  const eyebrow = (html) => html.match(/<p data-hs-meta="" class="([^"]*)">([^]*?)<\/p>/);
  // Open: unchanged, amber hand, "needs you"
  const open = eyebrow(ui.render(props({ meta: h.parseHumanStepMeta(meta({ source: 'auto' })) })));
  assert.match(open[2], /^<svg[^>]*lucide-hand[^>]*text-amber-300[^]*?<\/svg><span class="text-zinc-200">Pixel<\/span> needs you in the browser · spotted automatically$/);
  // Every closed status, from the live record and from the saved message; left_for_message comes with #721
  for (const status of ['done', 'declined', 'timed_out', 'resolved_auto', 'cancelled', 'left_for_message']) {
    for (const [how, p] of [
      ['live', props({ live: rec({ status }) })],
      ['live, auto', props({ meta: h.parseHumanStepMeta(meta({ source: 'auto' })), live: rec({ status }) })],
      ...(status === 'left_for_message' ? [] : [['meta', props({ meta: h.parseHumanStepMeta(meta({ status })) })]]),
    ]) {
      const html = ui.render(p);
      const m = eyebrow(html);
      assert.ok(m, `${status} ${how}: eyebrow`);
      assert.doesNotMatch(html, /needs you/i, `${status} ${how}: never "needs you"`);
      assert.doesNotMatch(m[0], /amber/, `${status} ${how}: no amber in the eyebrow`);
      assert.match(m[1], /text-zinc-500/);
      const auto = how.endsWith('auto') ? ' · spotted automatically' : '';
      assert.match(m[2], new RegExp(`^<svg[^>]*lucide-hand[^>]*text-zinc-500[^]*?</svg><span class="text-zinc-400">Pixel</span> asked for you in the browser${auto}$`), `${status} ${how}`);
      assert.match(html, /border-zinc-800/, `${status} ${how}: quiet border`);
    }
  }
});

test('captcha card after the turn ended: one line, carries on by itself, not "stopped waiting"', () => {
  const html = ui.render(props({ waiting: false, meta: h.parseHumanStepMeta(meta({ step_kind: 'captcha' })) }));
  assert.match(html, /role="status" aria-live="polite"[^>]*><span[^>]*><svg[^]*?<\/svg> Pixel carries on by itself once it’s solved\. Nothing you do there is passed to Pixel\.<\/span><\/p>/);
  // The near-duplicate footer is gone: the privacy note is said once, and nothing else sits under the line
  assert.equal((html.match(/Nothing you do there is passed/g) || []).length, 1);
  assert.doesNotMatch(html, /turn ended|Solve the puzzle and|continues by itself/);
  assert.doesNotMatch(html, /stopped waiting|instantly/);
  // Other kinds keep the pick-it-up-later copy
  assert.match(ui.render(props({ waiting: false, meta: h.parseHumanStepMeta(meta({ step_kind: 'sign_in' })) })), /stopped waiting\. Press Done any time/);
});

test('card icons never shrink when a line wraps on a phone (shrink-0 on every icon)', () => {
  const cap = (over = {}) => h.parseHumanStepMeta(meta({ step_kind: 'captcha', source: 'auto', ...over }));
  const cards = {
    'captcha, turn ended': props({ waiting: false, meta: cap() }),
    'captcha, paused': props({ meta: cap() }),
    'captcha, solved': props({ meta: cap({ status: 'resolved_auto' }) }),
    'captcha, timed out': props({ meta: cap({ status: 'timed_out' }) }),
    'sign-in, stopped waiting': props({ waiting: false }),
    'sign-in, paused': props(),
    'done': props({ live: rec({ status: 'done' }) }),
    'skipped': props({ live: rec({ status: 'declined' }) }),
    'cancelled': props({ live: rec({ status: 'cancelled' }) }),
    'left for a file': props({ live: rec({ status: 'left_for_message' }) }),
  };
  const seen = new Set();
  for (const [name, p] of Object.entries(cards)) {
    const html = ui.render(p);
    const icons = [...html.matchAll(/<svg[^>]*class="([^"]*\blucide\b[^"]*)"/g)].map((m) => m[1]);
    assert.ok(icons.length > 0, `${name}: has icons`);
    for (const cls of icons) {
      assert.match(cls, /(^|\s)shrink-0(\s|$)/, `${name}: ${cls}`);
      seen.add(cls.match(/lucide-[a-z0-9-]+/g).pop());
    }
    // The status line icon is a flex child of a wrapping line: the one Rigel saw shrink to a dot
    assert.match(html, /role="status"[^>]*><span class="inline-flex[^"]*"><svg[^>]*class="[^"]*\bshrink-0\b/, `${name}: status icon`);
  }
  for (const icon of ['lucide-clock', 'lucide-hand', 'lucide-globe', 'lucide-puzzle', 'lucide-monitor-play', 'lucide-circle-check', 'lucide-circle-minus']) {
    assert.ok(seen.has(icon), `covered ${icon} (saw ${[...seen].join(', ')})`);
  }
});

test('captcha target: finite box only, kind defaults to puzzle', () => {
  assert.deepEqual(h.parseCaptchaTarget({ x: 460, y: 180, width: 360, height: 380 }), { kind: 'puzzle', x: 460, y: 180, width: 360, height: 380 });
  assert.equal(h.parseCaptchaTarget({ x: 1, y: 1, width: 0, height: 5 }), null);
  assert.equal(h.parseCaptchaTarget({ x: 'a', y: 1, width: 5, height: 5 }), null);
  assert.equal(h.parseCaptchaTarget(null), null);
  const m = h.parseHumanStepMeta(meta({ step_kind: 'captcha', target: { kind: 'slider', x: 1, y: 2, width: 3, height: 4 } }));
  assert.equal(m.target.kind, 'slider');
  assert.equal(h.parseHumanStepMeta(meta({ status: 'resolved_auto' })).status, 'resolved_auto');
  assert.equal(h.parseHumanStepMeta(meta({ status: 'timed_out' })).status, 'timed_out');
});

test('captcha step for a sandbox: open first, a just-closed one lingers a minute', () => {
  const now = Date.parse('2026-10-22T12:00:00Z');
  const box = { kind: 'puzzle', x: 1, y: 2, width: 30, height: 40 };
  let map = h.applyHumanStep({}, rec({ id: 'c1', kind: 'captcha', sandbox_target: 'pixel', target: box }));
  map = h.applyHumanStep(map, rec({ id: 's1', kind: 'sign_in', sandbox_target: 'pixel', created_at: '2026-10-22T11:59:00Z' }));
  assert.equal(h.captchaStepFor(map, 'pixel', now).id, 'c1');
  assert.equal(h.captchaStepFor(map, 'other', now), null);
  // The box moves while the step stays open
  map = h.applyHumanStep(map, rec({ id: 'c1', kind: 'captcha', sandbox_target: 'pixel', target: { ...box, x: 9 } }));
  assert.equal(map.c1.target.x, 9);
  map = h.applyHumanStep(map, rec({ id: 'c1', kind: 'captcha', sandbox_target: 'pixel', status: 'resolved_auto', closed_at: '2026-10-22T11:59:40Z' }));
  assert.equal(h.captchaStepFor(map, 'pixel', now).status, 'resolved_auto');
  assert.equal(h.captchaStepFor(map, 'pixel', now + 60_000), null);
});

test('captcha card: Solve it is the main button and opens solve mode; auto outcomes', () => {
  const calls = [];
  const cm = h.parseHumanStepMeta(meta({ step_kind: 'captcha' }));
  const html = ui.render(props({ meta: cm, onOpenBrowser: (...a) => calls.push(a) }));
  assert.match(html, /Solve it/);
  assert.doesNotMatch(html, /Open browser/);
  assert.match(html, /continues by itself/);
  assert.equal((html.match(/<button/g) || []).length, 3);
  assert.match(ui.render(props({ meta: cm, live: rec({ kind: 'captcha', status: 'resolved_auto' }) })), /Solved\. Pixel continues\./);
  assert.match(ui.render(props({ meta: cm, live: rec({ kind: 'captcha', status: 'timed_out' }) })), /moved on without it/);
  assert.doesNotMatch(ui.render(props({ meta: cm, live: rec({ kind: 'captcha', status: 'timed_out' }) })), /<button/);
});

test('a closed card names the step, quietly: noun heading in zinc-300, the open card keeps its ask (22v)', () => {
  const heading = (html) => html.match(/<h3 id="hs-m1" class="([^"]*)">([^<]*)<\/h3>/);
  const nouns = { sign_in: 'Sign-in', captcha: 'Human check', two_factor: 'Sign-in code', other: 'Browser step' };
  const asks = { sign_in: 'Sign in needed', captcha: 'Human check needed', two_factor: 'Sign-in code needed', other: 'Your help is needed' };
  for (const kind of Object.keys(nouns)) {
    // Open: unchanged
    const open = heading(ui.render(props({ meta: h.parseHumanStepMeta(meta({ step_kind: kind })) })));
    assert.ok(open, `${kind} open: heading`);
    assert.equal(open[2], asks[kind]);
    assert.match(open[1], /text-zinc-100/);
    for (const status of ['done', 'declined', 'timed_out', 'resolved_auto', 'cancelled', 'left_for_message']) {
      for (const [how, p] of [
        ['live', props({ meta: h.parseHumanStepMeta(meta({ step_kind: kind })), live: rec({ kind, status }) })],
        ...(status === 'left_for_message' ? [] : [['meta', props({ meta: h.parseHumanStepMeta(meta({ step_kind: kind, status })) })]]),
      ]) {
        const html = ui.render(p);
        const m = heading(html);
        assert.ok(m, `${kind} ${status} ${how}: heading`);
        assert.equal(m[2], nouns[kind], `${kind} ${status} ${how}: noun form`);
        assert.doesNotMatch(m[1], /zinc-100/, `${kind} ${status} ${how}: not the loud white`);
        assert.match(m[1], /text-zinc-300/, `${kind} ${status} ${how}: zinc-300`);
        assert.doesNotMatch(html, /needed/i, `${kind} ${status} ${how}: nothing on a closed card is "needed"`);
        // Screen readers announce the noun through aria-labelledby
        assert.match(html, /aria-labelledby="hs-m1"/);
      }
    }
  }
});

test('a late Done or Skip on a closed step: nothing where the footer says it, words otherwise, never the raw status (22v)', () => {
  const raw = /timed_out|resolved_auto|left_for_message|declined|cancelled|_/;
  for (const status of ['done', 'declined', 'cancelled', 'timed_out', 'resolved_auto']) {
    for (const which of ['done', 'decline']) assert.equal(h.closedStepText(status, which, 'Pixel'), null, `${status} ${which}: the footer says it`);
  }
  for (const status of ['done', 'declined', 'cancelled', 'timed_out', 'resolved_auto', 'left_for_message', 'paused_for_x']) {
    for (const which of ['done', 'decline']) {
      const t = h.closedStepText(status, which, 'Pixel');
      assert.notEqual(t, undefined, `${status} ${which}: a closed step has an answer`);
      if (t !== null) {
        assert.doesNotMatch(t, raw, `${status} ${which}: "${t}"`);
        assert.match(t, /^[A-Z].*\.$/, `${status} ${which}: a sentence`);
      }
    }
  }
  // left_for_message + Done keeps #721's words; Skip lost nothing
  assert.match(h.closedStepText('left_for_message', 'done', 'Pixel'), /^This closed when you sent a file, so Pixel didn't get your Done\./);
  assert.equal(h.closedStepText('left_for_message', 'decline', 'Pixel'), null);
  // Open isn't closed, so the real error shows; an unknown status still reads as a sentence
  assert.equal(h.closedStepText('open', 'done', 'Pixel'), undefined);
  assert.equal(h.closedStepText('paused_for_x', 'done', 'Pixel'), 'This step is already closed.');
});

test('a closed card always says what is next: timed_out tells you to say so; an unknown closed status never says "Press Done" (23d)', () => {
  // timed_out: what to do if you did finish, the same words as left_for_message's late Done
  for (const kind of ['sign_in', 'captcha', 'two_factor', 'other']) {
    const html = ui.render(props({ meta: h.parseHumanStepMeta(meta({ step_kind: kind })), live: rec({ kind, status: 'timed_out' }), waiting: false }));
    assert.match(html, /Nobody solved it in time\. Pixel moved on without it\. If you finished, say so in the chat\./, kind);
    assert.doesNotMatch(html, /<button/, kind);
  }
  // A closed status the card has no line for: the neutral line, no buttons, never the open card's lines, waiting or not
  for (const kind of ['sign_in', 'captcha', 'two_factor', 'other']) {
    for (const waiting of [false, true]) {
      const html = ui.render(props({ meta: h.parseHumanStepMeta(meta({ step_kind: kind })), live: rec({ kind, status: 'left_for_message' }), waiting }));
      assert.match(html, /<span class="inline-flex items-center gap-1\.5 text-zinc-400"><svg[^>]*lucide-circle-minus[^]*?<\/svg> This step is closed\.<\/span>/, `${kind} waiting=${waiting}`);
      assert.doesNotMatch(html, /Press Done|stopped waiting|paused and waiting|carries on by itself|<button/, `${kind} waiting=${waiting}`);
    }
  }
  // The open card after the turn ended keeps its pick-it-up-later line and its buttons
  const open = ui.render(props({ waiting: false, meta: h.parseHumanStepMeta(meta({ step_kind: 'sign_in' })) }));
  assert.match(open, /stopped waiting\. Press Done any time and it picks up in a new reply\./);
  assert.match(open, /<button/);
  assert.doesNotMatch(open, /This step is closed/);
});
