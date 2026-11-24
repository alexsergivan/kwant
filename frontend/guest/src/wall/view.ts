// The Wall's guest and big-screen views (pa14), read from one Pocket API v1 state: pocket and me from pa1, the kit's
// view from Juno's src/pocket/wall.rs (guest_view, lobby, screen_view). Values are copied or left out, never worked
// out; anything with the wrong shape is dropped, so junk from the box can't draw a wrong screen. The guest view holds
// approved photos (`wall`) and this guest's own (`mine`); nobody else's waiting photo, no file size, no hidden count.

export interface WallPhoto {
  id: string;
  by_name: string;
  /** The guest's creature, 0-7 (wall.rs `creature`, a hash of the player id). */
  creature: number;
  caption?: string;
  at?: string;
  w?: number;
  h?: number;
  rotate: 0 | 90 | 180 | 270;
  best: boolean;
}

export interface MinePhoto {
  id: string;
  status: 'pending' | 'approved';
  caption?: string;
  caption_removed: boolean;
  at?: string;
  w?: number;
  h?: number;
  rotate: 0 | 90 | 180 | 270;
}

export interface Limits {
  photo_cap_mb?: number;
  per_guest_max?: number;
  mine_count?: number;
  caption_max: number;
}

export interface Me {
  id: string;
  name: string;
  notices: { kind: 'wrong_pin' | 'pin_reset'; at: string }[];
}

export interface WallView {
  title: string;
  ended: boolean;
  /** pocket.expires_at, else the kit's delete_on: the day everything is deleted. */
  deletes_on?: string;
  host_name?: string;
  when?: string;
  closes_at?: string;
  open: boolean;
  captions: boolean;
  best_set: boolean;
  /** Who picked the best set when a check did it (pa14b: "snap"). Absent while the host picks by hand. */
  best_by?: string;
  /** Approved photos on the wall (lobby `count` before a seat). */
  count?: number;
  limits: Limits;
  me?: Me;
  /** Approved photos, newest first. */
  wall: WallPhoto[];
  /** This guest's own photos (waiting or approved), newest first. */
  mine: MinePhoto[];
}

type Obj = Record<string, unknown>;
const obj = (x: unknown): Obj | undefined => (x && typeof x === 'object' && !Array.isArray(x) ? (x as Obj) : undefined);
const str = (x: unknown): string | undefined => (typeof x === 'string' ? x : undefined);
const text = (x: unknown): string | undefined => (typeof x === 'string' && x.trim() ? x : undefined);
const count = (x: unknown): number | undefined => (typeof x === 'number' && Number.isInteger(x) && x >= 0 ? x : undefined);
const list = (x: unknown): unknown[] => (Array.isArray(x) ? x : []);
const turn = (x: unknown): 0 | 90 | 180 | 270 => (x === 90 || x === 180 || x === 270 ? x : 0);
const size = (x: unknown): number | undefined => (typeof x === 'number' && x > 0 && Number.isFinite(x) ? x : undefined);
const ID = /^[\w-]{1,64}$/;

export function wallPhotoFrom(x: unknown): WallPhoto | undefined {
  const p = obj(x);
  const id = str(p?.id);
  const by = text(p?.by_name);
  if (!id || !ID.test(id) || !by) return undefined;
  const c = count(p?.creature);
  return { id, by_name: by, creature: c === undefined ? 0 : c % 8, caption: text(p?.caption), at: str(p?.at), w: size(p?.w), h: size(p?.h), rotate: turn(p?.rotate), best: p?.best === true };
}

function minePhotoFrom(x: unknown): MinePhoto | undefined {
  const p = obj(x);
  const id = str(p?.id);
  const status = p?.status;
  // A hidden one of yours never shows (it is dropped quietly, For Iris 4).
  if (!id || !ID.test(id) || (status !== 'pending' && status !== 'approved')) return undefined;
  return { id, status, caption: text(p?.caption), caption_removed: p?.caption_removed === true, at: str(p?.at), w: size(p?.w), h: size(p?.h), rotate: turn(p?.rotate) };
}

const day = (x: unknown): string | undefined => {
  const s = str(x);
  return s && /^\d{4}-\d{2}-\d{2}/.test(s) ? s : undefined;
};

/** One v1 state (boot JSON, info, state, join, rejoin, the upload's 201, a ws frame) as the Wall's guest view. */
export function wallFrom(json: unknown): { view: WallView; version?: number } | undefined {
  const j = obj(json);
  const pocket = obj(j?.pocket);
  if (!j || !pocket || str(pocket.kit) !== 'wall') return undefined;
  const kv = obj(j.view) ?? {};
  const m = obj(j.me);
  const me: Me | undefined =
    m && text(m.name)
      ? {
          id: str(m.id) ?? '',
          name: str(m.name)!,
          notices: list(m.notices)
            .map(obj)
            .filter((n): n is Obj => !!n && (n.kind === 'wrong_pin' || n.kind === 'pin_reset'))
            .map((n) => ({ kind: n.kind as 'wrong_pin' | 'pin_reset', at: str(n.at) ?? '' })),
        }
      : undefined;
  const lim = obj(kv.limits);
  const ended = str(pocket.state) === 'ended';
  const wall = me ? list(kv.wall).map(wallPhotoFrom).filter((p): p is WallPhoto => !!p) : [];
  const view: WallView = {
    title: text(kv.title) ?? str(pocket.title) ?? '',
    ended,
    deletes_on: day(pocket.expires_at) ?? day(kv.delete_on),
    host_name: text(kv.host_name),
    when: day(kv.when),
    closes_at: str(kv.closes_at),
    open: !ended && kv.state === 'open',
    captions: kv.captions !== false,
    best_set: kv.best_set === true,
    best_by: text(kv.best_by),
    count: count(kv.count),
    limits: {
      photo_cap_mb: count(lim?.photo_cap_mb) || undefined,
      per_guest_max: count(lim?.per_guest_max) || undefined,
      mine_count: count(lim?.mine_count),
      caption_max: count(lim?.caption_max) || 80,
    },
    me,
    wall,
    mine: me ? list(kv.mine).map(minePhotoFrom).filter((p): p is MinePhoto => !!p) : [],
  };
  const version = typeof j.version === 'number' && Number.isFinite(j.version) ? j.version : undefined;
  return { view, version };
}

// ---------- the big screen ----------

export interface TvView {
  title: string;
  ended: boolean;
  open: boolean;
  best_set: boolean;
  count?: number;
  /** The host's first name, once the screen view carries it (asked of Juno); else the screen says "the host". */
  host_name?: string;
  /** Approved photos, oldest first (the slideshow loops). */
  photos: WallPhoto[];
}

/** One v1 screen state (host/signin, host/state?view=screen, a ws frame) as the Wall's big-screen view. */
export function tvFrom(json: unknown): { view: TvView; version?: number } | undefined {
  const j = obj(json);
  const pocket = obj(j?.pocket);
  const v = obj(j?.view);
  if (!j || !pocket || !v || str(pocket.kit) !== 'wall') return undefined;
  const ended = str(pocket.state) === 'ended';
  const view: TvView = {
    title: text(v.title) ?? str(pocket.title) ?? '',
    ended,
    open: !ended && v.state === 'open',
    best_set: v.best_set === true,
    count: count(v.count),
    host_name: text(v.host_name),
    photos: list(v.photos).map(wallPhotoFrom).filter((p): p is WallPhoto => !!p),
  };
  const version = typeof j.version === 'number' && Number.isFinite(j.version) ? j.version : undefined;
  return { view, version };
}

// ---------- screens ----------

export type Screen = 'join' | 'rejoin' | 'wall' | 'best' | 'closed';
/** What the guest picked to see: the page's own choice, the Best screen (G6) or every photo. */
export type Pick = 'auto' | 'best' | 'all';

/**
 * Which screen a view draws. `rejoin`: the guest tapped "I've joined before". `pick`: "See the best" (G7) or "See all"
 * (G6). The guest's sending rows (G2, G4) are drawn on the wall screen, above the photos. The Best screen needs both
 * the server's `best_set` and at least one photo it marked `best`.
 */
export function screenOf(v: WallView, o: { rejoin?: boolean; pick?: Pick } = {}): Screen {
  if (!v.me) {
    if (v.ended || !v.open) return o.rejoin && !v.ended ? 'rejoin' : 'closed';
    return o.rejoin ? 'rejoin' : 'join';
  }
  if (o.pick === 'best' && hasBest(v)) return 'best';
  if (o.pick === 'all') return 'wall';
  if (v.ended || !v.open) return 'closed';
  return 'wall';
}

export const hasBest = (v: WallView): boolean => v.best_set && v.wall.some((p) => p.best);

/**
 * The wall's photos as the guest page shows them: what the page has already drawn stays put, new ones wait behind the
 * "N new photos" pill (never pushing the page, For Iris 5), and the list grows 12 at a time.
 * `shown` is the ids drawn so far (empty on the first draw, which shows the newest `PAGE`).
 */
export const PAGE = 12;

export function wallPage(wall: WallPhoto[], shown: readonly string[], limit: number): { photos: WallPhoto[]; fresh: WallPhoto[]; more: boolean } {
  if (!shown.length) return { photos: wall.slice(0, limit), fresh: [], more: wall.length > limit };
  const known = new Set(shown);
  const firstKnown = wall.findIndex((p) => known.has(p.id));
  // Everything newer than the newest photo already drawn is "new"; a hidden one simply drops out.
  const fresh = firstKnown < 0 ? [] : wall.slice(0, firstKnown);
  const rest = firstKnown < 0 ? wall : wall.slice(firstKnown);
  return { photos: rest.slice(0, limit), fresh, more: rest.length > limit };
}

// ---------- refusals ----------

export type Refusal = { kind: 'row'; title: string; fix?: string; again?: true } | { kind: 'page'; title: string; sub?: string } | { kind: 'closed' } | { kind: 'wait'; secs: number } | { kind: 'retry' };

/**
 * Plain words for a refused upload (design.md "For Iris" 3, in wall.rs's codes). Numbers come from the reply's
 * `limit` or the view's limits; when neither is there, the sentence leaves the number out.
 */
export function refusal(status: number, error: string | undefined, o: { limit?: number; host?: string; capMb?: number; perGuest?: number; retryAfter?: number } = {}): Refusal {
  const host = o.host || 'the host';
  if (status === 0 || (status >= 500 && status !== 507)) return { kind: 'retry' };
  if (status === 429 || error === 'slow_down') return { kind: 'wait', secs: o.retryAfter && o.retryAfter > 0 ? Math.min(o.retryAfter, 60) : 3 };
  if (status === 410 || error === 'closed' || error === 'ended') return { kind: 'closed' };
  if (error === 'too_big' || status === 413) {
    const mb = o.limit ?? o.capMb;
    return { kind: 'row', title: mb ? `Too big: photos can be up to ${mb} MB.` : 'Too big for this wall.', fix: 'Try a smaller copy, or a screenshot of it.' };
  }
  if (error === 'heic') return { kind: 'row', title: "This photo type can't go up.", fix: 'Try sharing it from Photos, which makes a JPEG.' };
  if (error === 'not_photo') return { kind: 'row', title: 'Not a photo: this wall takes photos only.', fix: 'JPEG, PNG or WebP.' };
  // The body stopped mid-upload (server-impl.md: like a network error), so the row offers Try again.
  if (error === 'broken_photo') return { kind: 'row', title: "That photo didn't arrive whole.", fix: 'Tap Try again to send it once more.', again: true };
  if (error === 'guest_max') {
    const n = o.limit ?? o.perGuest;
    return { kind: 'page', title: n ? `You've added ${n} photos, the most each guest can.` : "You've added the most photos each guest can.", sub: 'Photos you already added are safe.' };
  }
  if (error === 'wall_full' || error === 'disk_low' || status === 507) return { kind: 'page', title: `The wall is full for now. ${cap(host)} can make room.`, sub: 'Photos you already added are safe.' };
  if (error === 'bad_caption') return { kind: 'row', title: "That caption can't go up.", fix: 'Change the caption and try again.' };
  return { kind: 'row', title: "That photo didn't go up.", fix: 'Tap Try again.', again: true };
}

export const cap = (s: string): string => s.charAt(0).toUpperCase() + s.slice(1);
