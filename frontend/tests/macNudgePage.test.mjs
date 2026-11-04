import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

// The Mac app's "needs you" pill page (apps/mac/loading/nudge.html), run against a tiny DOM.
const html = await readFile(new URL('../../apps/mac/loading/nudge.html', import.meta.url), 'utf8');
const script = html.match(/<script>([\s\S]*?)<\/script>/)[1];

const fakeElement = () => {
  const attrs = new Map();
  const classes = new Set();
  return {
    attrs,
    className: '',
    dataset: {},
    innerHTML: '',
    offsetWidth: 0,
    classList: { add: (c) => classes.add(c), remove: (c) => classes.delete(c), contains: (c) => classes.has(c) },
    setAttribute: (k, v) => attrs.set(k, String(v)),
    getAttribute: (k) => attrs.get(k) ?? null,
    addEventListener() {},
    appendChild() {},
    remove() {},
  };
};

const loadPage = () => {
  const made = [];
  const slot = fakeElement();
  const document = {
    documentElement: fakeElement(),
    getElementById: () => slot,
    createElement: () => {
      const el = fakeElement();
      made.push(el);
      return el;
    },
  };
  const window = {};
  const context = {
    window,
    document,
    location: { hash: '', href: '' },
    matchMedia: () => ({ matches: false }),
    setTimeout: () => 0,
    clearTimeout() {},
  };
  vm.createContext(context);
  vm.runInContext(script, context);
  return { nudge: window.kwantNudge, pill: () => made[made.length - 1] };
};

test('the aria-label reads a bot name with & < > " as it is', () => {
  const { nudge, pill } = loadPage();
  nudge.show([{ kind: 'approval', name: 'R&D', title: 'Use "Bash" for <b>x</b> & y' }]);
  assert.equal(pill().getAttribute('aria-label'), 'R&D needs your OK. Use "Bash" for <b>x</b> & y');
});

test('the same text is escaped where it goes into the pill’s HTML', () => {
  const { nudge, pill } = loadPage();
  nudge.show([{ kind: 'question', name: '<img src=x onerror=alert(1)>', title: 'a & "b"' }]);
  const inner = pill().innerHTML;
  assert.ok(!inner.includes('<img'), inner);
  assert.match(inner, /&lt;img src=x onerror=alert\(1\)&gt; asked you something/);
  assert.match(inner, /a &amp; &quot;b&quot;/);
  // the avatar letter is escaped too
  assert.match(inner, /kw-nudge__av">&lt;</);
});

test('a coalesced pill’s label lists the real names', () => {
  const { nudge, pill } = loadPage();
  nudge.show([
    { kind: 'approval', name: 'R&D', title: 'one' },
    { kind: 'question', name: 'Q<A>', title: 'two' },
  ]);
  assert.equal(pill().getAttribute('aria-label'), '2 things need you. R&D and Q<A>');
  assert.match(pill().innerHTML, /R&amp;D and Q&lt;A&gt;/);
});
