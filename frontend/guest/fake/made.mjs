// Fake `made` pocket (pa12 P2) for the ready-made blocks, in the shapes of pa12/plan.md section 1 and P2/blocks.md
// inside pa1's v1 envelope. Codes `made-<scene>` are the phone, `screen-made-<scene>` the big screen and
// `remote-made-<scene>` the host remote (server.mjs routes them here). `MadeBox` plays the box: it keeps the rows,
// applies each collection's read rule per viewer (the table in plan.md section 1), counts the ballots and takes the
// actions of section 2, so a walk can add a book, vote, guess and see the reveal. The two specs are the section 1
// examples (a book shelf with a vote, and guess-the-opening-line). Names, books and lines are sample data.

const at = (s) => new Date(Date.now() + s * 1000).toISOString();
const copy = (x) => JSON.parse(JSON.stringify(x));

const SHELF = {
  format: 'made/1',
  title: 'Book swap shelf',
  collections: {
    shelf: { read: 'everyone', write: 'guest_many', max_per_guest: 5, fields: { title: { type: 'text', max: 80 }, genre: { type: 'choice', of: ['novel', 'poetry', 'other'] } } },
    picks: { read: 'mine', write: 'guest_one', fields: { pick: { type: 'row', of: 'shelf' } } },
  },
  actions: { vote: { collection: 'picks', count: 'one_pick' } },
  words: { join: 'Join the book swap', done: 'Thanks for swapping!' },
  screens: {
    phone: { blocks: [
      { type: 'say', text: 'Add a book you would swap, then vote for the next read.' },
      { type: 'form', into: 'shelf', noun: ['book', 'books'], fields: [{ name: 'title', type: 'text', label: 'Book title', max: 80 }, { name: 'genre', type: 'choice', label: 'Kind of book', of: ['novel', 'poetry', 'other'] }], submit: 'Add to the shelf', done: 'Last added:' },
      { type: 'board', from: 'shelf', label: 'Vote for the next read', noun: ['book', 'books'], show: 'title', vote: 'picks' },
    ] },
    screen: { blocks: [
      { type: 'say', text: 'Book swap: vote for the next read', size: 'big' },
      { type: 'board', from: 'shelf', label: 'Vote for the next read', show: 'title', vote: 'picks' },
      { type: 'list', from: 'shelf', label: 'On the shelf', show: ['title', 'genre'], noun: ['book', 'books'], empty: 'No books on the shelf yet.' },
    ] },
    remote: { blocks: [
      { type: 'form', into: 'shelf', label: 'On the shelf', noun: ['book', 'books'], fields: [{ name: 'title', type: 'text', label: 'Book title' }] },
      { type: 'board', from: 'shelf', label: 'Vote for the next read', show: 'title', vote: 'picks' },
    ] },
  },
};

const OPENING = {
  format: 'made/1',
  title: 'Guess the opening line',
  collections: { guesses: { read: 'mine', write: 'guest_one', fields: { q: { type: 'int', min: 0, max: 49 }, text: { type: 'text', max: 100 } } } },
  secrets: { answers: [{ text: 'Call me Ishmael.', accept: ['call me ishmael'] }, { text: 'It was a bright cold day in April, and the clocks were striking thirteen.' }] },
  actions: { answer: { collection: 'guesses', against: 'secrets.answers', match: 'text', points: 1 }, timer: { seconds: 30 }, turns: { order: 'join' }, reveal: { secrets: ['answers'] } },
  steps: [{ say: 'Round 1: which book opens like this?' }, { open: 'guesses', timer: 30 }, { lock: 'guesses' }, { reveal: ['answers'] }],
  words: { join: 'Join the opening-line game', done: 'Thanks for playing!' },
  screens: {
    phone: { blocks: [
      { type: 'say', from: 'step', size: 'big' },
      { type: 'timer', label: 'Time left' },
      { type: 'turn' },
      { type: 'form', into: 'guesses', action: 'answer', fields: [{ name: 'text', type: 'text', label: 'Your guess', max: 100 }], submit: 'Send guess', done: 'Your guess:' },
      { type: 'scores', top: 10 },
      { type: 'reveal', secret: 'answers', show: 'text', label: 'The opening lines', hidden: 'The answers show after the last round.' },
    ] },
    screen: { blocks: [
      { type: 'say', from: 'step', size: 'big' },
      { type: 'timer', label: 'Time left' },
      { type: 'turn' },
      { type: 'list', from: 'guesses', label: 'Guesses', show: ['text'], noun: ['guess', 'guesses'] },
      { type: 'scores', top: 8 },
      { type: 'reveal', secret: 'answers', show: 'text', label: 'The opening lines', hidden: 'The answers show after the last round.' },
    ] },
    remote: { blocks: [
      { type: 'timer', label: 'Time left' },
      { type: 'turn' },
      { type: 'form', into: 'guesses', label: 'Guesses', noun: ['guess', 'guesses'], fields: [{ name: 'text', type: 'text', label: 'Guess' }] },
      { type: 'scores', top: 60 },
      { type: 'reveal', secret: 'answers', show: 'text', label: 'The opening lines' },
    ] },
  },
};

// The states the two examples don't reach: the phone's own list with Take back, a vote whose counts stay hidden (no
// `count` on the vote action, so the box sends no tally), a list this viewer sees as a count only, and a refusal (one
// book per guest, and Mo has one already).
const SWAP = {
  format: 'made/1',
  title: 'Book swap shelf',
  collections: {
    shelf: { read: 'everyone', write: 'guest_many', max_per_guest: 1, fields: { title: { type: 'text', max: 80 }, genre: { type: 'choice', of: ['novel', 'poetry', 'other'] } } },
    picks: { read: 'mine', write: 'guest_one', fields: { pick: { type: 'row', of: 'shelf' } } },
    wishes: { read: 'host', write: 'guest_many', fields: { title: { type: 'text', max: 80 } } },
  },
  actions: { vote: { collection: 'picks' } },
  words: { join: 'Join the book swap', done: 'Thanks for swapping!' },
  screens: {
    phone: { blocks: [
      { type: 'list', from: 'shelf', label: 'On the shelf', show: ['title', 'genre'], noun: ['book', 'books'], take_back: true },
      { type: 'form', into: 'shelf', noun: ['book', 'books'], fields: [{ name: 'title', type: 'text', label: 'Book title', max: 80 }], submit: 'Add to the shelf', done: 'Last added:' },
      { type: 'board', from: 'shelf', label: 'Vote for the next read', show: 'title', vote: 'picks' },
      { type: 'list', from: 'wishes', label: 'Wish list', show: ['title'], noun: ['wish', 'wishes'] },
    ] },
    screen: { blocks: [
      { type: 'board', from: 'shelf', label: 'Vote for the next read', show: 'title', vote: 'picks' },
      { type: 'list', from: 'wishes', label: 'Wish list', show: ['title'], noun: ['wish', 'wishes'] },
    ] },
  },
};

const PLAYERS = [
  { id: 'g1', name: 'Mo', avatar: 'antenna' },
  { id: 'g2', name: 'Ana', avatar: 'beret' },
  { id: 'g3', name: 'Ben', avatar: 'goggles' },
  { id: 'g4', name: 'Cleo', avatar: 'bulb' },
];
const row = (id, owner, values) => ({ id, owner, values, at: at(-60) });
const BOOKS = [
  row('s1', 'g2', { title: 'The Left Hand of Darkness', genre: 'novel' }),
  row('s2', 'g3', { title: 'Ariel', genre: 'poetry' }),
  row('s3', 'g1', { title: 'A Wizard of Earthsea', genre: 'novel' }),
  row('s4', 'g4', { title: 'Braiding Sweetgrass', genre: 'other' }),
];
const VOTES = [row('p1', 'g1', { pick: 's1' }), row('p2', 'g2', { pick: 's1' }), row('p3', 'g3', { pick: 's3' }), row('p4', 'g4', { pick: 's1' })];
const GUESSES = [row('q1', 'g1', { q: 0, text: 'Moby-Dick' }), row('q2', 'g2', { q: 0, text: 'Moby Dick' }), row('q3', 'g3', { q: 0, text: 'Robinson Crusoe' })];

/** Scenes: the spec, the rows per collection, whether this phone is seated, and the box's own parts. */
export const madeScenes = {
  shelf: { spec: SHELF, rows: { shelf: BOOKS, picks: VOTES } },
  'shelf-join': { spec: SHELF, rows: { shelf: BOOKS, picks: VOTES }, seat: false },
  'shelf-empty': { spec: SHELF, rows: { shelf: [], picks: [] } },
  'shelf-locked': { spec: SHELF, rows: { shelf: BOOKS, picks: VOTES }, locked: ['shelf', 'picks'] },
  opening: { spec: OPENING, rows: { guesses: GUESSES }, step: 1, timer: { seconds: 30, left: 22, running: true }, turn: { player: 'g1', next: 'g2' }, scores: [['g2', 2], ['g1', 1], ['g3', 1], ['g4', 0]] },
  'opening-revealed': { spec: OPENING, rows: { guesses: GUESSES }, step: 3, locked: ['guesses'], timer: { seconds: 0, running: false }, turn: { player: 'g3', next: 'g4' }, scores: [['g1', 2], ['g2', 2], ['g3', 1], ['g4', 0]], revealed: true },
  // `counts` makes the box send a count only for these collections to this viewer (the read rule's count-only case).
  swap: { spec: SWAP, rows: { shelf: BOOKS, picks: VOTES, wishes: [row('w1', 'g2', { title: 'Middlemarch' }), row('w2', 'g3', { title: 'Beloved' }), row('w3', 'g4', { title: 'Kindred' })] }, counts: ['wishes'] },
  ended: { spec: SHELF, rows: { shelf: BOOKS, picks: VOTES }, ended: true },
  // Hostile text from guests: a name and a title that look like markup are drawn as text, never parsed.
  hostile: { spec: SHELF, rows: { shelf: [row('s9', 'g9', { title: '<img src=x onerror=alert(1)>', genre: 'other' }), ...BOOKS.slice(0, 1)], picks: [] }, extra: [{ id: 'g9', name: '"><svg onload=alert(1)>', avatar: 'horns' }] },
};

export class MadeBox {
  constructor(code) {
    this.code = code;
    this.surface = code.startsWith('screen-made-') ? 'screen' : code.startsWith('remote-made-') ? 'remote' : 'phone';
    const scene = code.replace(/^(screen-|remote-)?made-/, '').replace(/--\d+$/, '');
    this.s = madeScenes[scene];
    this.spec = this.s?.spec;
    this.rows = copy(this.s?.rows ?? {});
    this.locked = new Set(this.s?.locked ?? []);
    this.players = [...PLAYERS, ...(this.s?.extra ?? [])];
    this.seat = this.surface === 'phone' && this.s?.seat !== false;
    this.me = 'g1';
    this.step = this.s?.step ?? 0;
    this.timer = this.s?.timer ? { seconds: this.s.timer.left ?? this.s.timer.seconds, ends_at: this.s.timer.running ? at(this.s.timer.left) : null, running: this.s.timer.running } : null;
    this.turn = this.s?.turn ? { ...this.s.turn } : null;
    this.scores = (this.s?.scores ?? []).map(([player, points]) => ({ player, points }));
    this.revealed = !!this.s?.revealed;
    this.version = 1;
    this.n = 100;
    this.actions = [];
  }
  ended() {
    return !!this.s?.ended;
  }
  /** One collection for one viewer, by its read rule (plan.md section 1). `viewer`: guest | screen | host. */
  part(name, viewer) {
    const c = this.spec.collections[name];
    const all = this.rows[name] ?? [];
    const own = all.filter((r) => r.owner === this.me);
    const out = (rows) => ({ rows: rows.map((r) => ({ id: r.id, owner: r.owner, mine: viewer === 'guest' && r.owner === this.me, values: r.values })), count: rows.length, locked: this.locked.has(name) });
    const count = () => ({ count: all.length, locked: this.locked.has(name) });
    let p;
    if (this.s.counts?.includes(name)) return count();
    if (c.read === 'everyone') p = out(all);
    else if (c.read === 'mine') p = viewer === 'guest' ? out(own) : count();
    else if (c.read === 'host') p = viewer === 'guest' ? out(own) : viewer === 'host' ? out(all) : null;
    else if (c.read === 'after_reveal') p = this.revealed ? out(all) : viewer === 'guest' ? out(own) : count();
    // A vote's tally is worked out here, on the box, and only counts: who voted for what never leaves it.
    if (p && Object.values(this.spec.actions ?? {}).some((a) => a.collection === name && a.count)) {
      p.tally = {};
      for (const r of all) p.tally[r.values.pick] = (p.tally[r.values.pick] ?? 0) + 1;
    }
    return p;
  }
  view(viewer) {
    const collections = {};
    for (const name of Object.keys(this.spec.collections)) {
      const p = this.part(name, viewer);
      if (p) collections[name] = p;
    }
    const surface = viewer === 'guest' ? 'phone' : viewer === 'host' ? 'remote' : 'screen';
    const steps = this.spec.steps ?? [];
    const can = viewer === 'host' ? [...this.hostCan()] : [];
    return {
      title: this.spec.title,
      words: this.spec.words,
      collections,
      revealed: this.revealed ? { answers: this.spec.secrets?.answers?.map((a) => ({ text: a.text })) ?? [] } : {},
      step: steps.length ? { index: this.step, of: steps.length, say: steps.slice(0, this.step + 1).reverse().find((x) => x.say)?.say ?? null } : null,
      timer: this.timer,
      turn: this.turn,
      scores: this.spec.actions?.answer ? this.scores : null,
      players: this.players.map((p) => ({ id: p.id, name: p.name, avatar: p.avatar })),
      can,
      now: new Date().toISOString(),
      app: { version: '1', blocks: this.spec.screens[surface]?.blocks ?? this.spec.screens.screen.blocks },
    };
  }
  *hostCan() {
    for (const name of Object.keys(this.spec.collections)) yield this.locked.has(name) ? 'unlock' : 'lock';
    if (this.spec.actions?.timer) yield this.timer?.running ? 'timer_stop' : 'timer_start';
    if (this.spec.actions?.turns) yield* ['turn_next', 'turn_skip'];
    if (this.spec.actions?.reveal && !this.revealed) yield 'reveal';
    if ((this.spec.steps ?? []).length > this.step + 1) yield 'step';
  }
  env(seated = true) {
    const viewer = this.surface === 'screen' ? 'screen' : this.surface === 'remote' ? 'host' : seated ? 'guest' : 'anyone';
    const pocket = { code: this.code, kit: 'made', title: this.spec?.title ?? '', state: this.ended() ? 'ended' : 'open', opens_at: null, closes_at: null, ended_at: null, expires_at: null, players: this.players.length, max_players: 40, needs_pin: false, retention: { kind: 'after_end', days: 7 } };
    const me = viewer === 'guest' ? { me: { id: this.me, name: 'Mo', avatar: 'antenna', team: null, notices: [] } } : {};
    // `info` (anyone) is the lobby: title and words, no rows (Kit::lobby).
    const view = viewer === 'anyone' ? { title: this.spec.title, words: this.spec.words } : this.view(viewer);
    return { api: 'pocket/1', version: this.version, viewer, pocket, ...me, view };
  }
  set(change) {
    if (typeof change === 'function') change(this);
    this.version++;
  }
  /** The actions of plan.md section 2 that the blocks send. A guest never names points or scores. */
  act(name, body = {}, host = false) {
    const err = (status, error, message) => ({ status, body: { error, message } });
    this.actions.push({ name, body, host });
    const col = body.collection;
    const c = this.spec.collections[col];
    const ok = () => (this.version++, { status: 200, body: this.env(true) });
    if (host) {
      if (name === 'lock' || name === 'unlock') {
        if (!c) return err(400, 'bad_action', 'No such list.');
        name === 'lock' ? this.locked.add(col) : this.locked.delete(col);
        return ok();
      }
      if (name === 'timer_start') return (this.timer = { seconds: 30, ends_at: at(30), running: true }), ok();
      if (name === 'timer_stop') return (this.timer = { seconds: 0, ends_at: null, running: false }), ok();
      if (name === 'turn_next' || name === 'turn_skip') {
        const i = this.players.findIndex((p) => p.id === this.turn?.next);
        this.turn = { player: this.turn?.next ?? 'g1', next: this.players[(i + 1) % this.players.length].id };
        return ok();
      }
      if (name === 'reveal') return (this.revealed = true), ok();
      if (name === 'step') return (this.step = Math.min(this.step + 1, (this.spec.steps ?? []).length - 1)), ok();
      return err(400, 'unknown_action', 'Unknown action.');
    }
    if (!c) return err(400, 'bad_action', 'No such list.');
    if (this.locked.has(col)) return err(409, 'locked', 'This is closed for now.');
    const rows = (this.rows[col] ??= []);
    if (name === 'put' || name === 'answer' || name === 'vote') {
      const values = name === 'vote' ? { pick: body.value } : body.values ?? {};
      if ('points' in values || 'score' in values) return err(400, 'bad_action', 'Points come from the box.');
      for (const [k, v] of Object.entries(values)) {
        const f = c.fields[k];
        if (!f) return err(400, 'bad_action', `No field ${k}.`);
        if (f.type === 'text' && (typeof v !== 'string' || [...v].length > (f.max ?? 200))) return err(400, 'bad_action', `${k} is too long.`);
        if (f.type === 'choice' && !f.of.includes(v)) return err(400, 'bad_action', `Pick one of the choices for ${k}.`);
      }
      if (c.write === 'guest_one') {
        const old = rows.findIndex((r) => r.owner === this.me);
        if (old >= 0) rows.splice(old, 1);
      } else if (rows.filter((r) => r.owner === this.me).length >= (c.max_per_guest ?? 50)) return err(409, 'full', `You can add up to ${c.max_per_guest} here.`);
      rows.push(row(`r${++this.n}`, this.me, values));
      return ok();
    }
    if (name === 'take_back') {
      const i = rows.findIndex((r) => r.id === body.id && r.owner === this.me);
      if (i < 0) return err(404, 'not_found', 'That one is gone already.');
      rows.splice(i, 1);
      return ok();
    }
    return err(400, 'unknown_action', 'Unknown action.');
  }
  handle(path, body = {}, { seated: cookie = false, signed = false, method = 'GET' } = {}) {
    const [what, query] = path.split('?');
    const err = (status, error) => ({ status, body: { error, message: 'Fake made box.' } });
    if (!this.s) return err(404, 'not_found');
    if (what === 'info') return { status: 200, body: this.env(false) };
    const since = Number(new URLSearchParams(query).get('since'));
    if (this.surface !== 'phone') {
      if (what === 'host/signin') {
        if (method !== 'POST') return err(405, 'method');
        if (typeof body?.token !== 'string' || !body.token || body.token === 'used') return err(401, 'not_host');
        return { status: 200, body: this.env(), cookie: true };
      }
      if (!signed) return err(401, 'signed_out');
      if (what === 'host/state') {
        const screenView = new URLSearchParams(query).get('view') === 'screen';
        if (this.surface === 'screen' && !screenView) return err(403, 'screen_only');
        if (since && since === this.version) return { status: 204, body: null };
        return { status: 200, body: this.env() };
      }
      if (what === 'host/action' && this.surface === 'remote') return this.act(body.name, body.body ?? {}, true);
      return err(403, 'screen_only');
    }
    const seated = cookie || this.seat;
    if (this.ended() && what !== 'state') return err(410, 'ended');
    if (what === 'state') {
      if (!seated) return err(401, 'signed_out');
      if (since && since === this.version) return { status: 204, body: null };
      return { status: 200, body: this.env(true) };
    }
    if (what === 'join') {
      if (typeof body.name !== 'string' || !body.name.trim()) return { status: 400, body: { error: 'bad_join', message: 'Type a first name.' } };
      this.seat = true;
      this.version++;
      return { status: 201, body: this.env(true) };
    }
    if (what === 'action') {
      if (!seated) return err(401, 'signed_out');
      return this.act(body.name, body.body ?? {});
    }
    return err(404, 'not_found');
  }
}
