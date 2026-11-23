// Pocket apps (pa10a): the Secret Santa Pocket card in chat, states 1 Collecting, 1b No stable address,
// 2 Drawn, 3 Reveal day, 4 Ended, plus People and PINs with the Reset PIN one-time code (pa10a/design.md §1,
// "Rules this design keeps", "For Pixel" 1-5). The host card shows counts and first names only: the validator
// rejects a block that carries anything match-like or a question's words, and People and PINs renders only
// what the box answers (host/reset-pin gives a code and its expiry, nothing else). Holo once, only when earned:
// "All ready" when every Santa ticked their gift, on Reveal day and Ended. Everything that goes wrong is plain.

import React, { useEffect, useId, useRef, useState } from 'react';
import {
  ChevronDown,
  AlertTriangle,
  CalendarDays,
  Check,
  CheckCircle2,
  Copy,
  Ellipsis,
  Eye,
  EyeOff,
  Gift,
  Globe,
  HeartCrack,
  Info,
  KeyRound,
  Loader2,
  Lock,
  MessageSquare,
  PartyPopper,
  Plus,
  RotateCcw,
  ShieldCheck,
  Shuffle,
  Smartphone,
  Square,
  Trash2,
  UserMinus,
  Users,
  Wifi,
  X,
  Zap,
} from 'lucide-react';
import type { PocketBlock, SantaInfo, SantaPerson, SantaPhase } from '../../smartResults';
import { isHttpUrl, isIndex, isObj, isText, optText } from '../../smartResults';
import { Sticker } from '../Sticker';
import { guestCreature, renderCreature } from '../ModernAvatar';
import { pocketApi, PocketApiError, type ResetCode } from '../../pocketApi';
import { useFocusAfterRender } from '../../focusAfterRender';
import { BTN, Bar, Callout, GHOST, HALF, ICON, LATER_CLS, LaterButton, LinkField, PRIMARY, Qr, Row, STOP } from './pocketParts';

// ---------------------------------------------------------------------------------------------------
// Validator

export const SANTA_PHASES: SantaPhase[] = ['collecting', 'drawn', 'reveal_day', 'ended'];
const MAX_PEOPLE = 60;
const isCount = (v: unknown): v is number => isIndex(v) && v <= 100_000;
const optCount = (v: unknown) => (v === undefined || v === null ? null : isCount(v) ? v : undefined);

/**
 * Keys that would put a match, the loop, a wish list or a question's words on the host's card. Any of them,
 * anywhere in the block, at any depth, rejects it (design "Nobody sees a match but its owner").
 */
export const MATCH_KEYS = [
  'match', 'matches', 'my_match', 'pair', 'pairs', 'pairing', 'pairings', 'gives_to', 'giving_to', 'buys_for', 'buying_for',
  'receives_from', 'giver', 'givers', 'receiver', 'receivers', 'recipient', 'recipients', 'assigned', 'assignment', 'assignments',
  'santa_of', 'my_santa', 'loop', 'ring', 'reveal_loop', 'derangement', 'draw_result',
  'questions', 'question', 'asks', 'ask', 'answers', 'answer', 'q', 'a', 'text', 'body', 'wish', 'wishes', 'wish_list', 'my_wish',
] as const;
const MATCH_SET = new Set<string>(MATCH_KEYS);

/** The first match-like key path in `v`, or null. Walks arrays and objects; depth-capped. */
export function findMatchLike(v: unknown, path = '', depth = 0): string | null {
  if (depth > 8 || v === null || typeof v !== 'object') return null;
  if (Array.isArray(v)) {
    for (let i = 0; i < v.length; i++) {
      const hit = findMatchLike(v[i], `${path}[${i}]`, depth + 1);
      if (hit) return hit;
    }
    return null;
  }
  for (const [k, x] of Object.entries(v as Record<string, unknown>)) {
    if (MATCH_SET.has(k.toLowerCase())) return `${path}.${k}`;
    const hit = findMatchLike(x, `${path}.${k}`, depth + 1);
    if (hit) return hit;
  }
  return null;
}

const firstName = (v: unknown): v is string => isText(v, 40);

function validPerson(v: unknown): SantaPerson | undefined {
  if (!isObj(v) || !isText(v.id, 64) || !firstName(v.name)) return undefined;
  const avatar = v.avatar === undefined || v.avatar === null || v.avatar === '' ? null : typeof v.avatar === 'string' && /^[a-z0-9_-]{1,32}$/.test(v.avatar) ? v.avatar : undefined;
  const flag = (x: unknown) => (x === undefined || x === null ? null : typeof x === 'boolean' ? x : undefined);
  const looked = flag(v.looked);
  const gift_ready = flag(v.gift_ready);
  const you = flag(v.you);
  const locked = flag(v.locked);
  if ([avatar, looked, gift_ready, you, locked].includes(undefined)) return undefined;
  return { id: v.id, name: v.name, avatar: avatar ?? null, looked: looked ?? null, gift_ready: gift_ready ?? null, you: you === true, locked: locked === true };
}

function validSantaJoin(v: unknown): SantaInfo['join'] | undefined {
  if (!isObj(v) || (v.mode !== 'anywhere' && v.mode !== 'wifi' && v.mode !== 'none')) return undefined;
  const url = (u: unknown) => (u === undefined || u === null ? null : isHttpUrl(u) ? u : undefined);
  const anywhere_url = url(v.anywhere_url);
  const wifi_url = url(v.wifi_url);
  const kind = v.anywhere_kind === undefined || v.anywhere_kind === null ? null : v.anywhere_kind === 'https' || v.anywhere_kind === 'funnel' ? v.anywhere_kind : undefined;
  if (anywhere_url === undefined || wifi_url === undefined || kind === undefined) return undefined;
  // A quick tunnel's address changes on restart: never offered for a pocket that lasts weeks.
  if (v.quick_tunnel === true || (anywhere_url && /\.trycloudflare\.com(?:[:/]|$)/i.test(new URL(anywhere_url).host + '/'))) return undefined;
  // "From anywhere" needs an address that stays: an own https one or a Funnel one.
  if (v.mode === 'anywhere' && (!anywhere_url || !kind)) return undefined;
  return { mode: v.mode, anywhere_kind: kind, anywhere_url, wifi_url };
}

/**
 * The `santa` part of a Pocket block (card-impl.md "Shape I read"). Missing optional fields fall back; a wrong
 * type, a count above the people, or anything match-like rejects the whole block (the reply shows as markdown).
 */
export function validSanta(v: unknown): SantaInfo | undefined {
  if (!isObj(v) || !SANTA_PHASES.includes(v.phase as SantaPhase) || !isCount(v.joined)) return undefined;
  if (findMatchLike(v)) return undefined;
  const phase = v.phase as SantaPhase;
  const joined = v.joined;
  const expected = optCount(v.expected);
  const waiting = v.waiting_for ?? [];
  const people = v.people ?? [];
  const pairs = v.keep_apart ?? [];
  if (expected === undefined || !Array.isArray(waiting) || waiting.length > MAX_PEOPLE || !waiting.every(firstName)) return undefined;
  if (!Array.isArray(people) || people.length > MAX_PEOPLE || !Array.isArray(pairs) || pairs.length > MAX_PEOPLE) return undefined;
  if (!pairs.every((p) => Array.isArray(p) && p.length === 2 && firstName(p[0]) && firstName(p[1]))) return undefined;
  const rows = people.map(validPerson);
  if (rows.some((r) => !r) || rows.filter((r) => r!.you).length > 1) return undefined;
  const counts = { looked: optCount(v.looked), gifts_ready: optCount(v.gifts_ready), asks_asked: optCount(v.asks_asked), asks_answered: optCount(v.asks_answered) };
  if (Object.values(counts).includes(undefined)) return undefined;
  // Counts can't be more than the people in it, nor answers more than questions.
  if ((counts.looked ?? 0) > joined || (counts.gifts_ready ?? 0) > joined) return undefined;
  const asked = counts.asks_asked ?? null;
  const answered = counts.asks_answered ?? null;
  if (answered !== null && (asked === null || answered > asked)) return undefined;
  const reveal = v.reveal === undefined || v.reveal === null ? 'scheduled' : ['scheduled', 'shown', 'skipped', 'never'].includes(v.reveal as string) ? (v.reveal as SantaInfo['reveal']) : undefined;
  const texts = {
    draw_rule: optText(v.draw_rule, 200),
    drawn_at: optText(v.drawn_at, 60),
    reveal_at: optText(v.reveal_at, 40),
    revealed_at: optText(v.revealed_at, 60),
    deletes_on: optText(v.deletes_on, 40),
    ended_on: optText(v.ended_on, 40),
    blocked: optText(v.blocked, 300),
  };
  if (reveal === undefined || Object.values(texts).includes(undefined)) return undefined;
  if (v.can_draw !== undefined && v.can_draw !== null && typeof v.can_draw !== 'boolean') return undefined;
  const join = v.join === undefined || v.join === null ? null : validSantaJoin(v.join);
  if (join === undefined) return undefined;
  return {
    phase,
    joined,
    expected: expected ?? null,
    waiting_for: waiting as string[],
    people: rows as SantaPerson[],
    looked: counts.looked ?? null,
    gifts_ready: counts.gifts_ready ?? null,
    asks_asked: counts.asks_asked ?? null,
    asks_answered: counts.asks_answered ?? null,
    keep_apart: (pairs as string[][]).map((p) => [p[0], p[1]] as [string, string]),
    draw_rule: texts.draw_rule ?? null,
    drawn_at: texts.drawn_at ?? null,
    reveal,
    reveal_at: texts.reveal_at ?? null,
    revealed_at: texts.revealed_at ?? null,
    deletes_on: texts.deletes_on ?? null,
    ended_on: texts.ended_on ?? null,
    can_draw: phase === 'collecting' && v.can_draw === true,
    blocked: phase === 'collecting' ? texts.blocked ?? null : null,
    join,
  };
}

/** The one holo: every Santa ticked "I've got the gift", shown on Reveal day and Ended. */
export const earnedAllReady = (s: Pick<SantaInfo, 'phase' | 'joined' | 'gifts_ready'>): boolean =>
  (s.phase === 'reveal_day' || s.phase === 'ended') && s.joined > 0 && s.gifts_ready === s.joined;

/** Buttons whose santa action isn't named on the box yet (card-impl.md "Disabled until"). */
export const SANTA_LATER = {
  dates: 'Comes when Santa can change dates on the box',
} as const;

// ---------------------------------------------------------------------------------------------------
// Words

const NUM = new Intl.NumberFormat('en');
const num = (n: number) => NUM.format(n);
const usd = (n: number) => `$${n < 0.01 && n > 0 ? '0.01' : n.toFixed(2)}`;
const names = (list: string[]) => (list.length <= 1 ? list.join('') : `${list.slice(0, -1).join(', ')} and ${list[list.length - 1]}`);

export function santaBudgetLine(b: Pick<PocketBlock, 'budget'>): string {
  const tail = ' The draw and everything after it use none.';
  if (!b.budget) return `Bot calls to set this up aren't counted yet.${tail}`;
  const { calls, usd: spent } = b.budget;
  return `About ${num(calls)} bot ${calls === 1 ? 'call' : 'calls'} to set this up, about ${usd(spent)}.${tail}`;
}

const keptLine = (s: SantaInfo, why: boolean) =>
  s.deletes_on
    ? `Kept until ${s.deletes_on}${why ? ` (7 days after the ${s.reveal === 'scheduled' || s.reveal === 'shown' ? 'reveal' : 'gift day'})` : ''}, then deleted.`
    : 'Kept until 7 days after the gift day, then deleted.';

function rulesLine(s: SantaInfo): string {
  const p = s.keep_apart;
  const apart = p.length === 0 ? '' : p.length === 1 ? `, ${p[0][0]} and ${p[0][1]} apart` : p.length === 2 ? ', both pairs apart' : `, all ${p.length} pairs apart`;
  return `Rules kept: nobody drew themselves${apart}. Each person sees only their own match.`;
}

function asksLine(s: SantaInfo, phase: SantaPhase): string | null {
  if (s.asks_asked === null) return null;
  if (s.asks_asked === 0) return 'No Santa questions yet. You see the count, never the words.';
  const q = `${num(s.asks_asked)} Santa ${s.asks_asked === 1 ? 'question' : 'questions'} asked`;
  const a = s.asks_answered === null ? '' : s.asks_answered === s.asks_asked ? (s.asks_asked === 1 ? ', answered' : ', all answered') : `, ${num(s.asks_answered)} answered`;
  return phase === 'drawn' ? `${q}${a}. You see the count, never the words.` : `${q}${a}`;
}

/** A reset code's expiry in the person's own time: "Tue, Dec 8 at 9:41 PM". */
export function expiryText(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return 'for 24 hours';
  const day = new Intl.DateTimeFormat('en', { weekday: 'short', month: 'short', day: 'numeric' }).format(d);
  const time = new Intl.DateTimeFormat('en', { hour: 'numeric', minute: '2-digit' }).format(d);
  return `until ${day} at ${time}`;
}

function inviteText(name: string, when: string | null, url: string): string {
  return `You're invited to ${name}${when ? ` (${when})` : ''}. Open this on your phone, pick a creature and a 4-digit PIN: ${url}`;
}

// ---------------------------------------------------------------------------------------------------
// Small parts

/** The kit's pixel gift (pa10a card-1440.html `.kittile`, 9×8 grid). currentColor only. */
const GiftGlyph: React.FC = () => {
  const r: Array<[number, number, number, number]> = [[2, 0, 2, 1], [5, 0, 2, 1], [3, 1, 1, 1], [5, 1, 1, 1], [0, 2, 9, 1], [0, 3, 9, 1], [1, 4, 3, 1], [5, 4, 3, 1], [1, 5, 3, 1], [5, 5, 3, 1], [1, 6, 3, 1], [5, 6, 3, 1], [1, 7, 7, 1]];
  return (
    <svg viewBox="0 0 9 8" width="22" height="22" aria-hidden="true" data-glyph="gift" style={{ shapeRendering: 'crispEdges' }}>
      {r.map(([x, y, w, h]) => (
        <rect key={`${x}-${y}`} x={x} y={y} width={w} height={h} fill="currentColor" />
      ))}
    </svg>
  );
};

/** A guest's creature, as they picked it on the page. */
export const GuestCreature: React.FC<{ avatar: string | null; size?: number }> = ({ avatar, size = 22 }) => (
  <svg viewBox="5 2.5 16 17" width={size} height={size} aria-hidden="true" data-creature={avatar ?? 'antenna'} style={{ imageRendering: 'pixelated', shapeRendering: 'crispEdges' }}>
    {renderCreature(guestCreature(avatar)).filter((r) => (r as React.ReactElement).key !== 'shadow')}
  </svg>
);

const SantaChip: React.FC<{ s: SantaInfo }> = ({ s }) => {
  switch (s.phase) {
    case 'collecting':
      return (
        <span className="kw-chip kw-chip-info">
          <Users size={12} aria-hidden="true" />
          Collecting names
        </span>
      );
    case 'drawn':
      return (
        <span className="kw-chip kw-chip-ok">
          <Check size={12} aria-hidden="true" />
          Drawn
        </span>
      );
    case 'reveal_day':
      // The state the host glances for, like the quiz's Live chip. White on sky-600 is 5.1:1.
      return (
        <span className="kw-chip border-sky-400 bg-sky-600 font-semibold text-white" data-chip="reveal-day">
          <span className="kw-status-dot bg-white" aria-hidden="true" />
          Reveal day
        </span>
      );
    case 'ended':
      return <span className="kw-chip">{s.ended_on ? `Ended ${s.ended_on}` : 'Ended'}</span>;
  }
};

const AllReady: React.FC<{ n: number }> = ({ n }) => (
  <Sticker tone="holo" size="sm" ariaLabel={`All ready: all ${n} gifts ticked`}>
    All ready
  </Sticker>
);

// ---------------------------------------------------------------------------------------------------
// How people join (state 1 and 1b) and the compact box after the draw

type JoinMode = 'anywhere' | 'wifi';
const portOf = (url: string | null) => {
  try {
    return url ? new URL(url).port || null : null;
  } catch {
    return null;
  }
};

/** The 5-step How-to for Tailscale Funnel, frontend copy (card-impl.md "How-to copy"). */
export const howToSteps = (port: string | null): { text: string; cmd?: string }[] => [
  { text: 'Install Tailscale on this Mac from tailscale.com and sign in. The personal plan is free.' },
  { text: 'In the Tailscale admin page, turn on HTTPS certificates and allow Funnel for this Mac.' },
  { text: 'Open Terminal and run:', cmd: `tailscale funnel --bg ${port ?? '<the port in the Wi-Fi link>'}` },
  { text: 'Ask Santa to check the address again. This card then shows From anywhere and a link that stays the same.' },
  { text: 'Send that link to the family chat. It keeps working whenever this Mac is on.' },
];

const HowTo: React.FC<{ port: string | null }> = ({ port }) => {
  const [open, setOpen] = useState(false);
  const id = useId();
  return (
    <div className="flex flex-col gap-2.5">
      <button type="button" className={`${GHOST} self-start`} aria-expanded={open} aria-controls={id} onClick={() => setOpen(!open)} data-action="how-to">
        How to
        <ChevronDown size={14} aria-hidden="true" className={`transition-transform duration-150 motion-reduce:transition-none ${open ? 'rotate-180' : ''}`} />
      </button>
      {open && (
        <ol id={id} className="kw-fade-in flex list-decimal flex-col gap-1.5 pl-5 text-[15px] leading-[22px] text-zinc-200 marker:text-zinc-400 sm:text-[13px] sm:leading-5" data-how-to>
          {howToSteps(port).map((st) => (
            <li key={st.text}>
              {st.text}
              {st.cmd && (
                <code className="mt-1 block w-fit max-w-full overflow-x-auto whitespace-nowrap rounded-md border border-hairline bg-surface-main px-2 py-1 font-mono text-[13px] text-zinc-100">{st.cmd}</code>
              )}
            </li>
          ))}
        </ol>
      )}
    </div>
  );
};

const NoQuickLink: React.FC = () => (
  <p className="text-[13px] leading-5 text-zinc-400 sm:text-meta">A quick link isn't offered: it changes when the Mac restarts, and the link in your chat would break.</p>
);

const SantaJoinBox: React.FC<{ block: PocketBlock; s: SantaInfo }> = ({ block, s }) => {
  const join = s.join;
  const anywhere = join?.mode === 'anywhere' && !!join.anywhere_url;
  const [mode, setMode] = useState<JoinMode>(anywhere ? 'anywhere' : 'wifi');
  const radios = useRef<Array<HTMLButtonElement | null>>([]);
  const labelId = useId();
  const url = !join ? null : mode === 'anywhere' && anywhere ? join.anywhere_url : join.wifi_url;
  const modes: JoinMode[] = ['anywhere', 'wifi'];
  const onKey = (e: React.KeyboardEvent) => {
    if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(e.key)) return;
    e.preventDefault();
    const next = modes[(modes.indexOf(mode) + 1) % 2];
    setMode(next);
    radios.current[modes.indexOf(next)]?.focus();
  };
  return (
    <div className="flex flex-col gap-4 rounded-2xl border border-sky-800/70 bg-surface-rail/40 p-3.5 sm:flex-row" data-join="full" data-join-mode={anywhere ? 'anywhere' : join?.wifi_url ? 'wifi' : 'none'}>
      {url ? (
        <div key={url} className="kw-fade-in">
          <Qr url={url} size="full" name={block.name} />
        </div>
      ) : null}
      <div className="flex min-w-0 flex-1 flex-col gap-2.5">
        <div id={labelId} className="text-[13px] font-semibold text-zinc-200 sm:text-meta">
          How people join
        </div>
        {anywhere ? (
          <>
            <div role="radiogroup" aria-labelledby={labelId} onKeyDown={onKey} className="inline-flex self-start rounded-xl border border-hairline-strong bg-surface-rail p-1">
              {modes.map((m, i) => {
                const on = m === mode;
                const Icon = m === 'wifi' ? Wifi : Globe;
                return (
                  <button
                    key={m}
                    ref={(el) => (radios.current[i] = el)}
                    type="button"
                    role="radio"
                    aria-checked={on}
                    tabIndex={on ? 0 : -1}
                    onClick={() => setMode(m)}
                    className={`flex min-h-[44px] flex-none items-center justify-center gap-1.5 whitespace-nowrap rounded-lg px-3 text-meta font-semibold kw-focus sm:min-h-[32px] ${
                      on ? 'bg-sky-600 text-white' : 'text-zinc-300 hover:text-zinc-100'
                    }`}
                  >
                    <Icon size={14} aria-hidden="true" />
                    {m === 'wifi' ? 'Same Wi-Fi' : 'From anywhere'}
                  </button>
                );
              })}
            </div>
            {mode === 'anywhere' ? (
              <p className="text-[15px] leading-[22px] text-zinc-100 sm:text-body">
                {join?.anywhere_kind === 'https' ? 'Uses your own https address.' : "Uses your Mac's Tailscale Funnel address."} It stays the same, so the link can sit in the family chat for
                weeks. No app, no account.
              </p>
            ) : (
              <p className="text-[15px] leading-[22px] text-zinc-100 sm:text-body">People on your Wi-Fi open the page; anyone else can't reach it. Pick From anywhere for the family chat.</p>
            )}
            <p className="text-[13px] leading-5 text-zinc-400 sm:text-meta">Works while this Mac is on. When it sleeps, the page says so and tries again by itself; nothing is lost.</p>
            {url ? <LinkField url={url} /> : <p className="text-[13px] leading-5 text-zinc-400 sm:text-meta">Your box hasn't said its Wi-Fi address yet.</p>}
          </>
        ) : (
          <>
            {join?.wifi_url ? (
              <p className="flex items-start gap-2 text-[15px] leading-[22px] text-zinc-200 sm:text-[13px] sm:leading-5">
                <Wifi size={15} className={`${ICON} text-sky-300`} aria-hidden="true" />
                <span>
                  <strong className="font-semibold text-zinc-50">Same Wi-Fi only, for now.</strong> People on your Wi-Fi open the page; anyone else can't reach it.
                </span>
              </p>
            ) : (
              <p className="flex items-start gap-2 text-[15px] leading-[22px] text-zinc-200 sm:text-[13px] sm:leading-5" data-no-link>
                <Wifi size={15} className={`${ICON} text-zinc-500`} aria-hidden="true" />
                <span>
                  <strong className="font-semibold text-zinc-50">Only this Mac can open the page right now.</strong> Phones need an address they can reach.
                </span>
              </p>
            )}
            <div className="flex flex-col gap-2.5 rounded-xl border border-hairline bg-surface-rail/60 p-3" data-callout="no-stable-address">
              <p className="flex items-start gap-2 text-[15px] leading-[22px] text-zinc-200 sm:text-[13px] sm:leading-5">
                <Info size={15} className={`${ICON} text-zinc-400`} aria-hidden="true" />
                <span>No address that stays the same is set up on this Mac. To let people join from anywhere for weeks, turn on Tailscale Funnel: free, about 5 minutes.</span>
              </p>
              <HowTo port={portOf(join?.wifi_url ?? null)} />
              <NoQuickLink />
            </div>
            {url && <LinkField url={url} />}
          </>
        )}
      </div>
    </div>
  );
};

/** After the draw: a smaller code; the link lasts the month. */
const SantaCompactJoin: React.FC<{ block: PocketBlock; s: SantaInfo }> = ({ block, s }) => {
  const url = s.join?.anywhere_url ?? s.join?.wifi_url ?? null;
  if (!url) return null;
  const Icon = s.join?.anywhere_url ? Globe : Wifi;
  return (
    <div className="flex items-center gap-3.5 rounded-2xl border border-sky-800/70 bg-surface-rail/40 p-3.5" data-join="compact">
      <Qr url={url} size="compact" name={block.name} />
      <div className="flex min-w-0 flex-1 flex-col gap-2">
        <p className="flex items-start gap-1.5 text-[13px] leading-5 text-zinc-300 sm:text-meta">
          <Icon size={14} className="mt-0.5 shrink-0 text-sky-300" aria-hidden="true" />
          Same link for the whole month. Lost it? Scan again, then type your name and PIN.
        </p>
        <LinkField url={url} />
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------------------------------
// People and PINs (inside the card, not a modal)

type PeopleStep = { id: string; kind: 'menu' | 'reset' | 'remove' | 'code'; code?: ResetCode } | null;

const statusText = (p: SantaPerson, phase: SantaPhase) => {
  const parts: string[] = [];
  if (p.you) parts.push('you');
  if (p.locked) parts.push('locked: needs a reset code');
  else if (phase === 'collecting') parts.push('joined');
  else if (p.looked === false) parts.push('not looked yet');
  else if (p.looked === true) parts.push(p.gift_ready ? 'seen · gift ready' : 'seen');
  return parts.join(' · ');
};

const CodeBox: React.FC<{ name: string; code: ResetCode; onDone: () => void }> = ({ name, code, onDone }) => {
  const [copied, setCopied] = useState<'yes' | 'failed' | null>(null);
  const done = useRef<HTMLButtonElement | null>(null);
  useEffect(() => done.current?.focus(), []);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(code.code);
      setCopied('yes');
    } catch {
      setCopied('failed');
    }
  };
  return (
    <div className="kw-fade-in flex flex-col gap-3 rounded-xl border border-sky-700 bg-sky-500/[0.06] p-3.5" role="group" aria-label={`${name}'s one-time code`} data-reset-code>
      <p className="text-[15px] font-semibold text-zinc-50 sm:text-body">{name}'s one-time code</p>
      <span className="self-start rounded-lg border border-dashed border-sky-500 px-3.5 py-1.5 font-mono text-[28px] font-bold leading-9 tracking-[0.2em] text-zinc-50" data-code>
        {code.code}
      </span>
      <p className="text-[15px] leading-[22px] text-zinc-300 sm:text-[13px] sm:leading-5">
        Send it to {name} yourself. It works once, {expiryText(code.expires_at)}. On the page, {name} types the name "{name}", this code, then a new PIN.
      </p>
      <div className="flex flex-wrap items-center gap-2">
        <button type="button" onClick={copy} className={GHOST} data-action="copy-code">
          {copied === 'yes' ? <Check size={14} aria-hidden="true" /> : <Copy size={14} aria-hidden="true" />}
          <span aria-live="polite">{copied === 'yes' ? 'Copied' : copied === 'failed' ? "Couldn't copy" : 'Copy code'}</span>
        </button>
        <button ref={done} type="button" onClick={onDone} className={`inline-flex items-center justify-center rounded-xl px-3 font-semibold text-sky-300 hover:bg-white/[0.04] kw-focus ${BTN}`} data-action="code-done">
          Done
        </button>
      </div>
    </div>
  );
};

const PersonConfirm: React.FC<{ p: SantaPerson; kind: 'reset' | 'remove'; busy: boolean; error: string | null; onYes: () => void; onNo: () => void }> = ({ p, kind, busy, error, onYes, onNo }) => {
  const yes = useRef<HTMLButtonElement | null>(null);
  useEffect(() => yes.current?.focus(), []);
  return (
    <div
      role="alertdialog"
      aria-label={kind === 'reset' ? `Reset ${p.name}'s PIN?` : `Remove ${p.name}?`}
      onKeyDown={(e) => e.key === 'Escape' && !busy && onNo()}
      className={`kw-fade-in flex flex-col gap-3 rounded-xl border p-3.5 ${kind === 'remove' ? 'border-rose-300/35 bg-rose-300/[0.06]' : 'border-sky-700 bg-sky-500/[0.06]'}`}
      data-confirm={kind}
    >
      <p className="text-[15px] font-semibold text-zinc-50 sm:text-body">{kind === 'reset' ? `Reset ${p.name}'s PIN?` : `Remove ${p.name}?`}</p>
      <p className="text-[15px] leading-[22px] text-zinc-300 sm:text-[13px] sm:leading-5">
        {kind === 'reset'
          ? `${p.name} gets a one-time code to pick a new PIN. ${p.name}'s match, wish list and questions stay as they are. You won't see any of them. The old PIN stops working, and ${p.name}'s page says you reset it.`
          : `${p.name} leaves before the draw and can't join again with this name. Nothing is drawn yet, so nobody's match changes.`}
      </p>
      {error && (
        <p role="alert" className="flex items-start gap-2 text-[15px] text-bad sm:text-[13px]">
          <AlertTriangle size={15} className={ICON} aria-hidden="true" />
          {error}
        </p>
      )}
      <div className="grid grid-cols-1 gap-2 sm:flex sm:flex-wrap">
        <button
          ref={yes}
          type="button"
          onClick={onYes}
          disabled={busy}
          className={
            kind === 'reset'
              ? `${PRIMARY} disabled:opacity-60`
              : `inline-flex items-center justify-center gap-1.5 rounded-xl bg-rose-600 px-3.5 font-semibold text-white hover:bg-rose-700 disabled:opacity-60 kw-focus ${BTN}`
          }
          data-action={kind === 'reset' ? 'reset-yes' : 'remove-yes'}
        >
          {busy ? <Loader2 size={14} className="motion-safe:animate-spin" aria-hidden="true" /> : kind === 'reset' ? <KeyRound size={14} aria-hidden="true" /> : <UserMinus size={14} aria-hidden="true" />}
          {kind === 'reset' ? 'Reset and show the code' : `Remove ${p.name}`}
        </button>
        <button type="button" onClick={onNo} disabled={busy} className={GHOST}>
          Cancel
        </button>
      </div>
    </div>
  );
};

export const PeoplePanel: React.FC<{ block: PocketBlock; s: SantaInfo }> = ({ block, s }) => {
  const [step, setStep] = useState<PeopleStep>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [removed, setRemoved] = useState<string[]>([]);
  const [note, setNote] = useState<string | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const panelRef = useRef<HTMLElement | null>(null);
  const focusAfter = useFocusAfterRender();
  const people = s.people.filter((p) => !removed.includes(p.id));
  const canRemove = s.phase === 'collecting';
  const back = (id: string) => {
    setStep(null);
    setError(null);
    focusAfter(() => [...(panelRef.current?.querySelectorAll<HTMLButtonElement>('[data-person-menu]') ?? [])].find((b) => b.dataset.personMenu === id));
  };
  useEffect(() => {
    if (step?.kind !== 'menu') return;
    menuRef.current?.querySelector<HTMLButtonElement>('[role=menuitem]')?.focus();
    const off = (e: MouseEvent) => !menuRef.current?.contains(e.target as Node) && setStep(null);
    document.addEventListener('mousedown', off);
    return () => document.removeEventListener('mousedown', off);
  }, [step]);
  const reset = async (p: SantaPerson) => {
    setBusy(true);
    setError(null);
    try {
      const code = await pocketApi.resetPin(block.code, p.id);
      setStep({ id: p.id, kind: 'code', code });
    } catch (e) {
      setError(`Couldn't reset it: ${e instanceof PocketApiError ? e.message : 'something went wrong.'} Try again.`);
    } finally {
      setBusy(false);
    }
  };
  const remove = async (p: SantaPerson) => {
    setBusy(true);
    setError(null);
    try {
      await pocketApi.removePlayer(block.code, p.id);
      setRemoved((r) => [...r, p.id]);
      setNote(`${p.name} is removed. Nothing was drawn, so nobody's match changes.`);
      setStep(null);
    } catch (e) {
      setError(`Couldn't remove ${p.name}: ${e instanceof PocketApiError ? e.message : 'something went wrong.'} Try again.`);
    } finally {
      setBusy(false);
    }
  };
  const menuKeys = (e: React.KeyboardEvent, id: string) => {
    const items = [...(menuRef.current?.querySelectorAll<HTMLButtonElement>('[role=menuitem]') ?? [])];
    const i = items.indexOf(document.activeElement as HTMLButtonElement);
    if (e.key === 'Escape') {
      e.preventDefault();
      back(id);
    } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      items[(i + (e.key === 'ArrowDown' ? 1 : items.length - 1)) % items.length]?.focus();
    }
  };
  return (
    <section ref={panelRef} className="rounded-2xl border border-hairline-strong bg-surface-rail/40" aria-label="People and PINs" data-people>
      <header className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-0.5 border-b border-hairline px-3.5 py-2.5">
        <h4 className="text-[15px] font-semibold text-zinc-100 sm:text-meta">
          People and PINs · {num(people.length)}
        </h4>
        <span className="text-[13px] font-medium text-zinc-400 sm:text-2xs">Matches never show here</span>
      </header>
      {note && (
        <p role="status" className="flex items-start gap-2 border-b border-hairline px-3.5 py-2 text-[15px] text-ok sm:text-[13px]">
          <Check size={15} className={ICON} aria-hidden="true" />
          {note}
        </p>
      )}
      {people.length === 0 ? (
        <p className="px-3.5 py-3 text-[15px] text-zinc-400 sm:text-[13px]">Nobody has joined yet. Copy the invite to the family chat.</p>
      ) : (
        <ul>
          {people.map((p) => {
            const open = step?.id === p.id;
            return (
              <li key={p.id} className={`border-b border-hairline last:rounded-b-2xl last:border-b-0 ${open ? 'bg-white/[0.03]' : ''}`} data-person={p.name}>
                <div className="relative flex min-h-[56px] items-center gap-3 px-3.5 sm:min-h-[48px]">
                  <GuestCreature avatar={p.avatar} size={24} />
                  {/* Phone: the status sits under the name, so a long status never squeezes the name out. */}
                  <span className="flex min-w-0 flex-1 flex-col py-1.5 sm:flex-row sm:items-center sm:gap-3 sm:py-0">
                    <span className="min-w-0 truncate text-[15px] font-semibold text-zinc-100 sm:flex-1 sm:text-body">{p.name}</span>
                    <span className={`text-[13px] sm:shrink-0 sm:text-2xs ${p.locked ? 'text-zinc-200' : 'text-zinc-400'}`} data-status>
                      {statusText(p, s.phase)}
                    </span>
                  </span>
                  <button
                    type="button"
                    aria-label={`More for ${p.name}`}
                    aria-haspopup="menu"
                    aria-expanded={open && step?.kind === 'menu'}
                    data-person-menu={p.id}
                    onClick={() => setStep(open && step?.kind === 'menu' ? null : { id: p.id, kind: 'menu' })}
                    className={`flex h-11 w-11 shrink-0 items-center justify-center rounded-lg text-zinc-300 hover:bg-white/[0.05] kw-focus sm:h-9 sm:w-9 ${open ? 'border border-hairline-strong' : ''}`}
                  >
                    <Ellipsis size={16} aria-hidden="true" />
                  </button>
                  {open && step?.kind === 'menu' && (
                    <div
                      ref={menuRef}
                      role="menu"
                      aria-label={`${p.name}`}
                      onKeyDown={(e) => menuKeys(e, p.id)}
                      className="kw-fade-in absolute right-3 top-[calc(100%-4px)] z-10 flex min-w-[180px] flex-col rounded-xl border border-hairline-strong bg-surface-raised p-1 shadow-card"
                    >
                      <button type="button" role="menuitem" onClick={() => setStep({ id: p.id, kind: 'reset' })} className="flex min-h-[44px] items-center gap-2 rounded-lg px-3 text-left text-[15px] text-zinc-100 hover:bg-white/[0.05] kw-focus sm:min-h-[36px] sm:text-meta">
                        <KeyRound size={14} aria-hidden="true" />
                        Reset PIN
                      </button>
                      {canRemove ? (
                        <button type="button" role="menuitem" onClick={() => setStep({ id: p.id, kind: 'remove' })} className="flex min-h-[44px] items-center gap-2 rounded-lg px-3 text-left text-[15px] text-bad hover:bg-rose-300/10 kw-focus sm:min-h-[36px] sm:text-meta">
                          <UserMinus size={14} aria-hidden="true" />
                          Remove
                        </button>
                      ) : (
                        <button
                          type="button"
                          role="menuitem"
                          aria-disabled="true"
                          title="Only before the draw: after it, removing someone would change a match."
                          onClick={(e) => e.preventDefault()}
                          className={`flex min-h-[44px] items-center gap-2 rounded-lg px-3 text-left text-[15px] text-zinc-300 kw-focus sm:min-h-[36px] sm:text-meta ${LATER_CLS}`}
                        >
                          <UserMinus size={14} aria-hidden="true" />
                          Remove (before the draw only)
                        </button>
                      )}
                    </div>
                  )}
                </div>
                {open && (step?.kind === 'reset' || step?.kind === 'remove') && (
                  <div className="px-3.5 pb-3.5">
                    <PersonConfirm p={p} kind={step.kind} busy={busy} error={error} onYes={() => (step.kind === 'reset' ? reset(p) : remove(p))} onNo={() => back(p.id)} />
                  </div>
                )}
                {open && step?.kind === 'code' && step.code && (
                  <div className="px-3.5 pb-3.5">
                    <CodeBox name={p.name} code={step.code} onDone={() => back(p.id)} />
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
};

// ---------------------------------------------------------------------------------------------------
// The card's rows per phase

const CreatureStack: React.FC<{ people: SantaPerson[] }> = ({ people }) =>
  people.length > 0 ? (
    <span className="flex -space-x-1.5" aria-hidden="true" data-stack>
      {people.slice(0, 9).map((p) => (
        <span key={p.id} className="rounded-md bg-surface-raised">
          <GuestCreature avatar={p.avatar} size={22} />
        </span>
      ))}
    </span>
  ) : null;

const PairChip: React.FC<{ pair: [string, string] }> = ({ pair }) => (
  <span className="inline-flex min-h-[32px] items-center gap-1.5 rounded-full border border-hairline-strong bg-surface-rail px-2.5 text-[13px] font-semibold text-zinc-100 sm:min-h-[28px] sm:text-meta">
    {pair[0]}
    <X size={12} className="text-zinc-400" aria-label="kept apart from" />
    {pair[1]}
  </span>
);

const SantaBody: React.FC<{ block: PocketBlock; s: SantaInfo; ask: (t: string) => void; closed: boolean }> = ({ block, s, ask, closed }) => {
  const of = (n: number | null) => (s.expected ?? s.joined) > 0 && n !== null ? `${num(n)} of ${num(s.expected ?? s.joined)}` : null;
  switch (s.phase) {
    case 'collecting': {
      const joinedOf = of(s.joined);
      return (
        <>
          <ul className="flex flex-col gap-2">
            <li className="flex flex-col gap-2">
              <span className="flex items-start gap-2 text-[15px] leading-[22px] text-zinc-200 sm:text-[13px] sm:leading-5">
                <Users size={15} className={`${ICON} text-sky-300`} aria-hidden="true" />
                <span className="min-w-0 flex-1">
                  <strong className="font-semibold text-zinc-50">{joinedOf ?? num(s.joined)}</strong> joined
                  {s.waiting_for.length > 0 ? ` · waiting for ${s.waiting_for.length <= 3 ? names(s.waiting_for) : `${s.waiting_for.length} more`}` : ''}
                </span>
                <CreatureStack people={s.people} />
              </span>
              {s.expected ? <Bar value={s.joined / s.expected} label={`${s.joined} of ${s.expected} joined`} /> : null}
            </li>
            {s.draw_rule && <Row icon={<Shuffle size={15} />}>The draw: {s.draw_rule}.</Row>}
            <li className="flex flex-wrap items-center gap-2 text-[15px] leading-[22px] text-zinc-200 sm:text-[13px] sm:leading-5" data-kept-apart>
              <span className="flex items-center gap-2 sm:flex-1">
                <HeartCrack size={15} className="shrink-0 text-sky-300" aria-hidden="true" />
                {s.keep_apart.length > 0 ? 'Kept apart:' : 'Nobody is kept apart yet.'}
              </span>
              {/* Phone: the pairs get their own row. */}
              <span className="flex basis-full flex-wrap items-center gap-2 sm:basis-auto">
                {s.keep_apart.map((p) => (
                  <PairChip key={`${p[0]}-${p[1]}`} pair={p} />
                ))}
                <button
                  type="button"
                  onClick={() => ask(`In ${block.name}, keep  and  apart in the draw`)}
                  className="inline-flex min-h-[44px] items-center gap-1.5 rounded-lg px-2 text-[15px] font-semibold text-sky-300 hover:bg-white/[0.04] kw-focus sm:min-h-[28px] sm:text-meta"
                  data-action="add-pair"
                >
                  <Plus size={14} aria-hidden="true" />
                  Add a pair
                </button>
              </span>
            </li>
            <Row icon={<EyeOff size={15} />}>Nobody sees anyone else's match, you included. You join on your phone like everyone else.</Row>
          </ul>
          {s.blocked && (
            <Callout tone="warn">
              <p className="flex items-start gap-2">
                <AlertTriangle size={15} className={`${ICON} text-warn`} aria-hidden="true" />
                <span>
                  <strong className="font-semibold text-zinc-50">Santa can't draw yet.</strong> {s.blocked}
                </span>
              </p>
            </Callout>
          )}
          {!closed && <SantaJoinBox block={block} s={s} />}
        </>
      );
    }
    case 'drawn': {
      const notYet = s.people.filter((p) => p.looked === false).map((p) => p.name);
      const lookedOf = of(s.looked);
      const readyOf = of(s.gifts_ready);
      const asks = asksLine(s, 'drawn');
      return (
        <>
          <ul className="flex flex-col gap-2">
            <Row icon={<CheckCircle2 size={15} />}>{s.drawn_at ? `Drawn ${s.drawn_at} · everyone has their match` : 'Drawn · everyone has their match'}</Row>
            <Row icon={<ShieldCheck size={15} />}>{rulesLine(s)}</Row>
            {lookedOf && (
              <Row icon={<Eye size={15} />}>
                <strong className="font-semibold text-zinc-50">{lookedOf}</strong> have looked at their match
                {notYet.length > 0 ? ` · ${notYet.length <= 2 ? `${names(notYet)} ${notYet.length === 1 ? "hasn't" : "haven't"} yet` : `${notYet.length} haven't yet`}` : ''}
              </Row>
            )}
            {readyOf && (
              <li className="flex flex-col gap-2">
                <span className="flex items-start gap-2 text-[15px] leading-[22px] text-zinc-200 sm:text-[13px] sm:leading-5">
                  <Gift size={15} className={`${ICON} text-sky-300`} aria-hidden="true" />
                  <span>
                    <strong className="font-semibold text-zinc-50">{readyOf}</strong> gifts ready
                  </span>
                </span>
                <Bar value={(s.gifts_ready ?? 0) / Math.max(1, s.expected ?? s.joined)} label={`${readyOf} gifts ready`} />
              </li>
            )}
            {asks && <Row icon={<MessageSquare size={15} />}>{asks}</Row>}
            <Row icon={<Lock size={15} />}>{keptLine(s, true)}</Row>
          </ul>
          {!closed && <SantaCompactJoin block={block} s={s} />}
        </>
      );
    }
    case 'reveal_day': {
      const all = earnedAllReady(s);
      const readyOf = of(s.gifts_ready);
      const asks = asksLine(s, 'reveal_day');
      return (
        <ul className="flex flex-col gap-2">
          {readyOf && (
            <li className="flex flex-col gap-2">
              <span className="flex items-start gap-2 text-[15px] leading-[22px] text-zinc-200 sm:text-[13px] sm:leading-5">
                <Gift size={15} className={`${ICON} text-sky-300`} aria-hidden="true" />
                <span className="min-w-0 flex-1">
                  <strong className="font-semibold text-zinc-50">{readyOf}</strong> gifts ready{all ? ': every Santa ticked it' : ''}
                </span>
                {all && <AllReady n={s.joined} />}
              </span>
              {!all && <Bar value={(s.gifts_ready ?? 0) / Math.max(1, s.expected ?? s.joined)} label={`${readyOf} gifts ready`} />}
            </li>
          )}
          <Row icon={<PartyPopper size={15} />}>
            {s.reveal === 'scheduled'
              ? `At ${s.reveal_at ?? 'the reveal time'} everyone's page shows who was whose Santa. Until then it stays a secret.`
              : 'No reveal: who was whose Santa stays a secret.'}
          </Row>
          {asks && <Row icon={<MessageSquare size={15} />}>{asks}</Row>}
          <Row icon={<Lock size={15} />}>{keptLine(s, false)}</Row>
        </ul>
      );
    }
    case 'ended': {
      const all = earnedAllReady(s);
      const facts = [
        s.reveal === 'shown' ? (s.revealed_at ? `Revealed ${s.revealed_at}` : 'Revealed') : 'Kept secret, no reveal',
        `${num(s.joined)} ${s.joined === 1 ? 'person' : 'people'}`,
        s.gifts_ready !== null ? `${num(s.gifts_ready)} ${s.gifts_ready === 1 ? 'gift' : 'gifts'}` : null,
      ].filter(Boolean);
      return (
        <ul className="flex flex-col gap-2">
          <Row icon={<PartyPopper size={15} />} aside={all ? <AllReady n={s.joined} /> : undefined}>
            {facts.join(' · ')}
          </Row>
          <Row icon={<Lock size={15} />}>The link is closed. Names, wish lists and questions are deleted {s.deletes_on ? `on ${s.deletes_on}` : 'when the keep time ends'}.</Row>
        </ul>
      );
    }
  }
};

// ---------------------------------------------------------------------------------------------------
// Actions

type SantaConfirmKind = 'draw' | 'reveal' | 'skip' | 'stop' | 'end' | 'delete';
const CONFIRM: Record<SantaConfirmKind, { q: (n: string) => string; text: (b: PocketBlock, s: SantaInfo) => string; yes: string; no: string; danger: boolean }> = {
  draw: {
    q: () => 'Draw names now?',
    text: (_b, s) =>
      `${s.expected && s.joined < s.expected ? `Only ${num(s.joined)} of ${num(s.expected)} are in; anyone later has to be added by you. ` : ''}Everyone then opens their own match. Not even you can see who has whom.`,
    yes: 'Draw names',
    no: 'Not yet',
    danger: false,
  },
  reveal: { q: () => 'Show the reveal now?', text: () => "Everyone's page shows who was whose Santa straight away, instead of at the set time.", yes: 'Show it now', no: 'Wait', danger: false },
  skip: { q: () => 'Skip the reveal?', text: () => 'Who was whose Santa stays a secret for good. Everything is still deleted on the same day.', yes: 'Skip the reveal', no: 'Keep it', danger: false },
  stop: { q: (n) => `Stop ${n}?`, text: () => 'The link closes and nobody can join or open their page. Names and wish lists are deleted on the deletion date.', yes: 'Stop it', no: 'Keep going', danger: true },
  end: { q: (n) => `End ${n}?`, text: () => 'The link closes now, before the reveal. Names, wish lists and questions are deleted on the deletion date.', yes: 'End it', no: 'Keep going', danger: true },
  delete: { q: () => 'Delete it all now?', text: () => "Names, wish lists, questions and the matches are deleted from this Mac now, not on the deletion date. This can't be undone.", yes: 'Delete it all', no: 'Keep until then', danger: true },
};

const SantaConfirm: React.FC<{ block: PocketBlock; s: SantaInfo; kind: SantaConfirmKind; busy: boolean; onYes: () => void; onNo: () => void }> = ({ block, s, kind, busy, onYes, onNo }) => {
  const yes = useRef<HTMLButtonElement | null>(null);
  useEffect(() => yes.current?.focus(), []);
  const c = CONFIRM[kind];
  return (
    <div
      role="alertdialog"
      aria-label={c.q(block.name)}
      onKeyDown={(e) => e.key === 'Escape' && !busy && onNo()}
      className={`kw-fade-in flex flex-col gap-3 rounded-xl border p-3.5 ${c.danger ? 'border-rose-300/35 bg-rose-300/[0.06]' : 'border-sky-700 bg-sky-500/[0.06]'}`}
      data-confirm={kind}
    >
      <p className="text-[15px] leading-[22px] text-zinc-100 sm:text-body">
        <strong className="font-semibold">{c.q(block.name)}</strong> {c.text(block, s)}
      </p>
      <div className="grid grid-cols-1 gap-2 sm:flex sm:flex-wrap">
        <button
          ref={yes}
          type="button"
          onClick={onYes}
          disabled={busy}
          className={c.danger ? `inline-flex items-center justify-center gap-1.5 rounded-xl bg-rose-600 px-3.5 font-semibold text-white hover:bg-rose-700 disabled:opacity-60 kw-focus ${BTN}` : `${PRIMARY} disabled:opacity-60`}
          data-action={`${kind}-yes`}
        >
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

const DONE_TEXT: Record<SantaConfirmKind, string> = {
  draw: 'Drawn. Everyone can open their own match now. Ask Santa for a fresh card to see who has looked.',
  reveal: "Shown. Everyone's page now says who was whose Santa.",
  skip: 'Skipped. Who was whose Santa stays a secret.',
  stop: 'Stopped. The link is closed.',
  end: 'Ended. The link is closed.',
  delete: 'Deleted. Nothing of it is left on this Mac.',
};

const SantaActions: React.FC<{
  block: PocketBlock;
  s: SantaInfo;
  people: boolean;
  onPeople: () => void;
  ask: (t: string) => void;
  onConfirm: (k: SantaConfirmKind) => void;
}> = ({ block, s, people, onPeople, ask, onConfirm }) => {
  const join = s.join;
  const url = join?.anywhere_url ?? join?.wifi_url ?? null;
  const host = s.people.find((p) => p.you)?.name ?? null;
  const [copied, setCopied] = useState<'yes' | 'failed' | null>(null);
  useEffect(() => {
    if (!copied) return;
    const t = setTimeout(() => setCopied(null), 2000);
    return () => clearTimeout(t);
  }, [copied]);
  const grid = 'grid grid-cols-2 gap-2 sm:flex sm:flex-wrap sm:items-center';
  const full = 'col-span-2 sm:col-span-1';
  const stopCls = `${STOP} col-span-2 justify-self-center sm:ml-auto`;
  const noLink = 'The link shows here once your box has one';
  const pageLink = (label: string, primary: boolean, half = false) =>
    url ? (
      <a href={url} target="_blank" rel="noopener noreferrer" className={primary ? `${PRIMARY} ${full}` : half ? `kw-btn-ghost ${HALF}` : `${GHOST} ${full}`} data-action="my-page">
        <Smartphone size={14} aria-hidden="true" />
        {label}
      </a>
    ) : (
      <button type="button" aria-disabled="true" title={noLink} aria-description={noLink} onClick={(e) => e.preventDefault()} className={`${primary ? `${PRIMARY} ${full}` : half ? `kw-btn-ghost ${HALF}` : `${GHOST} ${full}`} ${LATER_CLS}`}>
        <Smartphone size={14} aria-hidden="true" />
        {label}
      </button>
    );
  const peopleBtn = (
    <button type="button" onClick={onPeople} aria-expanded={people} className={`kw-btn-ghost ${HALF}`} data-action="people">
      <Users size={14} aria-hidden="true" />
      People
    </button>
  );
  switch (s.phase) {
    case 'collecting': {
      const copy = async () => {
        if (!url) return;
        try {
          await navigator.clipboard.writeText(inviteText(block.name, block.when, url));
          setCopied('yes');
        } catch {
          setCopied('failed');
        }
      };
      const drawWhy = s.blocked ? "Santa can't draw with these rules yet" : s.joined < 3 ? 'A draw needs at least 3 people' : "The box hasn't said a draw can run yet";
      return (
        <div className={grid}>
          {url ? (
            <button type="button" onClick={copy} className={`${PRIMARY} ${full}`} data-action="copy-invite">
              {copied === 'yes' ? <Check size={14} aria-hidden="true" /> : <Copy size={14} aria-hidden="true" />}
              <span aria-live="polite">{copied === 'yes' ? 'Invite copied' : copied === 'failed' ? "Couldn't copy" : 'Copy invite'}</span>
            </button>
          ) : (
            <button type="button" aria-disabled="true" title={noLink} aria-description={noLink} onClick={(e) => e.preventDefault()} className={`${PRIMARY} ${full} ${LATER_CLS}`}>
              <Copy size={14} aria-hidden="true" />
              Copy invite
            </button>
          )}
          {pageLink(host ? `Join as ${host}` : 'Join on your phone', false, true)}
          {peopleBtn}
          {s.can_draw ? (
            <button type="button" onClick={() => onConfirm('draw')} className={`kw-btn-ghost ${HALF} col-span-1 sm:ml-auto`} data-action="draw">
              <Shuffle size={14} aria-hidden="true" />
              Draw now
            </button>
          ) : (
            <LaterButton half icon={<Shuffle size={14} aria-hidden="true" />} why={drawWhy} className="col-span-1 sm:ml-auto">
              Draw now
            </LaterButton>
          )}
          <button type="button" onClick={() => onConfirm('stop')} className={`${STOP} col-span-1 justify-self-stretch`} data-action="stop">
            <Square size={14} aria-hidden="true" />
            Stop
          </button>
        </div>
      );
    }
    case 'drawn':
      return (
        <div className={grid}>
          {pageLink('Open my Santa page', true)}
          {peopleBtn}
          <LaterButton half icon={<CalendarDays size={14} aria-hidden="true" />} why={SANTA_LATER.dates}>
            Change dates
          </LaterButton>
          <button type="button" onClick={() => onConfirm('stop')} className={stopCls} data-action="stop">
            <Square size={14} aria-hidden="true" />
            Stop
          </button>
        </div>
      );
    case 'reveal_day': {
      const canReveal = s.reveal === 'scheduled' || s.reveal === 'skipped';
      return (
        <div className={grid}>
          {canReveal ? (
            <button type="button" onClick={() => onConfirm('reveal')} className={`${PRIMARY} ${full}`} data-action="reveal">
              <PartyPopper size={14} aria-hidden="true" />
              Show the reveal now
            </button>
          ) : (
            <LaterButton icon={<PartyPopper size={14} aria-hidden="true" />} why="You chose to keep it secret forever" className={full}>
              Show the reveal now
            </LaterButton>
          )}
          {s.reveal === 'scheduled' ? (
            <button type="button" onClick={() => onConfirm('skip')} className={`kw-btn-ghost ${HALF}`} data-action="skip">
              <EyeOff size={14} aria-hidden="true" />
              Skip the reveal
            </button>
          ) : (
            <LaterButton half icon={<EyeOff size={14} aria-hidden="true" />} why="There's no reveal to skip">
              Skip the reveal
            </LaterButton>
          )}
          {pageLink('My Santa page', false, true)}
          <button type="button" onClick={() => onConfirm('end')} className={stopCls} data-action="end">
            <Square size={14} aria-hidden="true" />
            End
          </button>
        </div>
      );
    }
    case 'ended':
      return (
        <div className={grid}>
          <button type="button" className={`${PRIMARY} ${full}`} onClick={() => ask(`Run ${block.name} again next year with the same people`)} data-action="again">
            <RotateCcw size={14} aria-hidden="true" />
            Run it again next year
          </button>
          <button
            type="button"
            onClick={() => onConfirm('delete')}
            className={`inline-flex items-center justify-center gap-1.5 rounded-xl px-3 font-medium text-zinc-300 hover:bg-white/[0.04] hover:text-zinc-100 kw-focus ${BTN} col-span-2 sm:ml-auto`}
            data-action="delete"
          >
            <Trash2 size={14} aria-hidden="true" />
            Delete it all now
          </button>
        </div>
      );
  }
};

// ---------------------------------------------------------------------------------------------------
// The card

type Done = { kind: 'ok' | 'error'; text: string; closes: boolean } | null;

export const SantaCard: React.FC<{ block: PocketBlock; ask: (t: string) => void }> = ({ block, ask }) => {
  const s = block.santa!;
  const [people, setPeople] = useState(false);
  const [confirm, setConfirm] = useState<SantaConfirmKind | null>(null);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<Done>(null);
  const opener = useRef<HTMLDivElement | null>(null);
  const focusAfter = useFocusAfterRender();
  const act = async (kind: SantaConfirmKind) => {
    setBusy(true);
    try {
      if (kind === 'draw') await pocketApi.hostAction(block.code, 'draw_now');
      else if (kind === 'reveal') await pocketApi.hostAction(block.code, 'reveal_now');
      else if (kind === 'skip') await pocketApi.hostAction(block.code, 'skip_reveal');
      else if (kind === 'delete') await pocketApi.remove(block.code);
      else await pocketApi.end(block.code);
      setDone({ kind: 'ok', text: DONE_TEXT[kind], closes: kind === 'stop' || kind === 'end' || kind === 'delete' });
      setConfirm(null);
    } catch (e) {
      setDone({ kind: 'error', text: `Couldn't do that: ${e instanceof PocketApiError ? e.message : 'something went wrong.'} Try again.`, closes: false });
    } finally {
      setBusy(false);
    }
  };
  const close = () => {
    const k = confirm;
    setConfirm(null);
    focusAfter(() => opener.current?.querySelector<HTMLButtonElement>(`[data-action="${k}"]`));
  };
  const closed = done?.kind === 'ok' && done.closes;
  const showsPeople = people && s.phase !== 'ended' && !closed;
  return (
    <section
      className="pocket-card relative flex max-w-[680px] flex-col gap-4 overflow-hidden rounded-2xl border border-sky-800 bg-surface-raised p-4 shadow-card sm:p-[18px]"
      data-pocket={block.state}
      data-santa={s.phase}
      aria-label={`${block.name}, pocket app`}
    >
      <div className="pointer-events-none absolute inset-x-0 top-0 h-28 bg-gradient-to-b from-sky-500/[0.07] to-transparent" aria-hidden="true" />
      <header className="relative flex items-start gap-3">
        <span className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl border border-sky-700 bg-sky-950 text-sky-200" aria-hidden="true">
          <GiftGlyph />
        </span>
        <div className="min-w-0 flex-1">
          <div className="text-2xs font-semibold text-sky-300">Pocket app · Secret Santa</div>
          <h3 className="text-[20px] font-bold leading-[26px] text-zinc-50 sm:text-[19px]">{block.name}</h3>
          {block.when && <p className="text-[15px] text-zinc-300 sm:text-[13px]">{block.when}</p>}
          {/* Phone: the chip sits under the "when" line, so the title doesn't squeeze. */}
          <span className="mt-1.5 block sm:hidden">
            <SantaChip s={s} />
          </span>
        </div>
        <span className="hidden shrink-0 sm:block">
          <SantaChip s={s} />
        </span>
      </header>
      <div className="relative flex flex-col gap-4">
        <SantaBody block={block} s={s} ask={ask} closed={closed} />
        {showsPeople && <PeoplePanel block={block} s={s} />}
        {done ? (
          <p role="status" className={`flex items-start gap-2 text-[15px] sm:text-[13px] ${done.kind === 'ok' ? 'text-ok' : 'text-bad'}`} data-done={done.kind}>
            {done.kind === 'ok' ? <Check size={15} className={ICON} aria-hidden="true" /> : <AlertTriangle size={15} className={ICON} aria-hidden="true" />}
            {done.text}
          </p>
        ) : null}
        {confirm ? (
          <SantaConfirm block={block} s={s} kind={confirm} busy={busy} onYes={() => act(confirm)} onNo={close} />
        ) : done?.kind === 'ok' ? null : (
          <div ref={opener}>
            <SantaActions block={block} s={s} people={showsPeople} onPeople={() => setPeople(!people)} ask={ask} onConfirm={setConfirm} />
          </div>
        )}
      </div>
      <footer className="relative -mx-4 -mb-4 flex items-start gap-2 border-t border-hairline px-4 py-3 text-meta text-zinc-400 sm:-mx-[18px] sm:-mb-[18px] sm:px-[18px]" data-budget>
        <Zap size={13} className="mt-0.5 shrink-0" aria-hidden="true" />
        <span>{santaBudgetLine(block)}</span>
      </footer>
    </section>
  );
};
