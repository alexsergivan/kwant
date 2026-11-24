// Fake Wall pocket (pa14) for the guest page and its big screen, in the shapes of Juno's wall API (src/pocket/wall.rs,
// pa14/server-brief.md) inside pa1's v1 envelope. Codes `wall-<scene>` are the guest page, `screen-wall-<scene>` the TV
// (server.mjs routes them here). `WallBox` keeps a little state so a walk can join, send photos (POST photo, raw body,
// caption in X-Caption), see refusals, and push a newly approved photo to the TV.
// Titles, names and captions are sample data from the pa14 mockups; the photos are SVG stand-ins.

const CAPS = [
  ['Mia', 'First dance!!'], ['Priya', "Grandma's toast made everyone cry"], ['Hana', 'The cake survived'], ['Jonas', 'Table 4 says hi'],
  ['Hana', 'The arch'], ['Zoe', 'Sparklers'], ['Leo', 'Uncle Pete!!'], ['Sam', null], ['Ana', 'Shoes off, finally'], ['Kai', 'Best seat in the house'],
  ['Mia', 'Rings!'], ['Omar', null], ['Lena', 'The view from table 2'], ['Priya', 'Dessert round two'], ['Tom', 'Confetti everywhere'],
];
const SHAPES = [[4, 3], [3, 4], [4, 3], [1, 1], [3, 4], [16, 9], [4, 3]];
export const wallPhotos = CAPS.map(([by_name, caption], i) => {
  const [w, h] = SHAPES[i % SHAPES.length];
  return { id: `ph${String(i + 1).padStart(2, '0')}`, by_name, creature: (i * 3) % 8, caption, at: new Date(Date.UTC(2026, 10, 21, 18, i * 3)).toISOString(), w: w * 400, h: h * 400, rotate: i === 3 ? 90 : 0, best: [0, 1, 2, 5, 6, 9].includes(i) };
});
const newest = (ps) => [...ps].reverse();

const facts = {
  title: "Mia & Jonas's wedding", host_name: 'Mia', when: '2026-11-21', closes_at: '2026-11-22T23:00:00Z', delete_on: '2026-12-21',
  state: 'open', captions: true, best_set: false,
};
const limits = (mine) => ({ photo_cap_mb: 20, per_guest_max: 30, mine_count: mine.length, caption_max: 80 });
const ME = { id: 'p9', name: 'Sam', avatar: 'antenna', team: null, notices: [] };
const MINE = [
  { id: 'my1', status: 'pending', caption: 'Us at the bar', caption_removed: false, at: '2026-11-21T21:10:00Z', w: 1200, h: 1600, rotate: 0 },
  { id: 'my2', status: 'pending', caption: null, caption_removed: false, at: '2026-11-21T21:11:00Z', w: 1600, h: 1200, rotate: 0 },
  { id: 'ph08', status: 'approved', caption: null, caption_removed: false, at: wallPhotos[7].at, w: 1600, h: 1200, rotate: 0 },
];

/** Upload outcomes in order (cycling). 201 adds a pending photo; the rest are the box's refusals. */
const OK = { status: 201 };
const R = {
  heic: { status: 415, error: 'heic', message: "This photo type can't go up." },
  big: { status: 413, error: 'too_big', limit: 20, message: 'Photos can be up to 20 MB.' },
  slow: { status: 429, error: 'slow_down', retry: 2, message: 'One moment.' },
  max: { status: 409, error: 'guest_max', limit: 30, message: 'That is the most each guest can add.' },
  full: { status: 507, error: 'disk_low', message: 'The wall is full for now.' },
  closed: { status: 410, error: 'closed', message: 'The wall is closed.' },
};

export const wallScenes = {
  // G1 join (and G5 from "I've joined before"): join seats you on the live wall; rejoin brings your photos back.
  join: { info: facts, wall: wallPhotos.slice(0, 9), mine: [], rejoinMine: MINE, uploads: [OK] },
  // G3 the live wall, seated, with two of yours waiting.
  live: { seat: true, info: facts, wall: wallPhotos, mine: MINE, uploads: [OK] },
  // G2 sending then G4: the 2nd file is HEIC, the 3rd too big, the rest go up (one slow_down first).
  refuse: { seat: true, info: facts, wall: wallPhotos.slice(0, 6), mine: [], uploads: [OK, R.heic, R.big, R.slow, OK, OK], delayMs: 900 },
  // G8 page notes: the wall is full / your max.
  full: { seat: true, info: facts, wall: wallPhotos.slice(0, 6), mine: MINE.slice(2), uploads: [R.full] },
  max: { seat: true, info: facts, wall: wallPhotos.slice(0, 6), mine: MINE.slice(2), uploads: [R.max] },
  // Empty wall, seated.
  empty: { seat: true, info: facts, wall: [], mine: [], uploads: [OK] },
  // G6 the best: closed, best set, seated.
  best: { seat: true, info: { ...facts, state: 'closed', best_set: true }, wall: wallPhotos, mine: MINE.slice(2), uploads: [R.closed] },
  // G6 once pa14b ships: Snap's check picked the best set (`best_by: "snap"`), so the Picked sticker is earned.
  'best-snap': { seat: true, info: { ...facts, state: 'closed', best_set: true, best_by: 'snap' }, wall: wallPhotos, mine: MINE.slice(2), uploads: [R.closed] },
  // G7 closed, no seat (no best set).
  closed: { info: { ...facts, state: 'closed' }, wall: wallPhotos, mine: [], uploads: [R.closed] },
  // Uploads close mid-batch: G7 with "N photos didn't go up".
  closing: { seat: true, info: facts, wall: wallPhotos.slice(0, 6), mine: [], uploads: [OK, R.closed] },
  // The host's computer is away: state 503 (the page was served with the public facts).
  away: { away: true, info: facts, wall: [], mine: [] },
  // Ended pocket.
  ended: { info: { ...facts, state: 'closed' }, ended: true, wall: [], mine: [] },
  // TV scenes (screen-wall-<scene>): slides, waiting (no photos), still (same as slides; the walk turns on reduced motion).
  slides: { info: facts, wall: wallPhotos.slice(0, 8), mine: [] },
  waiting: { info: facts, wall: [], mine: [] },
  'tv-ended': { info: facts, ended: true, wall: wallPhotos, mine: [] },
  'tv-away': { away: true, info: facts, wall: [], mine: [] },
};

const COLORS = [['#2b3a67', '#f6b26b', '#7d5ba6'], ['#1f4e5f', '#f9d56e', '#e85a71'], ['#4a2c40', '#ffb6b9', '#61c0bf'], ['#253237', '#9db4c0', '#e0fbfc'], ['#3d2c8d', '#ff8fab', '#ffd166'], ['#16324f', '#f4a259', '#5b8e7d']];
/** An SVG stand-in for a photo: its own colours, its shape (w x h), and its id written on it. */
export function photoSvg(id, p) {
  const n = [...id].reduce((a, c) => a + c.charCodeAt(0), 0);
  const [bg, sun, hill] = COLORS[n % COLORS.length];
  const w = p?.w ?? 1600, h = p?.h ?? 1200;
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${w} ${h}" width="${w}" height="${h}"><rect width="${w}" height="${h}" fill="${bg}"/><circle cx="${w * 0.72}" cy="${h * 0.3}" r="${Math.min(w, h) * 0.14}" fill="${sun}"/><path d="M0 ${h * 0.78} Q ${w * 0.3} ${h * 0.5} ${w * 0.6} ${h * 0.72} T ${w} ${h * 0.66} V ${h} H 0 Z" fill="${hill}"/><text x="${w * 0.06}" y="${h * 0.14}" font-family="sans-serif" font-size="${Math.min(w, h) * 0.08}" fill="#fff" opacity="0.7">${id}</text></svg>`;
}

const copy = (x) => JSON.parse(JSON.stringify(x));

export class WallBox {
  /** `tv`: this is the big screen's box (screen-wall-<scene>). */
  constructor(code, { tv = code.startsWith('screen-') } = {}) {
    this.code = code;
    this.tv = tv;
    this.scene = code.replace(/^screen-wall-|^wall-/, '');
    this.s = wallScenes[tv && wallScenes[`tv-${this.scene}`] ? `tv-${this.scene}` : this.scene];
    this.info = copy(this.s?.info ?? facts);
    this.wall = copy(this.s?.wall ?? []); // approved, oldest first
    this.mine = copy(this.s?.mine ?? []);
    this.version = 1;
    this.n = 0;
    this.sent = []; // every upload the box got: { type, bytes, caption }
    this.writes = [];
  }
  ended() {
    return !!this.s?.ended;
  }
  env(seated = true) {
    const pocket = {
      code: this.code, kit: 'wall', title: this.info.title, state: this.ended() ? 'ended' : 'open', opens_at: null, closes_at: this.info.closes_at,
      ended_at: null, expires_at: '2026-12-21T00:00:00Z', players: 41, max_players: 300, needs_pin: true, retention: { kind: 'after_date', days: 30 },
    };
    if (this.tv) return { api: 'pocket/1', version: this.version, viewer: 'screen', pocket, me: null, view: { title: this.info.title, state: this.info.state, best_set: this.info.best_set, count: this.wall.length, host_name: this.info.screen_host_name, photos: this.wall } };
    if (!seated) return { api: 'pocket/1', version: this.version, viewer: 'anyone', pocket, view: { ...this.info, count: this.wall.length } };
    return { api: 'pocket/1', version: this.version, viewer: 'guest', pocket, me: ME, view: { ...this.info, limits: limits(this.mine), wall: newest(this.wall), mine: newest(this.mine) } };
  }
  /** A push from a test: a function edits the box (approve a photo, hide one), or an object replaces the facts. */
  set(change) {
    if (typeof change === 'function') change(this);
    else if (change) Object.assign(this.info, change);
    this.version++;
  }
  approve(p) {
    this.wall.push(p);
  }
  hide(id) {
    this.wall = this.wall.filter((p) => p.id !== id);
  }
  photo(id) {
    const p = this.wall.find((x) => x.id === id) ?? this.mine.find((x) => x.id === id);
    return p ? photoSvg(id, p) : null;
  }
  /** Upload: raw body, caption in X-Caption. Returns the scene's next outcome. */
  upload({ type, bytes, caption, seated }) {
    if (!seated && !this.s.seat) return { status: 401, body: { error: 'signed_out', message: 'Join first.' } };
    const ups = this.s.uploads ?? [OK];
    const o = ups[this.n++ % ups.length];
    this.sent.push({ type, bytes, caption, status: o.status });
    if (o.status !== 201) return { status: o.status, body: { error: o.error, message: o.message, ...(o.limit ? { limit: o.limit } : {}) }, headers: o.retry ? { 'Retry-After': String(o.retry) } : {} };
    const id = `up${this.n}`;
    this.mine.push({ id, status: 'pending', caption: caption || null, caption_removed: false, at: new Date().toISOString(), w: 1600, h: 1200, rotate: 0 });
    this.version++;
    return { status: 201, body: { ...this.env(), photo: { id } } };
  }
  handle(path, body = {}, { seated: cookie = false, signed = false, method = 'GET' } = {}) {
    // A scene marked `seat` is a phone that joined earlier; otherwise the seat is pa1's kp_ cookie (join sets it).
    const seated = cookie || !!this.s?.seat;
    const [what, query] = path.split('?');
    const err = (status, error, extra = {}) => ({ status, body: { error, message: extra.message ?? 'Fake wall box.', ...extra } });
    if (!this.s) return err(404, 'not_found');
    // The public facts stay answerable while the host is away (pa1 serves them in the page), for the phone and the TV alike.
    if (what === 'info') return { status: 200, body: this.env(false) };
    if (this.s.away) return err(503, 'away');
    if (this.tv) {
      if (what === 'host/signin') {
        if (method !== 'POST') return err(405, 'method');
        if (this.ended()) return err(410, 'ended');
        return { status: 200, body: this.env(), cookie: true };
      }
      if (what === 'host/state') {
        if (this.ended()) return err(410, 'ended');
        if (!signed) return err(401, 'signed_out');
        const since = Number(new URLSearchParams(query).get('since'));
        if (since && since === this.version) return { status: 204, body: null };
        return { status: 200, body: this.env() };
      }
      this.writes.push(`${method} ${path}`);
      return err(403, 'screen_only');
    }
    if (what === 'state') {
      if (this.ended()) return err(410, 'ended');
      if (!seated) return err(401, 'signed_out');
      const since = Number(new URLSearchParams(query).get('since'));
      if (since && since === this.version) return { status: 204, body: null };
      return { status: 200, body: this.env(true) };
    }
    if (what === 'join') {
      if (this.info.state !== 'open') return err(410, 'closed', { message: 'The wall is closed.' });
      if (!/^\d{4}$/.test(body.pin ?? '')) return err(400, 'pin_required', { message: 'Pick a 4-digit PIN.' });
      this.version++;
      return { status: 201, body: this.env(true) };
    }
    if (what === 'rejoin') {
      if (body.pin === '0000') return err(401, 'wrong_pin', { message: "That name and PIN don't match.", tries_left: 4 });
      if (this.s.rejoinMine) this.mine = copy(this.s.rejoinMine);
      this.version++;
      return { status: 200, body: this.env(true) };
    }
    if (what === 'notices/seen') return { status: 204, body: null };
    if (what === 'action') {
      if (!seated) return err(401, 'signed_out');
      if (body.name === 'caption') {
        const p = this.mine.find((x) => x.id === body.body?.id);
        if (!p || p.status !== 'pending') return err(409, 'not_now', { message: 'That photo is already on the wall.' });
        p.caption = body.body.caption || null;
        this.version++;
        return { status: 200, body: this.env(true) };
      }
      return err(400, 'bad_action');
    }
    return err(404, 'not_found');
  }
}
