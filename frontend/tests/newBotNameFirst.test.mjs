// 36g new-bot-name-first: the New bot form asks for a name and a job; the rest waits under one closed
// "More options". These render the real NewAgentModal in a DOM.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/s1', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event', 'KeyboardEvent', 'MouseEvent', 'SubmitEvent', 'HTMLFormElement', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage', 'matchMedia']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });
g.matchMedia = g.window.matchMedia;
g.window.HTMLElement.prototype.scrollIntoView ??= () => {};
g.window.HTMLMediaElement && (g.window.HTMLMediaElement.prototype.play = () => Promise.resolve());

const json = (body) => new Response(JSON.stringify(body), { status: 200, headers: { 'Content-Type': 'application/json' } });
g.fetch = async (url) => {
  const u = String(url);
  if (u.startsWith('/api/mcp/servers')) return json([]);
  if (u.includes('/mcp-servers')) return json({ server_ids: [] });
  if (u.startsWith('/api/capabilities')) return json({ flight_school: false });
  return json([]);
};
g.window.fetch = g.fetch;

const { React, createRoot, NewAgentModal, avatarForJob, hasCustomMoreOptions, JOB_AVATAR_WORDS, AVATAR_STYLES } = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { NewAgentModal } from './src/components/NewAgentModal';
  export { avatarForJob, hasCustomMoreOptions, JOB_AVATAR_WORDS } from './src/newBotForm';
  export { AVATAR_STYLES } from './src/components/ModernAvatar';
`);

const settle = async () => {
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
};
const act = async (fn) => {
  await fn();
  await settle();
};

async function open(props = {}) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  const sent = [];
  await act(async () => {
    root.render(
      React.createElement(NewAgentModal, {
        isOpen: true,
        onClose: () => {},
        onSubmit: (d) => {
          sent.push(d);
        },
        ...props,
      }),
    );
  });
  const $ = (sel) => host.querySelector(sel);
  const more = () => $('#agent-more-options');
  const toggle = () => $('[data-more-options]');
  // Controls a person can reach without opening More options.
  const visibleControls = () =>
    [...host.querySelectorAll('form input, form textarea, form select, form button')].filter((el) => !el.closest('[hidden]'));
  const type = async (el, v) => {
    await act(async () => {
      const proto = el.tagName === 'TEXTAREA' ? window.HTMLTextAreaElement.prototype : window.HTMLInputElement.prototype;
      Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, v);
      el.dispatchEvent(new window.Event('input', { bubbles: true }));
    });
  };
  const avatarPressed = () => $('[data-avatar-grid] [aria-pressed="true"]')?.getAttribute('title');
  const done = () => {
    root.unmount();
    host.remove();
  };
  return { host, $, more, toggle, visibleControls, type, avatarPressed, sent, done };
}

test('a new bot opens with only Name, "What should it do?" and Create; More options is closed', async () => {
  const v = await open();
  const labels = [...v.host.querySelectorAll('form label')].filter((l) => !l.closest('[hidden]')).map((l) => l.textContent.trim());
  assert.deepEqual(labels, ['Name', 'What should it do?']);
  const controls = v.visibleControls().map((el) => el.id || el.getAttribute('aria-label') || el.textContent.trim());
  assert.deepEqual(controls, ['agent-name', 'Change avatar', 'agent-job', 'More options· persona, avatar, engine, personality and more', 'Cancel', 'Create bot']);
  assert.equal(v.toggle().getAttribute('aria-expanded'), 'false');
  assert.equal(v.more().hidden, true);
  // The rest is there, just closed: persona, title, avatar, personality, Good for / Not for.
  for (const sel of ['#agent-title', '#agent-soul', '[data-avatar-grid]', '#agent-covers', '#agent-avoids']) assert.ok(v.more().querySelector(sel), sel);
  assert.ok(!v.host.textContent.includes('Blank'), 'no "Blank" persona');
  assert.ok(!v.host.textContent.includes('Soul'), 'the field is called Personality');
  assert.ok(v.more().textContent.includes('Personality'));
  v.done();
});

test('Name is focused when the form opens', async () => {
  const v = await open();
  assert.equal(document.activeElement, v.$('#agent-name'));
  v.done();
});

test('More options opens, keeps what was typed when it closes, and the values are saved', async () => {
  const v = await open();
  await act(async () => v.toggle().click());
  assert.equal(v.toggle().getAttribute('aria-expanded'), 'true');
  assert.equal(v.more().hidden, false);
  await v.type(v.$('#agent-title'), 'Support lead');
  await v.type(v.$('#agent-soul'), 'Calm and kind');
  await act(async () => v.toggle().click());
  assert.equal(v.more().hidden, true);
  assert.equal(v.$('#agent-title').value, 'Support lead');
  assert.equal(v.$('#agent-soul').value, 'Calm and kind');
  await v.type(v.$('#agent-name'), 'Helper');
  await v.type(v.$('#agent-job'), 'Answer support emails');
  await act(async () => v.$('form').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true })));
  assert.equal(v.sent.length, 1);
  assert.equal(v.sent[0].title, 'Support lead');
  assert.equal(v.sent[0].soul, 'Calm and kind');
  assert.equal(v.sent[0].persona_prompt, 'Answer support emails');
  v.done();
});

test('the avatar follows the job until the person picks one', async () => {
  const v = await open();
  assert.equal(v.avatarPressed(), 'Droid', 'nothing written yet: the plain droid');
  await v.type(v.$('#agent-job'), 'Answer our support emails');
  assert.equal(v.avatarPressed(), 'Front desk');
  await v.type(v.$('#agent-job'), 'Plan meals and find recipes');
  assert.equal(v.avatarPressed(), 'Chef');
  // Change opens More options on the avatar grid; a pick there stays.
  await act(async () => v.$('button[aria-label="Change avatar"]').click());
  assert.equal(v.more().hidden, false);
  assert.equal(document.activeElement?.getAttribute('title'), 'Chef', 'focus lands on the picked avatar');
  const teacher = v.$('[data-avatar-grid] button[title="Teacher"]');
  await act(async () => teacher.click());
  await v.type(v.$('#agent-job'), 'Review code and find bugs');
  assert.equal(v.avatarPressed(), 'Teacher', 'the pick is kept');
  await v.type(v.$('#agent-name'), 'Ms');
  await act(async () => v.$('form').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true })));
  assert.equal(v.sent[0].avatar_sprite, 'teacher');
  v.done();
});

test('a new bot with no job saves the avatar the job picked: the droid', async () => {
  const v = await open();
  await v.type(v.$('#agent-name'), 'Plain');
  await act(async () => v.$('form').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true })));
  assert.equal(v.sent[0].avatar_sprite, 'default');
  v.done();
});

test('Enter in Name with no job moves to the job instead of creating the bot', async () => {
  const v = await open();
  await v.type(v.$('#agent-name'), 'Helper');
  const ev = new window.KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true });
  await act(async () => v.$('#agent-name').dispatchEvent(ev));
  assert.equal(ev.defaultPrevented, true);
  assert.equal(document.activeElement, v.$('#agent-job'));
  assert.equal(v.sent.length, 0);
  await v.type(v.$('#agent-job'), 'Answer emails');
  const ev2 = new window.KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true });
  await act(async () => v.$('#agent-name').dispatchEvent(ev2));
  assert.equal(ev2.defaultPrevented, false, 'with a job, Enter submits as before');
  v.done();
});

test('a persona is a toggle: picking it pins its avatar, pressing it again starts blank', async () => {
  const v = await open();
  await act(async () => v.toggle().click());
  const first = v.more().querySelector('[role="group"][aria-labelledby="agent-persona-label"] button');
  assert.equal(first.getAttribute('aria-pressed'), 'false');
  await act(async () => first.click());
  assert.equal(first.getAttribute('aria-pressed'), 'true');
  assert.ok(v.$('#agent-name').value, 'the persona fills the name');
  const pinned = v.avatarPressed();
  await v.type(v.$('#agent-job'), 'Plan meals');
  assert.equal(v.avatarPressed(), pinned, 'the persona avatar stays');
  await act(async () => first.click());
  assert.equal(first.getAttribute('aria-pressed'), 'false');
  assert.equal(v.$('#agent-name').value, '');
  assert.equal(v.avatarPressed(), 'Droid');
  v.done();
});

test('editing a bot with a custom model or personality opens with More options expanded', async () => {
  const base = { name: 'Sandy', title: '', persona_prompt: 'p', avatar_sprite: 'writer', preferred_model: 'auto' };
  const plain = await open({ initial: base });
  assert.equal(plain.more().hidden, true, 'nothing custom: closed');
  assert.notEqual(document.activeElement, plain.$('#agent-name'), 'editing does not grab focus for Name');
  plain.done();
  for (const extra of [{ preferred_model: 'claude-sonnet-4-5' }, { soul: 'Dry wit' }, { covers: ['taxes'] }]) {
    const v = await open({ initial: { ...base, ...extra } });
    assert.equal(v.more().hidden, false, JSON.stringify(extra));
    assert.equal(v.toggle().getAttribute('aria-expanded'), 'true');
    v.done();
  }
  const edited = await open({ initial: { ...base, avatar_sprite: 'chef' } });
  await edited.type(edited.$('#agent-job'), 'Answer support emails');
  assert.equal(edited.avatarPressed(), 'Chef', "an existing bot's avatar never follows the job");
  edited.done();
});

test('avatarForJob: earliest matching word wins, then the name, else the droid', () => {
  assert.equal(avatarForJob('answers support emails'), 'desk');
  assert.equal(avatarForJob('write a newsletter about our support'), 'writer');
  assert.equal(avatarForJob('the latest news'), 'default', '"test" only at the start of a word');
  assert.equal(avatarForJob('', 'QuizNight'), 'quizmaster');
  assert.equal(avatarForJob('', ''), 'default');
  const ids = new Set(AVATAR_STYLES.map((s) => s.id));
  for (const [id] of JOB_AVATAR_WORDS) assert.ok(ids.has(id), `${id} is an avatar style`);
});

test('hasCustomMoreOptions: defaults stay closed', () => {
  assert.equal(hasCustomMoreOptions(null), false);
  const base = { name: 'a', title: 'AI Specialist', persona_prompt: '', avatar_sprite: 'x', preferred_model: 'auto', soul: '', covers: [], avoids: [], profile_failover: true, browser_allowed_sites: [], browser_blocked_sites: [], browser_block_private_network: false };
  assert.equal(hasCustomMoreOptions(base), false);
  for (const extra of [{ title: 'Editor' }, { profile_id: 'p1' }, { profile_failover: false }, { avoids: ['x'] }, { browser_blocked_sites: ['a.com'] }, { browser_block_private_network: true }]) {
    assert.equal(hasCustomMoreOptions({ ...base, ...extra }), true, JSON.stringify(extra));
  }
});
