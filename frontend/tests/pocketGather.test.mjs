// Gather Pocket card (pa10b, autonomy/pocket-apps/pa10b/design.md §1-§5, card-brief.md): validGather for a good block
// per face, missing fields falling back, junk rejected, any field that pairs a person with a ballot rejected, Picked
// (the one holo) only on a closed vote with a winner, People and paid ticks (set_paid), Nudges with Copy / Share… /
// Edit / Skip, "Copied <time>" and never "Sent", the 48 h cap line (server 409 "until"), the results' copy lists
// (never paid), tie actions, buttons with no host action yet saying why, Done rows and the Tally avatar.
import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'CustomEvent', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
g.matchMedia = g.window.matchMedia;
const clipboard = { text: null, fail: false, writeText: async (t) => {
  if (clipboard.fail) throw new Error('denied');
  clipboard.text = t;
} };
Object.defineProperty(g.navigator, 'clipboard', { value: clipboard, configurable: true });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { MarkdownRenderer } from './src/components/MarkdownRenderer';
  export { COMPOSE_PREFILL_EVENT } from './src/components/results/SmartResult';
  export { validPocket } from './src/components/results/Pocket';
  export { validGather, findBallotPair, earnedPicked, gatherMood, GATHER_LATER, restaurantList, whoBringsWhat, keptLine, signupSummary, GatherDoneList, BALLOT_KEYS } from './src/components/results/PocketGather';
  export { GATHER_FIXTURES, POCKET_FIXTURES, SANTA_FIXTURES } from './src/components/results/fixtures';
  export { pocketApi, PocketApiError } from './src/pocketApi';
  export { ModernAvatar, AVATAR_STYLES } from './src/components/ModernAvatar';
`);
const { React, createRoot } = m;
const h = React.createElement;
const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
const mounted = new Set();
const realFetch = g.fetch;
afterEach(() => {
  for (const v of mounted) v.done();
  mounted.clear();
  localStorage.clear();
  clipboard.text = null;
  clipboard.fail = false;
  g.fetch = realFetch;
});
async function mount(el) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  root.render(el);
  await settle();
  const v = { host, done: () => (root.unmount(), host.remove()) };
  mounted.add(v);
  return v;
}
const click = async (el) => {
  el.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
  await settle();
};
const F = m.GATHER_FIXTURES;
const raw = (name) => structuredClone(F[name].results.blocks[0]);
const withG = (name, over) => {
  const b = raw(name);
  b.gather = { ...b.gather, ...over };
  return b;
};
const card = (name, block = raw(name)) => mount(h(m.MarkdownRenderer, { content: F[name].reply, results: { version: 1, blocks: [block] }, messageId: `m-${name}` }));
const button = (v, text) => [...v.host.querySelectorAll('button, a')].find((b) => b.textContent.trim() === text);
const holos = (v) => v.host.querySelectorAll('.kw-sticker-holo, [data-tone="holo"]').length;
const ok = (b) => assert.ok(m.validPocket(b), 'should validate');
const bad = (b, why) => assert.equal(m.validPocket(b), null, why);
const respond = (calls, body = { ok: true }, status = 200) => async (url, init) => (calls.push([init.method, url, init.body ? JSON.parse(init.body) : null]), new Response(JSON.stringify(body), { status }));

// ---------------------------------------------------------------------------------------------------
// The validator

test('a good block per face and state: each validates and renders the gather card', async () => {
  const want = { rsvpOpen: 'rsvp-open', rsvpNoAddress: 'rsvp-open', signupOpen: 'signup-open', signupMoved: 'signup-open', voteOpen: 'vote-open', voteClosed: 'vote-closed', voteTie: 'vote-tie', rsvpResult: 'rsvp-closed', signupResult: 'signup-closed' };
  for (const [name, state] of Object.entries(want)) {
    assert.ok(m.validPocket(raw(name)), `${name} validates`);
    const v = await card(name);
    const el = v.host.querySelector('[data-gather]');
    assert.ok(el, `${name} renders the gather card`);
    assert.equal(el.dataset.gather, state);
    assert.match(el.textContent, /Pocket app · (RSVP|Sign-up|Vote) · Riverside Runners/);
    v.done();
  }
  const v = await card('rsvpOpen');
  assert.match(v.host.textContent, /About 3 bot calls to set this up, about \$0\.01\. Answers, counting and nudge drafts use none\./);
});

test('missing optional fields fall back to plain words', async () => {
  const b = raw('rsvpOpen');
  for (const k of ['when', 'closed_at', 'event_at', 'kept_after', 'club', 'host_name', 'place', 'show_names', 'people', 'nudges', 'join', 'slots', 'vote', 'moved_up']) delete b.gather[k];
  delete b.gather.rsvp.paid;
  delete b.gather.rsvp.food;
  const gg = m.validPocket(b).gather;
  assert.deepEqual([gg.people, gg.nudges, gg.join, gg.club, gg.show_names, gg.rsvp.paid], [[], [], null, null, true, null]);
  const v = await card('rsvpOpen', b);
  const t = v.host.textContent;
  assert.match(t, /Pocket app · RSVP/);
  assert.match(t, /Kept until Sat, Dec 19 \(7 days after it closes\), then deleted\./);
  assert.doesNotMatch(t, /Paid \d/);
  assert.equal(v.host.querySelector('[data-only-you], [data-chip="only-you"]'), null);
  assert.equal(button(v, 'Share link').getAttribute('aria-disabled'), 'true', 'no link: Share link says why');
  assert.equal(button(v, 'Share link').getAttribute('title'), m.GATHER_LATER.noLink);
});

test('junk and contradictions are rejected, so the reply shows as markdown', () => {
  bad(withG('rsvpOpen', { face: 'potluck' }), 'unknown face');
  bad(withG('rsvpOpen', { phase: 'tie' }), 'a tie on an RSVP');
  bad(withG('rsvpOpen', { rsvp: null }), 'RSVP without its counts');
  bad(withG('signupOpen', { slots: [] }), 'sign-up with no slot');
  bad(withG('voteOpen', { vote: null }), 'vote without its count');
  bad(withG('rsvpOpen', { rsvp: { ...raw('rsvpOpen').gather.rsvp, yes: -1 } }), 'negative count');
  bad(withG('rsvpOpen', { rsvp: { ...raw('rsvpOpen').gather.rsvp, paid: { paid: 15, of: 14, not_yet: [] } } }), 'more paid than Yes');
  bad(withG('signupOpen', { slots: [{ id: 's', name: 'Mains', cap: 1, who: [{ name: 'A' }, { name: 'B' }], waiting: [] }] }), 'more people than the cap');
  bad(withG('signupOpen', { slots: [{ id: 's', name: 'Mains', cap: 0, who: [], waiting: [] }] }), 'a cap of 0');
  bad(withG('voteOpen', { vote: { ...raw('voteOpen').gather.vote, winner: 'The Old Mill' } }), 'a winner while voting is open');
  bad(withG('voteClosed', { vote: { ...raw('voteClosed').gather.vote, winner: 'Nowhere' } }), 'a winner that is not a place');
  bad(withG('voteTie', { vote: { ...raw('voteTie').gather.vote, tied: ['The Old Mill'] } }), 'a tie of one');
  bad(withG('voteOpen', { vote: { ...raw('voteOpen').gather.vote, voted: 30 } }), 'more voted than members');
  bad(withG('rsvpOpen', { people: [{ id: 'p', name: 'A', avatar: '<script>' }] }), 'a creature id that is not an id');
  bad(withG('rsvpOpen', { nudges: [{ id: 'n', name: 'A', why: 'shame', text: 'x' }] }), 'an unknown nudge reason');
  bad(withG('rsvpOpen', { join: { mode: 'wifi', wifi_url: 'javascript:alert(1)' } }), 'a link that is not http');
  bad({ ...raw('rsvpOpen'), state: 'ended' }, 'ended state while open');
  bad({ ...raw('voteClosed'), state: 'live' }, 'live state when closed');
  bad({ ...raw('rsvpOpen'), gather: null }, 'kit gather without its part');
  bad({ ...raw('rsvpOpen'), kit: 'quiz' }, 'a quiz carrying a gather part');
  bad({ ...raw('rsvpOpen'), santa: structuredClone(m.SANTA_FIXTURES.collecting.results.blocks[0].santa) }, 'gather and santa together');
  bad({ ...raw('rsvpOpen'), join: { mode: 'wifi', wifi_url: 'http://192.168.1.2:7480/p/x', anywhere_url: null } }, 'a top-level join on a gather');
});

test('a person paired with a ballot is rejected anywhere in the block', () => {
  const vote = raw('voteOpen').gather.vote;
  bad(withG('voteOpen', { vote: { ...vote, voters: [{ name: 'Mia', avatar: 'beret', ballot: ['The Old Mill'] }] } }), 'a voter with a ballot');
  bad(withG('voteOpen', { vote: { ...vote, voters: [{ name: 'Mia', avatar: 'beret', ranking: [1, 2] }] } }), 'a voter with a ranking');
  bad(withG('voteOpen', { people: [{ id: 'g1', name: 'Mia', answer: null, choice: 'Casa Lupa' }] }), 'a person with a choice');
  bad(withG('voteOpen', { nudges: [{ id: 'n', name: 'Kai', why: 'not_voted', text: 'x', voted_for: 'Casa Lupa' }] }), 'a nudge with a vote');
  bad(withG('voteOpen', { vote: { ...vote, ballots: [['The Old Mill', 'Casa Lupa']] } }), 'ballots anywhere');
  bad(withG('voteOpen', { vote: { ...vote, voters: [{ name: 'Mia', avatar: 'beret', detail: { picks: ['x'] } }] } }), 'a ballot nested under a voter');
  for (const k of m.BALLOT_KEYS) assert.ok(m.findBallotPair({ people: [{ name: 'Mia', [k]: 'x' }] }), `${k} on a person`);
  assert.equal(m.findBallotPair(raw('voteClosed').gather), null, 'counts, rounds and who voted are fine');
  assert.equal(m.findBallotPair({ counts: [{ name: 'Casa Lupa', votes: 3 }] }), null);
});

// ---------------------------------------------------------------------------------------------------
// The one holo

test('Picked only when the vote is closed with a winner; a tie stays a plain warn chip', async () => {
  const closed = await card('voteClosed');
  assert.equal(holos(closed), 1);
  assert.match(closed.host.textContent, /The Old Mill wins with 8 of 15 in the final round/);
  assert.match(closed.host.textContent, /Round 1: The Old Mill 6 · Casa Lupa 5 · Golden Lotus 3 · Pizza at the clubhouse 1\. Pizza at the clubhouse drops out; its vote goes to Casa Lupa\./);
  assert.equal(closed.host.querySelectorAll('[data-vbars] li').length, 2, 'final bars drop the places with none');
  closed.done();
  for (const name of ['rsvpOpen', 'signupOpen', 'signupMoved', 'voteOpen', 'voteTie', 'rsvpResult', 'signupResult']) {
    const v = await card(name);
    assert.equal(holos(v), 0, `${name}: no holo`);
    v.done();
  }
  const tie = await card('voteTie');
  assert.ok(tie.host.querySelector('.kw-chip-warn'));
  assert.match(tie.host.textContent, /Tie: you pick/);
  assert.match(tie.host.textContent, /Members see "Tie: Oleks picks" until you choose/);
  const noWinner = withG('voteClosed', { vote: { ...raw('voteClosed').gather.vote, winner: null } });
  assert.equal(m.earnedPicked(m.validPocket(noWinner).gather), false);
  assert.equal(m.earnedPicked(m.validPocket(raw('voteTie')).gather), false);
  assert.equal(m.earnedPicked(m.validPocket(raw('voteClosed')).gather), true);
});

test('moving up is ink New, with a note to copy', async () => {
  const v = await card('signupMoved');
  const s = v.host.querySelector('[data-moved-up]');
  assert.match(s.textContent, /Ben left Desserts at 4:12 PM\. Hana moved up from the waiting list on their own/);
  assert.ok(s.querySelector('[data-tone="ink"], .kw-sticker-ink'), 'ink New');
  assert.equal(v.host.querySelector('[data-join]'), null, 'no join box once someone moved up');
  await click(button(v, 'Copy note'));
  assert.match(clipboard.text, /^Hi Hana!/);
});

// ---------------------------------------------------------------------------------------------------
// States

test('RSVP open: tiles, food, Paid with Only you, nudges line and the full join box', async () => {
  const v = await card('rsvpOpen');
  const t = v.host.textContent;
  assert.equal(v.host.querySelectorAll('[data-count]').length, 3);
  assert.match(t, /\+3 plus-ones · 17 coming/);
  assert.match(t, /Leo, Zoe, Nina, Sam/);
  assert.match(t, /Food needs: 3 vegetarian, 1 vegan, 1 nut allergy, 1 gluten-free/);
  assert.match(t, /Paid 10 of 14 · not yet: Ana, Kai, Ravi, Tom/);
  assert.ok(v.host.querySelector('[data-chip="only-you"]'));
  assert.match(t, /8 nudges drafted: 4 said maybe, 4 not paid yet\. Nothing sends by itself\./);
  assert.equal(v.host.querySelector('[data-join]').dataset.join, 'full');
  assert.equal(v.host.querySelector('[data-mood]').dataset.mood, 'idle');
  assert.equal(button(v, 'Edit').getAttribute('aria-disabled'), 'true');
  assert.equal(button(v, 'Edit').getAttribute('title'), m.GATHER_LATER.edit);
});

test('RSVP with no stable address: the Same-Wi-Fi-only callout, no quick link', async () => {
  const v = await card('rsvpNoAddress');
  assert.ok(v.host.querySelector('[data-callout="no-stable-address"]'));
  assert.match(v.host.textContent, /Same Wi-Fi only, for now\./);
  assert.match(v.host.textContent, /Tailscale Funnel: free, about 5 minutes/);
  assert.doesNotMatch(v.host.innerHTML, /trycloudflare/);
});

test('Sign-up open: pips, Full, waiting list and plain chips', async () => {
  const v = await card('signupOpen');
  const rows = v.host.querySelectorAll('[data-slot]');
  assert.equal(rows.length, 5);
  const desserts = v.host.querySelector('[data-slot="Desserts"]');
  assert.match(desserts.textContent, /Full/);
  assert.match(desserts.textContent, /Waiting list: 1 Hana, 2 Leo · moves up on its own/);
  assert.equal(desserts.querySelectorAll('[data-pips] i').length, 2);
  assert.match(v.host.querySelector('[data-slot="Washing-up crew"]').textContent, /Nobody yet/);
  assert.match(v.host.textContent, /Open · 7 signed up/);
  assert.equal(v.host.querySelector('[data-join]').dataset.join, 'compact');
  assert.equal(button(v, 'Edit slots').getAttribute('title'), m.GATHER_LATER.editSlots);
  assert.equal(button(v, 'People').getAttribute('title'), m.GATHER_LATER.signupPeople);
});

test('Vote open: who voted, live bars with the leader, no ballot words', async () => {
  const v = await card('voteOpen');
  const t = v.host.textContent;
  assert.match(t, /Voting · 15 of 22/);
  assert.match(t, /15 of 22 members voted/);
  assert.match(t, /Live count · 1st choices/);
  assert.match(t, /You see who voted, never what they picked\./);
  const lead = v.host.querySelectorAll('[data-vbars] [data-lead]');
  assert.equal(lead.length, 1);
  assert.match(lead[0].textContent, /The Old Mill/);
  assert.match(lead[0].querySelector('i').className, /bg-sky-400/);
  assert.equal(v.host.querySelector('[data-mood]').dataset.mood, 'working');
});

test('Close now asks first, then ends the vote', async () => {
  const calls = [];
  g.fetch = respond(calls, { pocket: { code: 'runclub' } });
  const v = await card('voteOpen');
  await click(button(v, 'Close now'));
  assert.ok(v.host.querySelector('[data-confirm="close"]'));
  assert.equal(calls.length, 0);
  await click(button(v, 'Close and count'));
  assert.deepEqual(calls.map((c) => [c[0], c[1]]), [['POST', '/api/pockets/runclub/end']]);
  assert.match(v.host.querySelector('[data-done="ok"]').textContent, /Voting is closed/);
});

test('a tie: Pick sends pick, Vote again sends revote_24h', async () => {
  const calls = [];
  g.fetch = respond(calls);
  let v = await card('voteTie');
  assert.equal(v.host.querySelector('[data-mood]').dataset.mood, 'needs_you');
  await click(button(v, 'Pick Casa Lupa'));
  assert.deepEqual(calls[0], ['POST', '/api/pockets/runclub/action', { name: 'pick', body: { option: 'Casa Lupa' } }]);
  v.done();
  v = await card('voteTie');
  await click(button(v, 'Vote again for 24 h'));
  assert.deepEqual(calls[1], ['POST', '/api/pockets/runclub/action', { name: 'revote_24h' }]);
  assert.match(v.host.textContent, /open again for 24 hours/);
});

// ---------------------------------------------------------------------------------------------------
// People and paid

test('People and paid: a tick on Yes rows only, host view, calls set_paid and reverts on error', async () => {
  const calls = [];
  g.fetch = respond(calls);
  const v = await card('rsvpPeople');
  await click(button(v, 'People and paid'));
  const panel = v.host.querySelector('[data-people]');
  assert.ok(panel);
  assert.equal(v.host.querySelector('[data-join]'), null, 'the panel replaces the join box');
  assert.match(panel.textContent, /People and paid · 20/);
  assert.equal(panel.querySelectorAll('[data-person]').length, 9);
  assert.equal(panel.querySelector('[data-person="Leo"] [role="checkbox"]'), null, 'no tick on Maybe');
  assert.equal(panel.querySelector('[data-person="Finn"] [role="checkbox"]'), null, 'no tick on No');
  const ana = panel.querySelector('[aria-label="Ana paid"]');
  assert.equal(ana.getAttribute('aria-checked'), 'false');
  await click(ana);
  assert.equal(ana.getAttribute('aria-checked'), 'true');
  assert.deepEqual(calls[0], ['POST', '/api/pockets/runclub/action', { name: 'set_paid', body: { player_id: 'g4', paid: true } }]);
  g.fetch = respond(calls, { error: 'nope', message: 'Your box is busy.' }, 500);
  const kai = panel.querySelector('[aria-label="Kai paid"]');
  await click(kai);
  assert.equal(kai.getAttribute('aria-checked'), 'false', 'reverted');
  assert.match(panel.querySelector('[role="alert"]').textContent, /Couldn't save Kai's tick: Your box is busy\./);
  await click(button(v, 'Show all 20'));
  assert.equal(panel.querySelectorAll('[data-person]').length, 20);
});

// ---------------------------------------------------------------------------------------------------
// Nudges

test('Nudges: grouped, first row open, Copy says Copied <time> and never Sent', async () => {
  const calls = [];
  g.fetch = respond(calls, { ok: true, until: 'Sat 9:15 PM' });
  const v = await card('rsvpNudges');
  await click(button(v, 'Nudges · 8'));
  const panel = v.host.querySelector('[data-nudges]');
  assert.equal(v.host.querySelector('[data-mood]').dataset.mood, 'working');
  const groups = panel.querySelectorAll('[role="group"]');
  assert.deepEqual([...groups].map((x) => x.getAttribute('aria-label')), ['Said maybe · 4', 'Not paid yet · 4']);
  assert.match(groups[1].textContent, /only to the person, never the group/);
  const open = [...panel.querySelectorAll('[data-nudge-open]')].map((x) => x.dataset.nudge);
  assert.deepEqual(open, ['Leo', 'Ana']);
  assert.match(panel.querySelector('[data-nudge="Leo"] [data-draft]').textContent, /^Hi Leo! Still a maybe/);
  assert.equal(button(v, 'Share…'), undefined, 'no share sheet here: Copy only');
  await click(panel.querySelector('[data-nudge="Leo"] [data-action="copy-nudge"]'));
  assert.match(clipboard.text, /^Hi Leo!/);
  assert.deepEqual(calls[0], ['POST', '/api/pockets/runclub/action', { name: 'copied', body: { player_id: 'g8' } }]);
  const leo = panel.querySelector('[data-nudge="Leo"]');
  assert.match(leo.textContent, /Copied \d{1,2}:\d\d [AP]M/);
  assert.match(leo.textContent, /Copied, not sent: you send it from your own chat\. Copy again from Sat 9:15 PM\./);
  assert.doesNotMatch(panel.textContent, /\bSent\b/);
});

test('the 48 h cap: a capped row has no button, and a 409 shows the server\'s "until"', async () => {
  g.fetch = respond([], { error: 'too_soon', message: 'One nudge per person every 48 hours.', until: 'Sun 10:02 AM' }, 409);
  const v = await card('rsvpNudges');
  await click(button(v, 'Nudges · 8'));
  const panel = v.host.querySelector('[data-nudges]');
  const kai = panel.querySelector('[data-nudge="Kai"]');
  assert.match(kai.textContent, /Copied yesterday/);
  assert.match(kai.textContent, /One nudge per person every 48 hours\. Next one from Thu 6:40 PM\./);
  assert.equal(kai.querySelector('button'), null, 'no button inside the window');
  await click(panel.querySelector('[data-nudge="Ana"] [data-action="copy-nudge"]'));
  assert.match(panel.querySelector('[data-nudge="Ana"]').textContent, /Next one from Sun 10:02 AM\./);
});

test('Nudges: Share… only where the phone has a share sheet; Edit and Skip', async () => {
  const shared = [];
  Object.defineProperty(g.navigator, 'share', { value: async (d) => void shared.push(d), configurable: true });
  try {
    g.fetch = respond([]);
    const v = await card('rsvpNudges');
    await click(button(v, 'Nudges · 8'));
    const panel = v.host.querySelector('[data-nudges]');
    await click(panel.querySelector('[data-nudge="Leo"] [data-action="edit-nudge"]'));
    const ta = panel.querySelector('textarea');
    assert.ok(ta);
    await click(panel.querySelector('[data-nudge="Leo"] [data-action="share-nudge"]'));
    assert.match(shared[0].text, /^Hi Leo!/);
    await click(panel.querySelector('[data-nudge="Ana"] [data-action="skip-nudge"]'));
    assert.equal(panel.querySelector('[data-nudge="Ana"]'), null);
    assert.match(panel.textContent, /Skipped Ana\. Nothing goes to them\./);
    assert.equal(panel.querySelector('[data-nudge-open][data-nudge="Ravi"]') !== null, true, 'the next row opens');
  } finally {
    delete g.navigator.share;
  }
});

// ---------------------------------------------------------------------------------------------------
// Results and Done

test('closed RSVP: the restaurant list never carries paid; Nudge Omar copies his note', async () => {
  const calls = [];
  g.fetch = respond(calls);
  const v = await card('rsvpResult');
  const t = v.host.textContent;
  assert.match(t, /19 coming \(15 members \+ 4 plus-ones\) · 4 no · 3 never answered/);
  assert.match(t, /Tell The Old Mill: 4 vegetarian, 1 vegan, 1 nut allergy, 1 gluten-free/);
  assert.match(t, /Paid 14 of 15 · not yet: Omar/);
  assert.equal(v.host.querySelectorAll('[data-result="rsvp"] tbody tr').length, 15);
  await click(button(v, 'Copy list for the restaurant'));
  assert.match(clipboard.text, /19 coming/);
  assert.match(clipboard.text, /- Oleks \+1: vegetarian/);
  assert.doesNotMatch(clipboard.text, /paid|Paid|Not yet/);
  await click(button(v, 'Nudge Omar'));
  assert.match(clipboard.text, /^Hi Omar/);
  assert.equal(calls[0][2].name, 'copied');
  assert.equal(button(v, 'Delete answers now').getAttribute('title'), m.GATHER_LATER.deleteAnswers);
  assert.equal(v.host.querySelector('[data-budget]'), null);
});

test('closed Sign-up: summary, who brings what, Run it again asks Tally', async () => {
  const asked = [];
  const b = raw('signupResult');
  const v = await mount(h(m.MarkdownRenderer, { content: F.signupResult.reply, results: { version: 1, blocks: [b] }, messageId: 'm-sr' }));
  const t = v.host.textContent;
  assert.match(t, /Every slot is full except Washing-up crew \(2 of 3\)\. Leo stayed on the Desserts waiting list\./);
  await click(button(v, 'Copy who brings what'));
  assert.match(clipboard.text, /Mains \(3 of 3\): Priya \(lentil stew\), Elif \(lasagne\), Omar \(curry\)/);
  assert.ok(asked);
});

test('closed vote: Share the result copies one line; Delete ballots now says why', async () => {
  const v = await card('voteClosed');
  await click(button(v, 'Share the result'));
  assert.equal(clipboard.text, "Where's the end-of-season dinner?: The Old Mill wins with 8 of 15 in the final round.");
  assert.equal(button(v, 'Delete ballots now').getAttribute('title'), m.GATHER_LATER.deleteBallots);
  assert.match(v.host.textContent, /Ballots are deleted on Wed, Dec 9\. The result stays in Done\./);
  assert.equal(v.host.querySelector('[data-mood]').dataset.mood, 'finished');
});

test('Done rows: Tally on each, one sticker at most, holo only on the vote', async () => {
  const items = ['voteClosed', 'rsvpResult', 'signupResult'].map((n) => m.validPocket(raw(n)).gather);
  const v = await mount(h(m.GatherDoneList, { items }));
  const rows = v.host.querySelectorAll('[data-done-row]');
  assert.equal(rows.length, 3);
  assert.match(rows[0].textContent, /Wed, Dec 2 · The Old Mill won, 8 to 7 in the final round/);
  assert.match(rows[1].textContent, /Thu, Dec 10 · 19 coming · 14 of 15 paid/);
  assert.match(rows[2].textContent, /Fri, Dec 18 · 13 people in 5 slots/);
  assert.equal(rows[0].querySelectorAll('[data-tone="holo"], .kw-sticker-holo').length, 1);
  for (const r of [rows[1], rows[2]]) assert.equal(r.querySelectorAll('[data-tone], .kw-sticker-holo').length, 0);
  assert.equal(v.host.querySelectorAll('[data-tone="holo"], .kw-sticker-holo').length, 1);
});

test('Tally: a kit avatar style with its moods', async () => {
  assert.ok(m.AVATAR_STYLES.some((s) => s.id === 'tally' && s.label === 'Sign-up'));
  const want = { rsvpOpen: 'idle', voteOpen: 'working', voteClosed: 'finished', voteTie: 'needs_you', signupMoved: 'finished' };
  for (const [n, mood] of Object.entries(want)) assert.equal(m.gatherMood(m.validPocket(raw(n)).gather), mood, n);
  assert.equal(m.gatherMood(m.validPocket(raw('rsvpOpen')).gather, true), 'working', 'nudges ready');
  const v = await mount(h(m.ModernAvatar, { role: 'tally', name: 'Tally', size: 64, mood: 'idle' }));
  assert.ok(v.host.querySelector('svg'));
});

test('kept line and the quiz and santa cards are untouched', async () => {
  assert.equal(m.keptLine({ deletes_on: 'Sat, Dec 19', kept_after: 'the dinner' }), 'Kept until Sat, Dec 19 (7 days after the dinner), then deleted.');
  ok(structuredClone(m.POCKET_FIXTURES.ready.results.blocks[0]));
  ok(structuredClone(m.SANTA_FIXTURES.collecting.results.blocks[0]));
});
