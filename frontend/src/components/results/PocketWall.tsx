// Pocket apps (pa14): the Wall's Pocket card in chat, states 1 Ready, 2 Live waiting, 3 Hide with Undo, 4 Live on
// the wall, 5 Space almost full, 6 Closed picking, 7 Best 30 picked, 8 Best 30 shown, plus the Done row and the one
// Needs-you item per wall (pa14/design.md §1, §4, §5, "For Pixel" 1-9). The block carries Juno's host summary
// (counts, MB and photo ids only); the photo lists come from the box when the card opens (`host_view`) and from
// every action's answer. Approve and Hide are optimistic and roll back with a plain line. Holo once, only when
// earned: "Picked" when Snap's check picked the best set and the host showed it (`best_set` + `best_by: "snap"`).

import React, { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import {
  AlertTriangle,
  Check,
  ChevronDown,
  Copy,
  Download,
  Eye,
  EyeOff,
  HardDrive,
  ImageOff,
  Images,
  Loader2,
  Maximize2,
  Pencil,
  RotateCcw,
  ShieldCheck,
  Sparkles,
  Square,
  Star,
  Trash2,
  Upload,
  Users,
  Zap,
} from 'lucide-react';
import type { PocketBlock, WallInfo } from '../../smartResults';
import { isIndex, isNum, isObj, isText } from '../../smartResults';
import { Sticker } from '../Sticker';
import { GUEST_CREATURES, ModernAvatar, guestCreature, renderCreature } from '../ModernAvatar';
import type { Mood } from '../../kwantMood';
import { linkUrl, pocketApi, PocketApiError, wallPhotoUrl, wallZipUrl } from '../../pocketApi';
import { useMedia } from '../../useMedia';
import { useFocusAfterRender } from '../../focusAfterRender';
import { BTN, Bar, Callout, GHOST, HALF, ICON, LATER_CLS, LaterButton, PRIMARY, Row, STOP } from './pocketParts';

// ---------------------------------------------------------------------------------------------------
// Validator

const isCount = (v: unknown): v is number => isIndex(v) && v <= 1_000_000;
const isMb = (v: unknown): v is number => isNum(v) && v >= 0 && v <= 10_000_000;
const PHOTO_ID = /^[A-Za-z0-9_-]{1,64}$/;
/** Keys a host summary never has: a guest id, a file size or a photo list in the chat block (design "Rules"). */
export const WALL_FORBIDDEN = ['by', 'bytes', 'guest', 'guest_id', 'player_id', 'players', 'queue', 'wall', 'photos', 'pin'];
const FORBIDDEN = new Set(WALL_FORBIDDEN);
function findForbidden(v: unknown, depth = 0): string | null {
  if (depth > 6) return null;
  if (Array.isArray(v)) {
    for (const x of v) {
      const hit = findForbidden(x, depth + 1);
      if (hit) return hit;
    }
    return null;
  }
  if (!isObj(v)) return null;
  for (const [k, x] of Object.entries(v)) {
    if (FORBIDDEN.has(k)) return k;
    const hit = findForbidden(x, depth + 1);
    if (hit) return hit;
  }
  return null;
}

const optStr = (v: unknown, max: number): string | null | undefined => (v === undefined || v === null ? null : isText(v, max) ? v : undefined);

/**
 * The `wall` object of a Pocket block (card-impl.md "Shape I read"). Counts and MB must be right or the card
 * doesn't show; display text, limits and pa14b fields fall back. Any guest id, file size or photo list rejects it.
 */
export function validWall(v: unknown): WallInfo | undefined {
  if (!isObj(v) || findForbidden(v)) return undefined;
  if (v.state !== 'open' && v.state !== 'closed') return undefined;
  const c = v.counts;
  if (!isObj(c) || !['total', 'waiting', 'approved', 'hidden', 'guests', 'senders'].every((k) => isCount(c[k]))) return undefined;
  const counts = { total: c.total, waiting: c.waiting, approved: c.approved, hidden: c.hidden, guests: c.guests, senders: c.senders } as WallInfo['counts'];
  if (counts.waiting + counts.approved + counts.hidden !== counts.total || counts.senders > counts.guests) return undefined;
  if (!isMb(v.used_mb) || !isMb(v.pocket_cap_mb) || v.pocket_cap_mb <= 0) return undefined;
  const best = v.best ?? [];
  if (!Array.isArray(best) || best.some((id) => typeof id !== 'string' || !PHOTO_ID.test(id))) return undefined;
  const bestMax = v.best_max === undefined || v.best_max === null ? 30 : v.best_max;
  if (!isCount(bestMax) || bestMax < 1 || best.length > bestMax) return undefined;
  if (v.best_set !== undefined && typeof v.best_set !== 'boolean') return undefined;
  const best_set = v.best_set === true;
  if (best_set && best.length === 0) return undefined;
  const closes_at = optStr(v.closes_at, 60);
  const delete_on = optStr(v.delete_on, 60);
  const host_name = optStr(v.host_name, 40);
  const best_by = optStr(v.best_by, 20);
  if ([closes_at, delete_on, host_name, best_by].includes(undefined)) return undefined;
  return {
    state: v.state,
    closes_at: closes_at ?? null,
    delete_on: delete_on ?? null,
    host_name: host_name ?? null,
    captions: v.captions !== false,
    best_set,
    best_by: best_by ?? null,
    counts,
    used_mb: v.used_mb,
    pocket_cap_mb: v.pocket_cap_mb,
    best: best as string[],
    best_max: bestMax,
    keep_days: isCount(v.keep_days) && v.keep_days > 0 ? v.keep_days : 30,
    photo_cap_mb: isMb(v.photo_cap_mb) ? v.photo_cap_mb : null,
    per_guest_max: isCount(v.per_guest_max) ? v.per_guest_max : null,
    picking: v.picking === true && v.state === 'closed' && best.length === 0,
  };
}

/** A photo in the host view (server-impl.md "HostPhoto"); only what the card shows is kept. */
export interface WallPhoto {
  id: string;
  by: string;
  by_name: string;
  creature: number;
  caption: string | null;
  at: string | null;
  bytes: number;
  w: number;
  h: number;
  rotate: 0 | 90 | 180 | 270;
}
export interface WallView {
  queue: WallPhoto[];
  wall: WallPhoto[];
  /** The summary numbers the same answer carried, to keep the card's counts current. */
  counts: WallInfo['counts'] | null;
  used_mb: number | null;
  best: string[] | null;
  best_set: boolean | null;
}

function validPhoto(v: unknown): WallPhoto | null {
  if (!isObj(v) || typeof v.id !== 'string' || !PHOTO_ID.test(v.id) || typeof v.by !== 'string' || !isText(v.by_name, 80)) return null;
  const rotate = [0, 90, 180, 270].includes(v.rotate as number) ? (v.rotate as WallPhoto['rotate']) : 0;
  return {
    id: v.id,
    by: v.by,
    by_name: v.by_name,
    creature: isIndex(v.creature) ? v.creature % 8 : 0,
    caption: isText(v.caption, 200) ? v.caption : null,
    at: typeof v.at === 'string' ? v.at : null,
    bytes: isCount(v.bytes) || (isNum(v.bytes) && v.bytes >= 0) ? (v.bytes as number) : 0,
    w: isIndex(v.w) && v.w > 0 ? v.w : 4,
    h: isIndex(v.h) && v.h > 0 ? v.h : 3,
    rotate,
  };
}

/** The `host_view` of a read or an action; null when it isn't a wall's. Bad photos are dropped, not fatal. */
export function validWallView(v: unknown): WallView | null {
  if (!isObj(v) || !Array.isArray(v.queue) || !Array.isArray(v.wall)) return null;
  const list = (a: unknown[]) => a.map(validPhoto).filter((p): p is WallPhoto => !!p);
  const c = v.counts;
  const counts = isObj(c) && ['total', 'waiting', 'approved', 'hidden', 'guests', 'senders'].every((k) => isCount(c[k])) ? (c as unknown as WallInfo['counts']) : null;
  const best = Array.isArray(v.best) && v.best.every((id) => typeof id === 'string' && PHOTO_ID.test(id)) ? (v.best as string[]) : null;
  return {
    queue: list(v.queue),
    wall: list(v.wall),
    counts: counts ? { total: counts.total, waiting: counts.waiting, approved: counts.approved, hidden: counts.hidden, guests: counts.guests, senders: counts.senders } : null,
    used_mb: isMb(v.used_mb) ? v.used_mb : null,
    best,
    best_set: typeof v.best_set === 'boolean' ? v.best_set : null,
  };
}

// ---------------------------------------------------------------------------------------------------
// Words and states

export type WallStage = 'ready' | 'waiting' | 'caught_up' | 'space' | 'picking' | 'closed' | 'picked' | 'shown';
/** Space is "almost full" at 90% of the cap while guests can still add photos. */
export const spaceLow = (w: Pick<WallInfo, 'used_mb' | 'pocket_cap_mb'>) => w.used_mb >= w.pocket_cap_mb * 0.9;

export function wallStage(w: WallInfo): WallStage {
  if (w.state === 'open') {
    if (spaceLow(w)) return 'space';
    if (w.counts.waiting > 0) return 'waiting';
    return w.counts.total === 0 ? 'ready' : 'caught_up';
  }
  if (w.best_set) return 'shown';
  if (w.best.length > 0) return 'picked';
  return w.picking ? 'picking' : 'closed';
}

/** design.md §5 Moods: idle ready, working caught up, needs_you waiting or space, thinking picking, finished best set. */
export const SNAP_MOOD: Record<WallStage, Mood> = {
  ready: 'idle',
  waiting: 'needs_you',
  caught_up: 'working',
  space: 'needs_you',
  picking: 'thinking',
  closed: 'finished',
  picked: 'finished',
  shown: 'finished',
};
const SNAP_SAYS: Record<Mood, string> = {
  idle: 'Snap is ready for the first photo',
  needs_you: 'Snap needs you',
  working: 'Snap is minding the wall',
  thinking: 'Snap is picking the best photos',
  finished: 'Snap is done',
  failed: 'Snap hit a problem',
  resting: 'Snap is resting',
};

/** The one holo on this card: the best set is shown and Snap's check picked it (server-impl.md 9). */
export const earnedPicked = (w: Pick<WallInfo, 'best_set' | 'best_by'>) => w.best_set && w.best_by === 'snap';

const NUM = new Intl.NumberFormat('en');
const num = (n: number) => NUM.format(Math.round(n));
const usd = (n: number) => `$${n < 0.01 && n > 0 ? '0.01' : n.toFixed(2)}`;
const plural = (n: number, one: string, many = `${one}s`) => `${num(n)} ${n === 1 ? one : many}`;
const possessive = (name: string) => (/s$/i.test(name) ? `${name}'` : `${name}'s`);

export const WALL_LATER = {
  twoGb: "The box hasn't said there's room for 2 GB yet",
  swap: 'Swapping photos comes with the star toggles in a later update',
  noLink: 'The link shows here once your box has one',
} as const;

export function wallBudgetLine(b: Pick<PocketBlock, 'budget'>): string {
  const tail = ' Uploads, approving and the slideshow use none.';
  if (!b.budget) return `Bot calls so far aren't counted yet.${tail}`;
  return `About ${plural(b.budget.calls, 'bot call')} to set this up, about ${usd(b.budget.usd)}.${tail}`;
}

export function keepLine(w: WallInfo): string {
  return w.delete_on
    ? `Photos go to you. They leave this box on ${w.delete_on} (${w.keep_days} days after the party): download them before then.`
    : `Photos go to you. They leave this box ${w.keep_days} days after the party: download them before then.`;
}

export function meterLine(w: WallInfo): string {
  const limits = [w.photo_cap_mb !== null ? `up to ${num(w.photo_cap_mb)} MB a photo` : null, w.per_guest_max !== null ? `${num(w.per_guest_max)} a guest` : null].filter(Boolean);
  return `${num(w.used_mb)} MB of ${num(w.pocket_cap_mb)} MB photo space used${limits.length ? ` · ${limits.join(', ')}` : ''}`;
}

/** "About 20 photos left" from the average photo so far, rounded to tens past 10. */
export function photosLeft(w: WallInfo): number {
  const left = Math.max(0, w.pocket_cap_mb - w.used_mb);
  const avg = w.counts.total > 0 ? w.used_mb / w.counts.total : w.photo_cap_mb ?? 4;
  const n = avg > 0 ? Math.floor(left / avg) : 0;
  return n > 10 ? Math.round(n / 10) * 10 : n;
}

/** One Needs-you item per wall while photos wait, never one per photo; the latest card of each wall wins. */
export function wallNeedsYou(blocks: PocketBlock[]): Array<{ code: string; name: string; waiting: number; text: string }> {
  const latest = new Map<string, PocketBlock>();
  for (const b of blocks) if (b.kit === 'wall' && b.wall) latest.set(b.code, b);
  return [...latest.values()]
    .filter((b) => b.wall!.counts.waiting > 0)
    .map((b) => ({ code: b.code, name: b.name, waiting: b.wall!.counts.waiting, text: `${plural(b.wall!.counts.waiting, 'photo')} waiting on ${b.name}` }));
}

const timeOf = (iso: string | null) => {
  if (!iso) return null;
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? null : d.toLocaleTimeString('en', { hour: 'numeric', minute: '2-digit' });
};
const mb = (bytes: number) => `${(bytes / 1_048_576).toFixed(1)} MB`;
const firstName = (name: string) => name.split(/\s+/)[0] || name;

// ---------------------------------------------------------------------------------------------------
// Small parts

const CREATURE_IDS = Object.keys(GUEST_CREATURES);
/** A guest's creature from the box's `creature` (0-7), in the same order as the guest page (Iris's AVATARS). */
export const WallCreature: React.FC<{ n: number; size?: number }> = ({ n, size = 22 }) => {
  const id = CREATURE_IDS[n % CREATURE_IDS.length];
  return (
    <svg viewBox="5 2.5 16 17" width={size} height={size} aria-hidden="true" data-creature={id} className="shrink-0" style={{ imageRendering: 'pixelated', shapeRendering: 'crispEdges' }}>
      {renderCreature(guestCreature(id)).filter((r) => (r as React.ReactElement).key !== 'shadow')}
    </svg>
  );
};

/** Snap in the kit tile, with the wall's mood. */
const SnapTile: React.FC<{ mood: Mood }> = ({ mood }) => (
  <span className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl border border-sky-700 bg-sky-950" data-snap={mood}>
    <ModernAvatar role="snap" name="Snap" size={36} mood={mood} moodText={SNAP_SAYS[mood]} modelBadge={false} />
  </span>
);

const LIVE = 'kw-chip border-sky-400 bg-sky-600 font-semibold text-white';
const WallChip: React.FC<{ w: WallInfo; stage: WallStage; waiting: number }> = ({ w, stage, waiting }) => {
  switch (stage) {
    case 'ready':
      return <span className="kw-chip kw-chip-info">Ready · no photos yet</span>;
    case 'waiting':
    case 'caught_up':
      // White on sky-600 is 5.1:1, like the quiz's Live chip.
      return (
        <span className={LIVE} data-chip="live">
          <span className="kw-status-dot bg-white" aria-hidden="true" />
          {waiting > 0 ? `Live · ${num(waiting)} waiting` : 'Live'}
        </span>
      );
    case 'space':
      return (
        <span className="kw-chip kw-chip-warn" data-chip="space">
          <AlertTriangle size={12} aria-hidden="true" />
          Space almost full
        </span>
      );
    case 'picking':
    case 'closed':
      return <span className="kw-chip">{w.closes_at ? `Closed · ${w.closes_at}` : 'Closed'}</span>;
    default:
      return <span className="kw-chip">Closed</span>;
  }
};

const Tile: React.FC<{ n: number; label: string; on?: boolean }> = ({ n, label, on }) => (
  <div className={`flex min-w-0 flex-col rounded-xl border px-3 py-2 ${on ? 'border-sky-600 bg-sky-500/[0.06]' : 'border-hairline bg-surface-rail/40'}`} data-tile={label} data-on={on || undefined}>
    <span className="text-[22px] font-bold leading-7 tabular-nums text-zinc-50 sm:text-[20px]">{num(n)}</span>
    <span className="text-[13px] leading-4 text-zinc-300 sm:text-meta sm:leading-5">{label}</span>
  </div>
);

/** A full photo turned as the box says (no thumbnails yet), lazily, cropped to its box. */
const Photo: React.FC<{ code: string; p: WallPhoto; ratio?: number; className?: string }> = ({ code, p, ratio = 4 / 3, className = '' }) => {
  const [failed, setFailed] = useState(false);
  const side = p.rotate === 90 || p.rotate === 270;
  const alt = `Photo from ${firstName(p.by_name)}${p.caption ? `: ${p.caption}` : ''}`;
  return (
    <span className={`relative block overflow-hidden rounded-[10px] bg-surface-main ${className}`} data-photo={p.id}>
      {failed ? (
        <span className="absolute inset-0 grid place-items-center text-zinc-500" role="img" aria-label={`${alt} (didn't load)`}>
          <ImageOff size={18} aria-hidden="true" />
        </span>
      ) : (
        <img
          src={wallPhotoUrl(code, p.id)}
          alt={alt}
          loading="lazy"
          decoding="async"
          onError={() => setFailed(true)}
          className="absolute left-1/2 top-1/2 max-w-none object-cover"
          style={{
            imageOrientation: 'none',
            // A side-turned photo swaps its box before the turn, so it still covers the frame.
            width: side ? `${100 / ratio}%` : '100%',
            height: side ? `${100 * ratio}%` : '100%',
            transform: `translate(-50%, -50%) rotate(${p.rotate}deg)`,
          }}
        />
      )}
    </span>
  );
};

const Picked: React.FC<{ n: number }> = ({ n }) => (
  <Sticker tone="holo" size="sm" ariaLabel={`Picked: Snap's best ${n}, shown to guests`}>
    Picked
  </Sticker>
);

// ---------------------------------------------------------------------------------------------------
// The approval queue and the wall grid

type Pending = Record<string, 'approve' | 'hide'>;
type Undo = { id: string; name: string; left: number } | null;
export const UNDO_MS = 5000;
const FAIL = "Couldn't reach the box. Try again";
const QUEUE_SHOWN = 5;
const GRID_SHOWN = 15;

const QueueRow: React.FC<{ code: string; p: WallPhoto; captions: boolean; err: string | null; leaving?: boolean; onApprove: () => void; onHide: () => void }> = ({ code, p, captions, err, leaving, onApprove, onHide }) => {
  const at = timeOf(p.at);
  return (
    <li
      className={`wall-row grid grid-cols-[88px_minmax(0,1fr)] items-center gap-3 border-b border-hairline px-3.5 py-2.5 last:border-b-0 sm:grid-cols-[96px_minmax(0,1fr)_auto] motion-safe:transition-[opacity,transform] motion-safe:duration-[160ms] motion-safe:ease-[var(--kw-ease)] ${leaving ? 'pointer-events-none -translate-y-2 opacity-0 motion-reduce:hidden' : ''}`}
      data-row={p.id}
      data-leaving={leaving || undefined}
      aria-hidden={leaving || undefined}
      {...(leaving ? { inert: '' } : {})}
    >
      <Photo code={code} p={p} className="h-[66px] w-[88px] sm:h-[72px] sm:w-24" />
      <div className="min-w-0 text-[14px] leading-5 text-zinc-300 sm:text-[13px] sm:leading-[19px]">
        <div className="truncate">
          <b className="font-semibold text-zinc-50">{firstName(p.by_name)}</b>
          {at && ` · ${at}`} · {mb(p.bytes)}
        </div>
        {captions && (p.caption ? <q className="block truncate text-zinc-100">{p.caption}</q> : <span className="block text-zinc-400">No caption</span>)}
        {err && (
          <p role="alert" className="mt-0.5 flex items-center gap-1 text-bad" data-row-error>
            <AlertTriangle size={13} aria-hidden="true" />
            {err}
          </p>
        )}
      </div>
      <div className="col-span-2 grid grid-cols-2 gap-2 sm:col-span-1 sm:flex">
        <button type="button" onClick={onApprove} className={`inline-flex items-center justify-center gap-1.5 rounded-xl border border-teal-400/45 bg-teal-400/[0.12] px-3 font-semibold text-ok hover:bg-teal-400/20 kw-focus ${BTN}`} data-action="approve" aria-label={`Approve ${firstName(p.by_name)}'s photo`}>
          <Check size={14} aria-hidden="true" />
          Approve
        </button>
        <button type="button" onClick={onHide} className={`kw-btn-ghost ${HALF} text-zinc-200`} data-action="hide" aria-label={`Hide ${firstName(p.by_name)}'s photo`}>
          <EyeOff size={14} aria-hidden="true" />
          Hide
        </button>
      </div>
    </li>
  );
};

const UndoRow: React.FC<{ u: NonNullable<Undo>; onUndo: () => void }> = ({ u, onUndo }) => {
  const undo = useRef<HTMLButtonElement | null>(null);
  useEffect(() => undo.current?.focus({ preventScroll: true }), []);
  return (
    <li className="flex items-center gap-3 border-b border-hairline bg-surface-raised px-3.5 py-2.5 last:border-b-0" data-undo={u.id}>
      <p role="status" className="min-w-0 flex-1 text-[14px] leading-5 text-zinc-200 sm:text-[13px]">
        Hid {possessive(u.name)} photo. It's deleted in {u.left} s, and nobody else saw it.
      </p>
      <button ref={undo} type="button" onClick={onUndo} className={`${GHOST} shrink-0`} data-action="undo">
        <RotateCcw size={14} aria-hidden="true" />
        Undo
      </button>
    </li>
  );
};

type Group = { by: string; name: string; creature: number; photos: WallPhoto[] };
const groupsOf = (queue: WallPhoto[]): Group[] => {
  const map = new Map<string, Group>();
  for (const p of queue) {
    const g = map.get(p.by) ?? { by: p.by, name: firstName(p.by_name), creature: p.creature, photos: [] };
    g.photos.push(p);
    map.set(p.by, g);
  }
  return [...map.values()];
};

const Tabs: React.FC<{ tab: 'waiting' | 'wall'; setTab: (t: 'waiting' | 'wall') => void; waiting: number; approved: number; ids: [string, string] }> = ({ tab, setTab, waiting, approved, ids }) => {
  const refs = useRef<Array<HTMLButtonElement | null>>([]);
  const order: Array<'waiting' | 'wall'> = ['waiting', 'wall'];
  const onKey = (e: React.KeyboardEvent) => {
    if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
    e.preventDefault();
    const next = order[(order.indexOf(tab) + 1) % 2];
    setTab(next);
    refs.current[order.indexOf(next)]?.focus();
  };
  return (
    <div role="tablist" aria-label="Photos" onKeyDown={onKey} className="flex gap-1 border-b border-hairline p-1.5">
      {order.map((t, i) => {
        const on = t === tab;
        return (
          <button
            key={t}
            ref={(el) => (refs.current[i] = el)}
            type="button"
            role="tab"
            id={`${ids[i]}-tab`}
            aria-selected={on}
            aria-controls={ids[i]}
            tabIndex={on ? 0 : -1}
            onClick={() => setTab(t)}
            className={`inline-flex min-h-[44px] flex-1 items-center justify-center gap-1.5 rounded-lg text-[14px] font-semibold kw-focus sm:min-h-[36px] sm:text-[13px] ${on ? 'bg-surface-raised text-zinc-50 ring-1 ring-inset ring-sky-700' : 'text-zinc-300 hover:text-zinc-100'}`}
            data-tab={t}
          >
            {t === 'waiting' ? `Waiting · ${num(waiting)}` : `On the wall · ${num(approved)}`}
          </button>
        );
      })}
    </div>
  );
};

const WallGrid: React.FC<{ code: string; photos: WallPhoto[]; captions: boolean; errs: Record<string, string>; onHide: (p: WallPhoto) => void }> = ({ code, photos, captions, errs, onHide }) => {
  const [all, setAll] = useState(false);
  const shown = all ? photos : photos.slice(0, GRID_SHOWN);
  const err = Object.keys(errs).find((id) => photos.some((p) => p.id === id));
  if (photos.length === 0) {
    return (
      <p className="flex items-center gap-2 px-3.5 py-4 text-[14px] text-zinc-400 sm:text-[13px]" data-empty="wall">
        <Images size={16} aria-hidden="true" />
        Nothing on the wall yet. Photos you approve show here, on the TV and on every phone.
      </p>
    );
  }
  return (
    <div className="flex flex-col gap-2 p-2.5">
      <ul className="grid grid-cols-3 gap-1.5 sm:grid-cols-5" data-grid>
        {shown.map((p) => (
          <li key={p.id} className="wall-tile group relative aspect-[4/3]">
            <Photo code={code} p={p} className="h-full w-full" />
            {captions && p.caption && (
              <span className="pointer-events-none absolute inset-x-0 bottom-0 truncate rounded-b-[10px] bg-gradient-to-t from-[color-mix(in_srgb,var(--kw-surface-rail)_88%,transparent)] to-transparent px-2 pb-1.5 pt-4 text-[11.5px] font-semibold leading-[15px] text-zinc-50">
                {p.caption}
              </span>
            )}
            <button
              type="button"
              onClick={() => onHide(p)}
              aria-label={`Hide ${firstName(p.by_name)}'s photo`}
              title="Hide"
              className="wall-hide absolute right-1 top-1 grid h-11 w-11 place-items-center rounded-[10px] border border-hairline-strong bg-[color-mix(in_srgb,var(--kw-surface-rail)_85%,transparent)] text-zinc-100 kw-focus [@media(hover:hover)]:h-9 [@media(hover:hover)]:w-9 [@media(hover:hover)]:opacity-0 [@media(hover:hover)]:group-hover:opacity-100 [@media(hover:hover)]:focus-visible:opacity-100"
              data-action="hide-approved"
            >
              <EyeOff size={15} aria-hidden="true" />
            </button>
          </li>
        ))}
      </ul>
      {photos.length > shown.length && (
        <button type="button" onClick={() => setAll(true)} className="self-start rounded-lg px-1 py-2 text-[14px] font-semibold text-sky-300 hover:text-sky-200 kw-focus sm:text-[12.5px]" data-action="more-wall">
          Show all {num(photos.length)}
        </button>
      )}
      {err && (
        <p role="alert" className="flex items-center gap-1.5 text-[14px] text-bad sm:text-[13px]" data-row-error>
          <AlertTriangle size={13} aria-hidden="true" />
          {errs[err]}
        </p>
      )}
      <p className="flex items-start gap-1.5 text-[13px] leading-5 text-zinc-400 sm:text-meta">
        <Eye size={13} className={ICON} aria-hidden="true" />
        Hide takes a photo off the TV and every phone at once and deletes it.
      </p>
    </div>
  );
};

// ---------------------------------------------------------------------------------------------------
// Best 30

const BestBlock: React.FC<{ code: string; w: WallInfo; stage: 'picked' | 'shown'; photos: Map<string, WallPhoto> }> = ({ code, w, stage, photos }) => {
  const wide = useMedia('(min-width: 640px)');
  const n = w.best.length;
  const max = wide ? 15 : 9;
  const shown = w.best.slice(0, n > max ? max : n);
  const snap = w.best_by === 'snap';
  const head = snap ? `Snap's best ${num(n)}` : `Your best ${num(n)}`;
  const line =
    stage === 'shown'
      ? `Guests see these ${num(n)} on their page now.`
      : snap
        ? 'Snap picked these for sharp, smiling, and not too alike. Swap any, then show them to guests.'
        : 'Your picks so far. Show them to guests when you are happy with them.';
  return (
    <div className="flex flex-col gap-2.5 rounded-xl border border-hairline-strong bg-surface-main px-3.5 py-3" data-best={stage}>
      <div className="flex items-center gap-2.5 text-[15px] font-semibold text-zinc-50 sm:text-[14px]">
        <Star size={15} className="text-sky-300" aria-hidden="true" />
        {head}
        {stage === 'shown' && earnedPicked(w) && (
          <span className="ml-auto">
            <Picked n={n} />
          </span>
        )}
      </div>
      <ul className="grid grid-cols-5 gap-1.5 sm:grid-cols-8" data-strip>
        {shown.map((id) => {
          const p = photos.get(id);
          return (
            <li key={id} className="aspect-square">
              {p ? <Photo code={code} p={p} ratio={1} className="h-full w-full rounded-lg" /> : <span className="block h-full w-full rounded-lg border border-hairline bg-surface-raised" aria-hidden="true" />}
            </li>
          );
        })}
        {n > shown.length && (
          <li className="grid aspect-square place-items-center rounded-lg border border-hairline-strong bg-surface-raised text-[14px] font-bold text-zinc-200" aria-label={`and ${n - shown.length} more`}>
            +{num(n - shown.length)}
          </li>
        )}
      </ul>
      <p className="text-[14px] leading-5 text-zinc-300 sm:text-[13px]">{line}</p>
    </div>
  );
};

// ---------------------------------------------------------------------------------------------------
// Actions

type ConfirmKind = 'stop' | 'delete';
const DownloadAll: React.FC<{ code: string; label: string; waiting: number; primary?: boolean; className?: string }> = ({ code, label, waiting, primary, className = '' }) => {
  const [open, setOpen] = useState(false);
  const menuId = useId();
  const wrap = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!open) return;
    const off = (e: MouseEvent) => wrap.current && !wrap.current.contains(e.target as Node) && setOpen(false);
    document.addEventListener('mousedown', off);
    return () => document.removeEventListener('mousedown', off);
  }, [open]);
  const main = (
    <a href={wallZipUrl(code)} download={`wall-${code}.zip`} className={`${primary ? PRIMARY : GHOST} ${waiting > 0 ? 'rounded-r-none' : ''} min-w-0 flex-1`} data-action="download-all">
      <Download size={14} aria-hidden="true" />
      {label}
    </a>
  );
  if (waiting === 0) return <div className={`flex ${className}`}>{main}</div>;
  return (
    <div ref={wrap} className={`relative flex ${className}`} onKeyDown={(e) => e.key === 'Escape' && setOpen(false)}>
      {main}
      <button type="button" aria-expanded={open} aria-controls={menuId} aria-label="More download options" onClick={() => setOpen(!open)} className={`${primary ? PRIMARY : GHOST} rounded-l-none border-l border-hairline px-2`} data-action="download-menu">
        <ChevronDown size={14} aria-hidden="true" />
      </button>
      {open && (
        <div id={menuId} className="kw-fade-in absolute left-0 top-full z-10 mt-1 min-w-full rounded-xl border border-hairline-strong bg-surface-raised p-1 shadow-card">
          <a href={wallZipUrl(code, 'all')} download={`wall-${code}.zip`} onClick={() => setOpen(false)} className={`flex items-center gap-2 rounded-lg px-3 text-zinc-100 hover:bg-white/[0.05] kw-focus ${BTN}`} data-action="download-with-waiting">
            <Download size={14} aria-hidden="true" />
            Include the {num(waiting)} waiting
          </a>
        </div>
      )}
    </div>
  );
};

const WallActions: React.FC<{
  block: PocketBlock;
  w: WallInfo;
  stage: WallStage;
  joinUrl: string | null;
  waiting: number;
  approved: number;
  gridOpen: boolean;
  showing: boolean;
  ask: (t: string) => void;
  onGrid: () => void;
  onShow: () => void;
  onConfirm: (k: ConfirmKind) => void;
}> = ({ block, w, stage, joinUrl, waiting, approved, gridOpen, showing, ask, onGrid, onShow, onConfirm }) => {
  const [copied, setCopied] = useState<'yes' | 'failed' | null>(null);
  const [screenErr, setScreenErr] = useState<string | null>(null);
  useEffect(() => {
    if (!copied) return;
    const t = setTimeout(() => setCopied(null), 2000);
    return () => clearTimeout(t);
  }, [copied]);
  const grid = 'grid grid-cols-2 gap-2 sm:flex sm:flex-wrap sm:items-center';
  const full = 'col-span-2 sm:col-span-1';
  const share = async () => {
    if (!joinUrl) return;
    try {
      await navigator.clipboard.writeText(`Add your photos to ${block.name}: ${joinUrl}`);
      setCopied('yes');
    } catch {
      setCopied('failed');
    }
  };
  const bigScreen = async () => {
    if (!joinUrl) return;
    setScreenErr(null);
    // Open the window inside the click (popup blockers), then point it at the minted link.
    const win = window.open('about:blank', '_blank');
    try {
      const link = await pocketApi.mintLink(block.code, 'screen');
      const url = linkUrl(joinUrl, 'screen', link.token);
      if (win) win.location.href = url;
      else window.open(url, '_blank', 'noopener');
    } catch (e) {
      win?.close();
      setScreenErr(`Couldn't open the big screen: ${e instanceof PocketApiError ? e.message : 'something went wrong.'}`);
    }
  };
  const cls = (primary: boolean, half = false) => (primary ? `${PRIMARY} ${full}` : half ? `kw-btn-ghost ${HALF}` : `${GHOST} ${full}`);
  const needsLink = (node: React.ReactNode, label: string, icon: React.ReactNode, primary: boolean, half = false) =>
    joinUrl ? (
      node
    ) : (
      <button type="button" aria-disabled="true" title={WALL_LATER.noLink} aria-description={WALL_LATER.noLink} onClick={(e) => e.preventDefault()} className={`${cls(primary, half)} ${LATER_CLS}`}>
        {icon}
        {label}
      </button>
    );
  const shareBtn = (primary: boolean, half = false) =>
    needsLink(
      <button type="button" onClick={share} className={cls(primary, half)} data-action="share">
        {copied === 'yes' ? <Check size={14} aria-hidden="true" /> : <Copy size={14} aria-hidden="true" />}
        <span aria-live="polite">{copied === 'yes' ? 'Link copied' : copied === 'failed' ? "Couldn't copy" : 'Share link'}</span>
      </button>,
      'Share link',
      <Copy size={14} aria-hidden="true" />,
      primary,
      half,
    );
  const screenBtn = (primary: boolean, half = false) =>
    needsLink(
      <button type="button" onClick={bigScreen} className={cls(primary, half)} data-action="big-screen">
        <Maximize2 size={14} aria-hidden="true" />
        Big screen
      </button>,
      'Big screen',
      <Maximize2 size={14} aria-hidden="true" />,
      primary,
      half,
    );
  const edit = (
    <button type="button" onClick={() => ask(`Change the settings of ${block.name}`)} className={`kw-btn-ghost ${HALF}`} data-action="edit">
      <Pencil size={14} aria-hidden="true" />
      Edit
    </button>
  );
  const stop = (
    <button type="button" onClick={() => onConfirm('stop')} className={`${STOP} col-span-1 justify-self-stretch sm:ml-auto`} data-action="stop">
      <Square size={14} aria-hidden="true" />
      Stop
    </button>
  );
  const del = (
    <button
      type="button"
      onClick={() => onConfirm('delete')}
      className={`inline-flex items-center justify-center gap-1.5 rounded-xl px-3 font-medium text-zinc-300 hover:bg-white/[0.04] hover:text-zinc-100 kw-focus ${BTN} col-span-2 sm:ml-auto`}
      data-action="delete"
    >
      <Trash2 size={14} aria-hidden="true" />
      Delete now
    </button>
  );
  const total = approved + waiting;
  const errLine = screenErr && (
    <p role="alert" className="col-span-2 flex items-center gap-1.5 text-[14px] text-bad sm:basis-full sm:text-[13px]">
      <AlertTriangle size={13} aria-hidden="true" />
      {screenErr}
    </p>
  );
  if (w.state === 'open') {
    if (stage === 'ready') {
      return (
        <div className={grid}>
          {shareBtn(true)}
          {screenBtn(false, true)}
          {needsLink(
            <a href={joinUrl ?? undefined} target="_blank" rel="noopener noreferrer" className={cls(false, true)} data-action="try">
              <Upload size={14} aria-hidden="true" />
              Try adding one
            </a>,
            'Try adding one',
            <Upload size={14} aria-hidden="true" />,
            false,
            true,
          )}
          {edit}
          {stop}
          {errLine}
        </div>
      );
    }
    return (
      <div className={grid}>
        {screenBtn(true)}
        <DownloadAll code={block.code} label="Download all" waiting={waiting} className={full} />
        {shareBtn(false, true)}
        {edit}
        {stop}
        {errLine}
      </div>
    );
  }
  switch (stage) {
    case 'picked':
      return (
        <div className={grid}>
          <button type="button" onClick={onShow} disabled={showing} className={`${PRIMARY} ${full} disabled:opacity-60`} data-action="show-best">
            {showing ? <Loader2 size={14} className="motion-safe:animate-spin" aria-hidden="true" /> : <Users size={14} aria-hidden="true" />}
            Show the {num(w.best.length)} to guests
          </button>
          <LaterButton half icon={<Star size={14} aria-hidden="true" />} why={WALL_LATER.swap}>
            Swap photos
          </LaterButton>
          <DownloadAll code={block.code} label="Download all" waiting={waiting} className="col-span-1" />
          {del}
        </div>
      );
    case 'shown':
      return (
        <div className={grid}>
          <a href={wallZipUrl(block.code, 'best')} download={`wall-${block.code}.zip`} className={`${PRIMARY} ${full}`} data-action="download-best">
            <Download size={14} aria-hidden="true" />
            Download the best {num(w.best.length)}
          </a>
          <DownloadAll code={block.code} label={`Download all · ${num(total)}`} waiting={waiting} className="col-span-2 sm:col-span-1" />
          {shareBtn(false)}
          {del}
        </div>
      );
    default:
      return (
        <div className={grid}>
          <DownloadAll code={block.code} label={`Download all · ${num(total)}`} waiting={waiting} primary className={full} />
          <button type="button" onClick={onGrid} aria-expanded={gridOpen} className={`kw-btn-ghost ${HALF}`} data-action="open-wall">
            <Images size={14} aria-hidden="true" />
            {gridOpen ? 'Close the wall' : 'Open the wall'}
          </button>
          {del}
        </div>
      );
  }
};

const CONFIRM: Record<ConfirmKind, { q: (b: PocketBlock, n: number) => string; text: (b: PocketBlock, n: number) => string; yes: string; no: string }> = {
  stop: {
    q: (b) => `Stop ${b.name}?`,
    text: () => "The link closes, so guests can't add photos any more. The photos already here stay until they leave this box.",
    yes: 'Stop it',
    no: 'Keep going',
  },
  delete: {
    q: (_b, n) => `Delete all ${num(n)} photos from this box now?`,
    text: () => 'Download them first if you want to keep them.',
    yes: 'Delete them now',
    no: 'Keep them',
  },
};
const DONE_TEXT: Record<ConfirmKind, string> = {
  stop: 'Stopped. The link is closed; the photos stay until they leave this box.',
  delete: 'Deleted. No photo of this wall is left on this box.',
};

const WallConfirm: React.FC<{ block: PocketBlock; n: number; kind: ConfirmKind; busy: boolean; onYes: () => void; onNo: () => void }> = ({ block, n, kind, busy, onYes, onNo }) => {
  const yes = useRef<HTMLButtonElement | null>(null);
  useEffect(() => yes.current?.focus(), []);
  const c = CONFIRM[kind];
  return (
    <div
      role="alertdialog"
      aria-label={c.q(block, n)}
      onKeyDown={(e) => e.key === 'Escape' && !busy && onNo()}
      className="kw-fade-in flex flex-col gap-3 rounded-xl border border-rose-300/35 bg-rose-300/[0.06] p-3.5"
      data-confirm={kind}
    >
      <p className="text-[15px] leading-[22px] text-zinc-100 sm:text-body">
        <strong className="font-semibold">{c.q(block, n)}</strong> {c.text(block, n)}
      </p>
      <div className="grid grid-cols-1 gap-2 sm:flex sm:flex-wrap">
        <button ref={yes} type="button" onClick={onYes} disabled={busy} className={`inline-flex items-center justify-center gap-1.5 rounded-xl bg-rose-600 px-3.5 font-semibold text-white hover:bg-rose-700 disabled:opacity-60 kw-focus ${BTN}`} data-action={`${kind}-yes`}>
          {busy && <Loader2 size={14} className="motion-safe:animate-spin" aria-hidden="true" />}
          {c.yes}
        </button>
        <button type="button" onClick={onNo} disabled={busy} className={GHOST}>
          {c.no}
        </button>
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------------------------------
// The card

type Done = { kind: 'ok' | 'error'; text: string; closes: boolean } | null;
type Load = 'loading' | 'ok' | 'error';

/** Merge the numbers an answer carried into the card's own copy of the summary. */
const mergeView = (w: WallInfo, v: WallView): WallInfo => ({
  ...w,
  counts: v.counts ?? w.counts,
  used_mb: v.used_mb ?? w.used_mb,
  best: v.best ?? w.best,
  best_set: v.best_set ?? w.best_set,
});

export const WallCard: React.FC<{ block: PocketBlock; ask: (t: string) => void; join?: React.ReactNode; undoMs?: number }> = ({ block, ask, join, undoMs = UNDO_MS }) => {
  const [w, setW] = useState<WallInfo>(block.wall!);
  useEffect(() => setW(block.wall!), [block.wall]);
  const [view, setView] = useState<WallView | null>(null);
  const [load, setLoad] = useState<Load>('loading');
  const [pending, setPending] = useState<Pending>({});
  // Rows approved a moment ago stay 160 ms to fade and move 8 px up (design Motion); counts move at once.
  const [leaving, setLeaving] = useState<Set<string>>(() => new Set());
  const [errs, setErrs] = useState<Record<string, string>>({});
  const [undo, setUndo] = useState<Undo>(null);
  const [tab, setTab] = useState<'waiting' | 'wall' | null>(null);
  const [more, setMore] = useState(false);
  const [gridOpen, setGridOpen] = useState(false);
  const [confirm, setConfirm] = useState<ConfirmKind | null>(null);
  const [busy, setBusy] = useState(false);
  const [showing, setShowing] = useState(false);
  const [done, setDone] = useState<Done>(null);
  const opener = useRef<HTMLDivElement | null>(null);
  const focusAfter = useFocusAfterRender();
  const tabIds: [string, string] = [useId(), useId()];
  const code = block.code;

  const take = useCallback((r: unknown) => {
    const v = isObj(r) ? validWallView(r.host_view) : null;
    if (!v) return false;
    setView(v);
    setW((old) => mergeView(old, v));
    return true;
  }, []);

  const read = useCallback(async () => {
    setLoad('loading');
    try {
      const r = await pocketApi.read(code);
      setLoad(take(r) ? 'ok' : 'error');
    } catch {
      setLoad('error');
    }
  }, [code, take]);
  useEffect(() => {
    void read();
  }, [read]);

  /** Send an action for these ids; their rows are already gone. On failure they come back with a plain line. */
  const send = useCallback(
    async (ids: string[], name: string, body: Record<string, unknown>) => {
      try {
        const r = await pocketApi.hostAction(code, name, body);
        if (!take(r)) await read();
        setErrs((e) => {
          const next = { ...e };
          for (const id of ids) delete next[id];
          return next;
        });
      } catch {
        setErrs((e) => ({ ...e, ...Object.fromEntries(ids.map((id) => [id, FAIL])) }));
      } finally {
        setPending((p) => {
          const next = { ...p };
          for (const id of ids) delete next[id];
          return next;
        });
      }
    },
    [code, read, take],
  );

  // The 5 s Undo on hiding a waiting photo. Leaving the card or approving another sends it at once.
  const timer = useRef<{ id: string; t: ReturnType<typeof setTimeout>; tick: ReturnType<typeof setInterval> } | null>(null);
  const flush = useCallback(() => {
    const cur = timer.current;
    if (!cur) return;
    clearTimeout(cur.t);
    clearInterval(cur.tick);
    timer.current = null;
    setUndo(null);
    void send([cur.id], 'hide', { id: cur.id });
  }, [send]);
  const flushRef = useRef(flush);
  flushRef.current = flush;
  useEffect(() => () => flushRef.current(), []);

  const mark = (ids: string[], kind: 'approve' | 'hide') => {
    setPending((p) => ({ ...p, ...Object.fromEntries(ids.map((id) => [id, kind])) }));
    setErrs((e) => {
      const next = { ...e };
      for (const id of ids) delete next[id];
      return next;
    });
  };
  const fade = (ids: string[]) => {
    setLeaving((l) => new Set([...l, ...ids]));
    setTimeout(() => setLeaving((l) => new Set([...l].filter((id) => !ids.includes(id)))), 160);
  };
  const approve = (p: WallPhoto) => {
    flush();
    fade([p.id]);
    mark([p.id], 'approve');
    void send([p.id], 'approve', { id: p.id });
  };
  const approveAll = (g: Group) => {
    flush();
    const ids = g.photos.filter((p) => !pending[p.id]).map((p) => p.id);
    fade(ids);
    mark(ids, 'approve');
    void send(ids, 'approve_all_from', { guest: g.by });
  };
  const hideWaiting = (p: WallPhoto) => {
    flush();
    mark([p.id], 'hide');
    const seconds = Math.max(1, Math.round(undoMs / 1000));
    setUndo({ id: p.id, name: firstName(p.by_name), left: seconds });
    const tick = setInterval(() => setUndo((u) => (u && u.id === p.id ? { ...u, left: Math.max(1, u.left - 1) } : u)), undoMs / seconds);
    const t = setTimeout(() => flushRef.current(), undoMs);
    timer.current = { id: p.id, t, tick };
  };
  const undoHide = () => {
    const cur = timer.current;
    if (!cur) return;
    clearTimeout(cur.t);
    clearInterval(cur.tick);
    timer.current = null;
    setUndo(null);
    setPending((p) => {
      const next = { ...p };
      delete next[cur.id];
      return next;
    });
    focusAfter(() => opener.current?.closest('section')?.querySelector<HTMLButtonElement>(`[data-row="${cur.id}"] [data-action="hide"]`));
  };
  const hideApproved = (p: WallPhoto) => {
    mark([p.id], 'hide');
    void send([p.id], 'hide', { id: p.id });
  };

  const showBest = async () => {
    setShowing(true);
    try {
      const r = await pocketApi.hostAction(code, 'set_best', { ids: w.best });
      if (!take(r)) setW((old) => ({ ...old, best_set: true }));
      setDone({ kind: 'ok', text: `Shown. Guests see the ${num(w.best.length)} on their page now.`, closes: false });
    } catch (e) {
      setDone({ kind: 'error', text: `Couldn't show them: ${e instanceof PocketApiError ? e.message : 'something went wrong.'} Try again.`, closes: false });
    } finally {
      setShowing(false);
    }
  };
  const act = async (kind: ConfirmKind) => {
    setBusy(true);
    try {
      flush();
      if (kind === 'delete') await pocketApi.remove(code);
      else await pocketApi.end(code);
      setDone({ kind: 'ok', text: DONE_TEXT[kind], closes: true });
      setConfirm(null);
    } catch (e) {
      setDone({ kind: 'error', text: `Couldn't do that: ${e instanceof PocketApiError ? e.message : 'something went wrong.'} Try again.`, closes: false });
    } finally {
      setBusy(false);
    }
  };
  const closeConfirm = () => {
    const k = confirm;
    setConfirm(null);
    focusAfter(() => opener.current?.querySelector<HTMLButtonElement>(`[data-action="${k}"]`));
  };

  // What the card shows: the box's lists less what's in flight; the block's counts until the lists arrive.
  const queue = useMemo(() => (view ? view.queue.filter((p) => !pending[p.id]) : []), [view, pending]);
  const rowsShown = useMemo(() => (view ? view.queue.filter((p) => !pending[p.id] || (pending[p.id] === 'approve' && leaving.has(p.id))) : []), [view, pending, leaving]);
  const wallList = useMemo(
    () => (view ? [...view.queue.filter((p) => pending[p.id] === 'approve'), ...view.wall.filter((p) => pending[p.id] !== 'hide')] : []),
    [view, pending],
  );
  const waiting = view ? queue.length : w.counts.waiting;
  const approved = view ? wallList.length : w.counts.approved;
  const shownW: WallInfo = { ...w, counts: { ...w.counts, waiting, approved } };
  const stage = wallStage(shownW);
  const mood = SNAP_MOOD[stage];
  const deleted = done?.kind === 'ok' && done.closes;
  const curTab = tab ?? (waiting > 0 || undo ? 'waiting' : 'wall');
  const photos = useMemo(() => new Map([...(view?.wall ?? []), ...(view?.queue ?? [])].map((p) => [p.id, p])), [view]);
  const showPanel = !deleted && ((w.state === 'open' && (w.counts.total > 0 || waiting > 0)) || (w.state === 'closed' && (waiting > 0 || gridOpen)));
  const groups = groupsOf(rowsShown);
  let budget = more ? Infinity : QUEUE_SHOWN;
  const left = Math.max(0, queue.length - QUEUE_SHOWN);
  const undoPhoto = undo ? view?.queue.find((p) => p.id === undo.id) : undefined;
  const undoGroup = undoPhoto?.by;

  return (
    <section
      className="pocket-card relative flex max-w-[680px] flex-col gap-4 overflow-hidden rounded-2xl border border-sky-800 bg-surface-raised p-4 shadow-card sm:p-[18px]"
      data-pocket={block.state}
      data-wall={stage}
      aria-label={`${block.name}, pocket app`}
    >
      <div className="pointer-events-none absolute inset-x-0 top-0 h-28 bg-gradient-to-b from-sky-500/[0.07] to-transparent" aria-hidden="true" />
      <header className="relative flex items-start gap-3">
        <SnapTile mood={mood} />
        <div className="min-w-0 flex-1">
          <div className="text-2xs font-semibold text-sky-300">Pocket app · Photo wall</div>
          <h3 className="text-[20px] font-bold leading-[26px] text-zinc-50 sm:text-[19px]">{block.name}</h3>
          {block.when && <p className="text-[15px] text-zinc-300 sm:text-[13px]">{block.when}</p>}
          <span className="mt-1.5 block sm:hidden">
            <WallChip w={w} stage={stage} waiting={waiting} />
          </span>
        </div>
        <span className="hidden shrink-0 sm:block">
          <WallChip w={w} stage={stage} waiting={waiting} />
        </span>
      </header>
      <div className="relative flex flex-col gap-4">
        <div className="grid grid-cols-3 gap-2" data-counts>
          <Tile n={approved} label="On the wall" />
          <Tile n={waiting} label="Waiting for you" on={waiting > 0} />
          <Tile n={w.counts.guests} label="Guests" />
        </div>
        <div className="flex flex-col gap-1.5" data-meter>
          <Bar value={w.used_mb / w.pocket_cap_mb} label="Photo space used" tone={spaceLow(w) ? 'warn' : 'accent'} />
          <p className="text-[13px] leading-5 text-zinc-400 sm:text-meta">{meterLine(w)}</p>
        </div>
        {stage === 'space' && (
          <Callout tone="warn">
            <p>
              About {plural(photosLeft(w), 'photo')} left. When the wall is full, guests read 'The wall is full for now. {w.host_name ?? 'The host'} can make room.' Nothing is lost that's already here.
            </p>
            <p className="mt-1 text-zinc-400">The wall also stops if this computer has under 2 GB free.</p>
            <div className="mt-2.5">
              <LaterButton icon={<HardDrive size={14} aria-hidden="true" />} why={WALL_LATER.twoGb}>
                Make it 2 GB
              </LaterButton>
            </div>
          </Callout>
        )}
        <ul className="flex flex-col gap-2">
          {w.state === 'open' && (
            <Row icon={<ShieldCheck size={15} aria-hidden="true" />}>
              {stage === 'ready'
                ? 'Nothing shows on the TV or other phones until you approve it. Location and camera details are removed when a photo arrives.'
                : "You approve each photo before it shows on the TV or anyone's phone."}
            </Row>
          )}
          <Row icon={<Download size={15} aria-hidden="true" />}>{keepLine(w)}</Row>
          {stage === 'picking' && (
            <Row icon={<Sparkles size={15} aria-hidden="true" />} aside={<span className="kw-chip kw-chip-info">Picking</span>}>
              Snap is picking the best {num(w.best_max)} from the {num(w.counts.approved)}. You look them over before guests see them.
            </Row>
          )}
        </ul>
        {stage === 'ready' && join}
        {(stage === 'picked' || stage === 'shown') && !deleted && <BestBlock code={code} w={w} stage={stage} photos={photos} />}
        {showPanel && (
          <div className="overflow-hidden rounded-xl border border-hairline-strong bg-surface-main" data-queue>
            {load === 'loading' && !view ? (
              <p role="status" className="flex items-center gap-2 px-3.5 py-4 text-[14px] text-zinc-400 sm:text-[13px]">
                <Loader2 size={15} className="motion-safe:animate-spin" aria-hidden="true" />
                Getting the photos from your box…
              </p>
            ) : !view ? (
              <div className="flex flex-wrap items-center gap-3 px-3.5 py-3.5" data-load-error>
                <p role="alert" className="flex items-center gap-1.5 text-[14px] text-bad sm:text-[13px]">
                  <AlertTriangle size={14} aria-hidden="true" />
                  {FAIL}
                </p>
                <button type="button" onClick={() => void read()} className={GHOST} data-action="retry">
                  <RotateCcw size={14} aria-hidden="true" />
                  Try again
                </button>
              </div>
            ) : (
              <>
                <Tabs tab={curTab} setTab={setTab} waiting={waiting} approved={approved} ids={tabIds} />
                <div role="tabpanel" id={curTab === 'waiting' ? tabIds[0] : tabIds[1]} aria-labelledby={`${curTab === 'waiting' ? tabIds[0] : tabIds[1]}-tab`}>
                  {curTab === 'waiting' ? (
                    queue.length === 0 && !undo ? (
                      <p className="flex items-center gap-2 px-3.5 py-4 text-[14px] text-zinc-400 sm:text-[13px]" data-empty="queue">
                        <Check size={15} className="text-ok" aria-hidden="true" />
                        All caught up. New photos wait here for you.
                      </p>
                    ) : (
                      <ul>
                        {undo && !undoGroup && <UndoRow u={undo} onUndo={undoHide} />}
                        {groups.map((g, gi) => {
                          if (budget <= 0) return null;
                          const rows = g.photos.slice(0, budget);
                          budget -= rows.length;
                          const n = g.photos.filter((p) => !pending[p.id]).length;
                          return (
                            <li key={g.by} data-group={g.name}>
                              {n > 0 && (
                                <div className={`flex min-h-[44px] items-center gap-2.5 border-b border-hairline bg-sky-500/[0.05] px-3.5 py-0 text-[14px] text-zinc-200 sm:text-[13px] ${gi > 0 ? 'border-t' : ''}`} data-group-header>
                                  <WallCreature n={g.creature} />
                                  <span className="min-w-0 flex-1 truncate">
                                    <b className="font-semibold text-zinc-50">{g.name}</b>
                                    {/* With 2+ the Approve all button carries the count on phones; with 1 the header does. */}
                                    <span className={n > 1 ? 'max-sm:sr-only' : ''}> · {num(n)} waiting</span>
                                  </span>
                                  {n > 1 && (
                                    <button type="button" onClick={() => approveAll(g)} className={`kw-btn-ghost ${HALF} ml-auto`} data-action="approve-all">
                                      <Check size={14} aria-hidden="true" />
                                      Approve all {num(n)}
                                    </button>
                                  )}
                                </div>
                              )}
                              <ul>
                                {undo && undoGroup === g.by && <UndoRow u={undo} onUndo={undoHide} />}
                                {rows.map((p) => (
                                  <QueueRow key={p.id} code={code} p={p} captions={w.captions} err={errs[p.id] ?? null} leaving={!!pending[p.id]} onApprove={() => approve(p)} onHide={() => hideWaiting(p)} />
                                ))}
                              </ul>
                            </li>
                          );
                        })}
                        {undo && undoGroup && !groups.some((g) => g.by === undoGroup) && <UndoRow u={undo} onUndo={undoHide} />}
                        {!more && left > 0 && (
                          <li className="px-3.5 py-1.5">
                            <button type="button" onClick={() => setMore(true)} className="rounded-lg px-1 py-2 text-[14px] font-semibold text-sky-300 hover:text-sky-200 kw-focus sm:text-[12.5px]" data-action="more-queue">
                              Show {num(left)} more waiting
                            </button>
                          </li>
                        )}
                      </ul>
                    )
                  ) : (
                    <WallGrid code={code} photos={wallList} captions={w.captions} errs={errs} onHide={hideApproved} />
                  )}
                </div>
              </>
            )}
          </div>
        )}
        {done ? (
          <p role="status" className={`flex items-start gap-2 text-[15px] sm:text-[13px] ${done.kind === 'ok' ? 'text-ok' : 'text-bad'}`} data-done={done.kind}>
            {done.kind === 'ok' ? <Check size={15} className={ICON} aria-hidden="true" /> : <AlertTriangle size={15} className={ICON} aria-hidden="true" />}
            {done.text}
          </p>
        ) : null}
        {confirm ? (
          <WallConfirm block={block} n={w.counts.total - w.counts.hidden} kind={confirm} busy={busy} onYes={() => act(confirm)} onNo={closeConfirm} />
        ) : deleted ? null : (
          <div ref={opener}>
            <WallActions
              block={block}
              w={w}
              stage={stage}
              joinUrl={block.join?.anywhere_url ?? block.join?.wifi_url ?? null}
              waiting={waiting}
              approved={approved}
              gridOpen={gridOpen}
              showing={showing}
              ask={ask}
              onGrid={() => {
                setGridOpen(!gridOpen);
                setTab('wall');
              }}
              onShow={showBest}
              onConfirm={setConfirm}
            />
          </div>
        )}
      </div>
      <footer className="relative -mx-4 -mb-4 flex items-start gap-2 border-t border-hairline px-4 py-3 text-meta text-zinc-400 sm:-mx-[18px] sm:-mb-[18px] sm:px-[18px]" data-budget>
        <Zap size={13} className="mt-0.5 shrink-0" aria-hidden="true" />
        <span>{wallBudgetLine(block)}</span>
      </footer>
    </section>
  );
};

// ---------------------------------------------------------------------------------------------------
// Done row (Inbox) — the component; the server's Inbox item is a follow-up (card-impl.md)

export function wallDoneText(block: PocketBlock): string {
  const w = block.wall!;
  const base = `${block.name} · ${plural(w.counts.approved, 'photo')} from ${plural(w.counts.senders, 'guest')}`;
  if (w.best_set) return `${base} · best ${num(w.best.length)} shown`;
  return w.delete_on ? `${base} · leaves this box ${w.delete_on}` : base;
}

export const WallDoneRow: React.FC<{ block: PocketBlock; onOpen?: () => void }> = ({ block, onOpen }) => {
  const w = block.wall!;
  const mood = SNAP_MOOD[wallStage(w)];
  return (
    <div className="flex min-h-[56px] items-center gap-3 rounded-xl border border-hairline bg-surface-raised px-3.5 py-2.5" data-wall-done={w.best_set ? 'shown' : 'kept'}>
      <ModernAvatar role="snap" name="Snap" size={28} mood={mood} moodText={SNAP_SAYS[mood]} modelBadge={false} />
      <button type="button" onClick={onOpen} className="min-w-0 flex-1 rounded-lg text-left text-[15px] leading-[22px] text-zinc-100 kw-focus sm:text-[13px] sm:leading-5">
        <span className="line-clamp-2">{wallDoneText(block)}</span>
      </button>
      {earnedPicked(w) && <Picked n={w.best.length} />}
    </div>
  );
};
