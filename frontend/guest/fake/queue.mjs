// Fake Queue ticket pocket (pa11b) for the guest page and its big screen, in the shapes of Vega's queue API
// (src/pocket/queue.rs, pa11/queue-impl.md, which wins over the design) inside pa1's v1 envelope. Codes `queue-<scene>`
// are the guest page, `screen-queue-<scene>` the TV (server.mjs routes them here). `QueueBox` keeps a little state so a
// walk can join, take a ticket, be called, leave and see every refusal. Title, line and texts are sample data from the
// pa11 mockups.

const at = (min) => new Date(Date.now() + min * 60000).toISOString();
const face = { title: "Tine's Bakery", line: 'Pick up at the side counter', hours_until: '13:00', call_minutes: 3, ask_name: false, host_name: 'Tine', max_waiting: 40 };

const T = (number, state, extra = {}) => ({
  number, name: null, place: null, ahead: null, state, taken_at: at(-14), called_at: null, call_ends_at: null, done_at: null, closed: false, ...extra,
});
const waiting = (number, place) => T(number, 'waiting', { place, ahead: place - 1 });

/** Scenes: `q` the queue (now serving, waiting, avg), `ticket` this phone's (a seat when set), `take` the refusal Take gets. */
export const queueScenes = {
  // 1 take: open, 5 waiting, measured 3 min a turn.
  take: { q: { now_serving: 23, waiting: 5, avg_minutes: 3 } },
  // 1 take, first name on.
  'take-name': { q: { now_serving: 23, waiting: 5, avg_minutes: 3 }, face: { ask_name: true } },
  // 1 take, fewer than 3 calls today: no minutes anywhere.
  'take-early': { q: { now_serving: 2, waiting: 1, avg_minutes: null } },
  // 2 your ticket, 4th.
  ticket: { q: { now_serving: 23, waiting: 5, avg_minutes: 3 }, ticket: waiting(27, 4) },
  // 3 iPhone, came back (the walk sets the user agent).
  iphone: { q: { now_serving: 23, waiting: 5, avg_minutes: 3 }, ticket: waiting(27, 4) },
  // 4 you're next.
  next: { q: { now_serving: 26, waiting: 3, avg_minutes: 3 }, ticket: waiting(27, 1) },
  // 5 your turn: called a few seconds ago, 3 minutes to come.
  turn: { q: { now_serving: 27, waiting: 2, avg_minutes: 3 }, ticket: T(27, 'called', { called_at: at(-0.2), call_ends_at: at(2.8) }) },
  // 9 skipped.
  skipped: { q: { now_serving: 28, waiting: 6, avg_minutes: 3 }, ticket: T(27, 'skipped', { called_at: at(-4), done_at: at(-1) }) },
  // 10 served.
  served: { q: { now_serving: 27, waiting: 2, avg_minutes: 3 }, ticket: T(27, 'served', { called_at: at(-2), done_at: at(-1) }) },
  // 7 left (Leave the queue is walked from `ticket`; this is the page after).
  left: { q: { now_serving: 23, waiting: 4, avg_minutes: 3 }, ticket: T(27, 'left', { done_at: at(-0.5) }) },
  // 8 paused, holding a ticket.
  paused: { q: { state: 'paused', now_serving: 23, waiting: 5, avg_minutes: 3, pause_text: 'Back in 10 minutes', paused_at: at(-4) }, ticket: waiting(27, 4) },
  // 11 closed while holding a ticket.
  closed: { q: { state: 'closed', now_serving: 24, waiting: 0, avg_minutes: 3, close_text: "Sorry, we've closed for today. Thanks for waiting, and see you next time!" }, ticket: T(27, 'waiting', { closed: true, done_at: at(-1) }) },
  // 12 ended.
  ended: { q: { state: 'ended', now_serving: 61, waiting: 0, avg_minutes: 3 }, ended: true },
  // Refusals on Take a ticket (the view still says open, then refetches).
  full: { q: { now_serving: 23, waiting: 5, avg_minutes: 3 }, take: { status: 409, error: 'full', message: 'The queue is full right now.' }, after: { waiting: 40 } },
  'r-paused': { q: { now_serving: 23, waiting: 5, avg_minutes: 3 }, take: { status: 409, error: 'paused', message: 'The queue is paused.' }, after: { state: 'paused', pause_text: 'Back in 10 minutes', paused_at: at(-1) } },
  'r-closed': { q: { now_serving: 23, waiting: 5, avg_minutes: 3 }, take: { status: 410, error: 'closed', message: 'The queue is closed for today.' }, after: { state: 'closed' } },
  slow: { q: { now_serving: 23, waiting: 5, avg_minutes: 3 }, take: { status: 429, error: 'slow_down', limit: 'phone', message: 'Slow down.' } },
  'slow-net': { q: { now_serving: 23, waiting: 5, avg_minutes: 3 }, take: { status: 429, error: 'slow_down', limit: 'network', message: 'Slow down.' } },
  // Big screen.
  'tv-open': { q: { now_serving: 24, waiting: 5, avg_minutes: 3, next: [25, 26, 27, 28, 29] } },
  'tv-paused': { q: { state: 'paused', now_serving: 23, waiting: 5, avg_minutes: 3, pause_text: 'Back in 10 minutes', next: [24, 25, 26] } },
  'tv-closed': { q: { state: 'closed', now_serving: 61, waiting: 0, avg_minutes: 3, close_text: "Sorry, we've closed for today. See you tomorrow from 7 AM!", next: [] } },
};

const copy = (x) => JSON.parse(JSON.stringify(x));

export class QueueBox {
  constructor(code) {
    this.code = code;
    this.tv = code.startsWith('screen-queue-');
    // A `--<n>` tail gives a walk its own fresh box of the same scene.
    const scene = code.replace(/^(screen-)?queue-/, '').replace(/--\d+$/, '');
    this.s = queueScenes[scene];
    this.q = { state: 'open', now_serving: null, waiting: 0, avg_minutes: null, pause_text: null, close_text: null, paused_at: null, next: [], ...copy(this.s?.q ?? {}) };
    this.face = { ...face, ...(this.s?.face ?? {}) };
    this.ticket = this.s?.ticket ? copy(this.s.ticket) : null;
    this.seat = !!this.s?.ticket;
    this.version = 1;
    this.counter = 27;
    this.takes = [];
    this.writes = [];
  }
  ended() {
    return !!this.s?.ended;
  }
  view(seated) {
    const q = this.q;
    if (this.tv) {
      return { state: q.state, title: this.face.title, line: this.face.line, now_serving: q.now_serving, next: q.next, waiting: q.waiting, avg_minutes: q.avg_minutes, pause_text: q.pause_text, close_text: q.close_text, hours_until: this.face.hours_until, now: new Date().toISOString() };
    }
    return {
      state: q.state, title: this.face.title, line: this.face.line, hours_until: this.face.hours_until, now_serving: q.now_serving, waiting: q.waiting, avg_minutes: q.avg_minutes,
      call_minutes: this.face.call_minutes, max_waiting: this.face.max_waiting, ask_name: this.face.ask_name, host_name: this.face.host_name, pause_text: q.pause_text,
      close_text: q.close_text, paused_at: q.paused_at, now: new Date().toISOString(), my_ticket: seated ? this.ticket : null,
    };
  }
  env(seated = true) {
    const pocket = { code: this.code, kit: 'queue', title: this.face.title, state: this.ended() ? 'ended' : 'open', opens_at: null, closes_at: null, ended_at: null, expires_at: null, players: null, max_players: null, needs_pin: false, retention: { kind: 'after_done', days: 0 } };
    const viewer = this.tv ? 'screen' : seated ? 'guest' : 'anyone';
    return { api: 'pocket/1', version: this.version, viewer, pocket, ...(seated && !this.tv ? { me: { id: 'g1', name: '', avatar: 'antenna', team: null, notices: [] } } : {}), view: this.view(seated) };
  }
  /** A push from a walk: a function edits the box (call the next number, pause, close). */
  set(change) {
    if (typeof change === 'function') change(this);
    this.version++;
  }
  /** The host calls the next number: this phone's ticket moves up, and at place 0 it is called. */
  callNext() {
    this.q.now_serving = (this.q.now_serving ?? 0) + 1;
    this.q.next = this.q.next.filter((n) => n > this.q.now_serving);
    this.q.waiting = Math.max(0, this.q.waiting - 1);
    const t = this.ticket;
    if (t?.state === 'waiting') {
      if (t.number === this.q.now_serving) Object.assign(t, { state: 'called', place: null, ahead: null, called_at: new Date().toISOString(), call_ends_at: at(this.face.call_minutes) });
      else Object.assign(t, { place: Math.max(1, t.place - 1), ahead: Math.max(0, t.ahead - 1) });
    }
  }
  handle(path, body = {}, { seated: cookie = false, signed = false, method = 'GET' } = {}) {
    const seated = cookie || this.seat;
    const [what, query] = path.split('?');
    const err = (status, error, extra = {}) => ({ status, body: { error, message: extra.message ?? 'Fake queue box.', ...extra } });
    if (!this.s) return err(404, 'not_found');
    if (what === 'info') return { status: 200, body: this.env(false) };
    if (this.tv) {
      if (what === 'host/signin') {
        if (method !== 'POST') return err(405, 'method');
        if (this.ended()) return err(410, 'ended');
        return { status: 200, body: this.env(), cookie: true };
      }
      if (what === 'host/state') {
        if (this.ended()) return err(410, 'ended');
        if (!signed) return err(401, 'signed_out');
        if (new URLSearchParams(query).get('view') !== 'screen') return err(403, 'screen_only');
        const since = Number(new URLSearchParams(query).get('since'));
        if (since && since === this.version) return { status: 204, body: null };
        return { status: 200, body: this.env() };
      }
      this.writes.push(`${method} ${path}`);
      return err(403, 'screen_only');
    }
    if (this.ended()) return err(410, 'ended');
    if (what === 'state') {
      if (!seated) return err(401, 'signed_out');
      const since = Number(new URLSearchParams(query).get('since'));
      if (since && since === this.version) return { status: 204, body: null };
      return { status: 200, body: this.env(true) };
    }
    if (what === 'join') {
      // pa11b: join {} hands out a key and nothing else; the ticket comes with `take`.
      if (Object.keys(body).length) return err(400, 'bad_join', { message: 'Join takes no fields.' });
      this.version++;
      return { status: 201, body: this.env(true) };
    }
    if (what === 'action') {
      if (!seated) return err(401, 'signed_out');
      if (body.name === 'take') {
        this.takes.push(body.body ?? {});
        if (this.ticket && ['waiting', 'called'].includes(this.ticket.state) && !this.ticket.closed) return err(409, 'has_ticket', { ticket: this.ticket });
        const r = this.s.take;
        if (r) {
          if (this.s.after) Object.assign(this.q, this.s.after);
          this.version++;
          return err(r.status, r.error, r);
        }
        const name = body.body?.name;
        if (name !== undefined && (typeof name !== 'string' || [...name.trim()].length > 24)) return err(400, 'bad_action', { message: 'A first name, up to 24 letters.' });
        this.counter += 1;
        this.q.waiting += 1;
        this.ticket = waiting(this.counter, this.q.waiting);
        this.ticket.taken_at = new Date().toISOString();
        this.version++;
        return { status: 200, body: this.env(true) };
      }
      if (body.name === 'leave') {
        if (this.ticket && ['waiting', 'called'].includes(this.ticket.state)) {
          Object.assign(this.ticket, { state: 'left', place: null, ahead: null, call_ends_at: null, done_at: new Date().toISOString() });
          this.q.waiting = Math.max(0, this.q.waiting - 1);
        }
        this.version++;
        return { status: 200, body: this.env(true) };
      }
      return err(400, 'bad_action', { message: 'Unknown action.' });
    }
    return err(404, 'not_found');
  }
}
