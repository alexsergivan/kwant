// Pocket apps (pa4): the Pocket card in chat in its 7 states, and the results card (pa0a/design.md §1,
// §6, "Rules this design keeps", "Data the app doesn't have yet"). A `pocket` block is always a slot
// block, drawn after the Quizmaster's reply. The QR code never shows until a dry run passed (ready and
// live only). Holo is earned: "Checked" on ready, "Winners" on ended and results; everything else is a
// plain chip. Buttons whose route isn't built yet (pa4b: the remote, editing, the big screen, keeping
// results) render disabled with a short reason.

import React, { useEffect, useId, useRef, useState } from 'react';
import {
  AlertTriangle,
  ArrowLeftRight,
  CalendarClock,
  Check,
  CheckCircle2,
  Globe,
  Hourglass,
  ListChecks,
  Loader2,
  Lock,
  Monitor,
  Pause,
  Pencil,
  Play,
  Repeat,
  Archive,
  Smartphone,
  Square,
  Trophy,
  Users,
  Wifi,
  Zap,
} from 'lucide-react';
import type { PocketBlock, PocketJoinMode, PocketMiss, PocketResults, PocketRound, PocketState, PocketTeam } from '../../smartResults';
import { inRange, isHttpUrl, isIndex, isNum, isObj, isText, optText, parseCommon } from '../../smartResults';
import { Sticker } from '../Sticker';
import { BTN, Bar, Callout, GHOST, ICON, LATER_CLS, LaterButton, LinkField, NoLinkYet, PRIMARY, Qr, Row, STOP, WARN_FILL } from './pocketParts';
import { TEAM_PALETTES, isPalette, paletteCreature, renderCreature } from '../ModernAvatar';
import { pocketApi, PocketApiError } from '../../pocketApi';
import { SantaCard, findMatchLike, validSanta } from './PocketSanta';
import type { RendererProps } from './registry';

// ---------------------------------------------------------------------------------------------------
// Validator

export const POCKET_STATES: PocketState[] = ['writing', 'checking', 'dry_run_stopped', 'ready', 'live', 'ended'];
const MAX_COUNT = 100_000;
const isCount = (v: unknown): v is number => isIndex(v) && v <= MAX_COUNT;

/** Absent or null is null; anything else must pass `ok`, or the whole block is off (undefined). */
function opt<T>(v: unknown, ok: (v: unknown) => T | undefined): T | null | undefined {
  return v === undefined || v === null ? null : ok(v);
}

const validProgress = (v: unknown) => (isObj(v) && isCount(v.done) && isCount(v.total) && v.total > 0 && v.done <= v.total ? { done: v.done, total: v.total } : undefined);

function validRound(v: unknown): PocketRound | undefined {
  if (!isObj(v) || !isText(v.label, 40) || !isCount(v.written) || !isCount(v.total) || v.written > v.total) return undefined;
  const topic = optText(v.topic, 80);
  if (topic === undefined) return undefined;
  return { label: v.label, topic, written: v.written, total: v.total };
}

function validChecks(v: unknown) {
  if (!isObj(v) || !isCount(v.matched) || !isCount(v.total) || v.matched > v.total) return undefined;
  const swapped = v.swapped ?? [];
  if (!inRange(swapped, 0, 20) || !swapped.every((s) => isText(s, 12))) return undefined;
  return { matched: v.matched, total: v.total, swapped: swapped as string[] };
}

function validDryRun(v: unknown) {
  if (!isObj(v) || !isCount(v.players) || typeof v.passed !== 'boolean') return undefined;
  const stopped_at = optText(v.stopped_at, 80);
  const what_happened = optText(v.what_happened, 400);
  const what_im_doing = optText(v.what_im_doing, 400);
  if (stopped_at === undefined || what_happened === undefined || what_im_doing === undefined) return undefined;
  return { players: v.players, passed: v.passed, stopped_at, what_happened, what_im_doing };
}

const validScore = (v: unknown) => (isObj(v) && isText(v.name, 40) && isNum(v.points) && Math.abs(v.points) <= 10_000_000 ? { name: v.name, points: v.points } : undefined);

function validLive(v: unknown) {
  if (!isObj(v) || !isCount(v.players) || !isCount(v.teams)) return undefined;
  const leader = opt(v.leader, validScore);
  const now = optText(v.now, 120);
  const palettes = v.palettes ?? [];
  if (leader === undefined || now === undefined || !inRange(palettes, 0, 12) || !palettes.every((p) => typeof p === 'string' && isPalette(p))) return undefined;
  return { players: v.players, teams: v.teams, leader, now, palettes: palettes as string[] };
}

function validJoin(v: unknown) {
  if (!isObj(v) || (v.mode !== 'wifi' && v.mode !== 'anywhere')) return undefined;
  const url = (u: unknown) => (u === undefined || u === null ? null : isHttpUrl(u) ? u : undefined);
  const wifi_url = url(v.wifi_url);
  const anywhere_url = url(v.anywhere_url);
  if (wifi_url === undefined || anywhere_url === undefined) return undefined;
  return { mode: v.mode as PocketJoinMode, wifi_url, anywhere_url };
}

function validEnded(v: unknown) {
  if (!isObj(v) || !isText(v.at, 40)) return undefined;
  const winner = opt(v.winner, validScore);
  const stats = optText(v.stats, 200);
  const deletes_on = optText(v.deletes_on, 40);
  if (winner === undefined || stats === undefined || deletes_on === undefined) return undefined;
  return { at: v.at, winner, stats, deletes_on };
}

const validBudget = (v: unknown) => (isObj(v) && isCount(v.calls) && isNum(v.usd) && v.usd >= 0 && v.usd < 10_000 ? { calls: v.calls, usd: v.usd } : undefined);

function validTeam(v: unknown): PocketTeam | undefined {
  if (!isObj(v) || !isText(v.name, 40) || !isCount(v.players) || !isNum(v.points) || Math.abs(v.points) > 10_000_000) return undefined;
  const palette = v.palette === undefined || v.palette === null ? null : typeof v.palette === 'string' && isPalette(v.palette) ? v.palette : undefined;
  const members = v.members ?? [];
  if (palette === undefined || !inRange(members, 0, 12) || !members.every((m) => isText(m, 24))) return undefined;
  return { name: v.name, players: v.players, points: v.points, palette, members: members as string[] };
}

function validMiss(v: unknown): PocketMiss | undefined {
  if (!isObj(v) || !isText(v.label, 12) || !isText(v.question, 200) || !isNum(v.pct_right) || v.pct_right < 0 || v.pct_right > 100) return undefined;
  const answer = optText(v.answer, 80);
  if (answer === undefined) return undefined;
  return { label: v.label, question: v.question, answer, pct_right: v.pct_right };
}

function validResults(v: unknown): PocketResults | undefined {
  if (!isObj(v) || !inRange(v.teams, 1, 12) || !inRange(v.missed ?? [], 0, 3)) return undefined;
  const teams = v.teams.map(validTeam);
  const missed = ((v.missed ?? []) as unknown[]).map(validMiss);
  if (teams.some((t) => !t) || missed.some((m) => !m)) return undefined;
  const n = (x: unknown) => (x === undefined || x === null ? null : isCount(x) ? x : undefined);
  const margin = n(v.margin);
  const near = n(v.near_misses);
  const accepted = n(v.near_misses_accepted);
  if (margin === undefined || near === undefined || accepted === undefined) return undefined;
  if (near !== null && accepted !== null && accepted > near) return undefined;
  return { teams: teams as PocketTeam[], margin, missed: missed as PocketMiss[], near_misses: near, near_misses_accepted: accepted };
}

/**
 * A Pocket card (pa4, impl.md "PocketBlock"). Slot only. Unknown or missing optional fields fall back
 * as design.md says; a wrong type anywhere, or a contradiction (ready without a passed dry run, results
 * before the end), rejects the block, and the reply shows as markdown.
 */
export function validPocket(raw: unknown): PocketBlock | null {
  const b = parseCommon(raw, 'pocket');
  if (!b || b.source.kind !== 'slot') return null;
  if (typeof b.code !== 'string' || !/^[A-Za-z0-9_-]{3,32}$/.test(b.code)) return null;
  if (typeof b.kit !== 'string' || !/^[a-z][a-z0-9_]{0,23}$/.test(b.kit) || !isText(b.name, 80)) return null;
  if (!POCKET_STATES.includes(b.state as PocketState)) return null;
  const state = b.state as PocketState;
  if (b.paused !== undefined && b.paused !== null && typeof b.paused !== 'boolean') return null;
  const when = optText(b.when, 160);
  const progress = opt(b.progress, validProgress);
  const rounds = b.rounds ?? [];
  if (!inRange(rounds, 0, 8)) return null;
  const roundList = rounds.map(validRound);
  const checks = opt(b.checks, validChecks);
  const dry_run = opt(b.dry_run, validDryRun);
  const eta = optText(b.eta, 120);
  const opens_line = optText(b.opens_line, 200);
  const live = opt(b.live, validLive);
  const join = opt(b.join, validJoin);
  const ended = opt(b.ended, validEnded);
  const budget = opt(b.budget, validBudget);
  const results = opt(b.results, validResults);
  if ([when, progress, checks, dry_run, eta, opens_line, live, join, ended, budget, results].includes(undefined) || roundList.some((r) => !r)) return null;
  // A ready or live pocket passed its dry run; a stopped one didn't. Results only exist at the end.
  if (state === 'ready' && dry_run?.passed !== true) return null;
  if (state === 'dry_run_stopped' && dry_run?.passed === true) return null;
  if (results && state !== 'ended') return null;
  // Secret Santa (pa10a): the kit carries `santa`, nothing else does; it's live until its phase ends, and no
  // part of the block may carry a match or a question's words.
  const santa = b.santa === undefined || b.santa === null ? null : validSanta(b.santa);
  if (santa === undefined || (b.kit === 'santa') !== !!santa || (santa && findMatchLike(raw))) return null;
  if (santa && state !== (santa.phase === 'ended' ? 'ended' : 'live')) return null;
  if (santa && results) return null;
  // Santa joins live in `santa.join` (no quick tunnel); a quiz-style top-level join never rides along.
  if (santa && b.join !== undefined && b.join !== null) return null;
  return {
    id: b.id,
    type: 'pocket',
    title: b.title,
    source: b.source,
    provenance: b.provenance,
    code: b.code,
    kit: b.kit,
    name: b.name,
    when: when ?? null,
    state,
    paused: state === 'live' && b.paused === true,
    progress: progress ?? null,
    rounds: roundList as PocketRound[],
    checks: checks ?? null,
    dry_run: dry_run ?? null,
    eta: eta ?? null,
    opens_line: opens_line ?? null,
    live: live ?? null,
    join: join ?? null,
    ended: ended ?? null,
    budget: budget ?? null,
    results: results ?? null,
    santa,
  };
}

/** The QR code (and the join box) only once a dry run passed: ready and live. */
export const showsQr = (b: Pick<PocketBlock, 'state' | 'dry_run'>): boolean =>
  b.state === 'ready' ? b.dry_run?.passed === true : b.state === 'live';

/** The "Checked" holo: every answer has a source and the dry run passed. */
export const earnedChecked = (b: PocketBlock): boolean =>
  b.state === 'ready' && b.dry_run?.passed === true && !!b.checks && b.checks.total > 0 && b.checks.matched === b.checks.total;

// ---------------------------------------------------------------------------------------------------
// Words

const NUM = new Intl.NumberFormat('en');
const num = (n: number) => NUM.format(n);
const usd = (n: number) => `$${n < 0.01 && n > 0 ? '0.01' : n.toFixed(2)}`;
const KIT_LABEL: Record<string, string> = { quiz: 'Quiz', santa: 'Secret Santa' };
export const kitLabel = (kit: string) => KIT_LABEL[kit] ?? kit.charAt(0).toUpperCase() + kit.slice(1).replace(/_/g, ' ');

export function budgetLine(b: PocketBlock): string {
  const tail = b.kit === 'quiz' ? ' Live play uses none unless a near miss needs judging.' : '';
  if (!b.budget) return `Bot calls so far aren't counted yet.${tail}`;
  const { calls, usd: spent } = b.budget;
  return `About ${num(calls)} bot ${calls === 1 ? 'call' : 'calls'} so far, about ${usd(spent)} on your account.${tail}`;
}

/** Hover reasons for buttons whose route comes later (impl.md "Disabled until pa4b"). */
export const LATER = {
  remote: 'Comes with the remote',
  edit: 'Comes with question editing',
  bigScreen: 'Comes with the big screen',
  pause: 'Comes with the remote',
  keep: 'Comes with the remote',
  editRound1: 'Edit opens when round 1 is written',
} as const;

// ---------------------------------------------------------------------------------------------------
// Small parts

/** The kit's pixel "?" (quiz) or trophy (results), in the 44 px tile. currentColor only. */
const PixelGlyph: React.FC<{ kind: 'quiz' | 'trophy' }> = ({ kind }) => {
  // The kit glyph from pa0a card-1440.html `.kittile` (9×10 grid, 2-px strokes).
  const q: Array<[number, number, number, number]> = [[2, 0, 5, 1], [1, 1, 2, 1], [6, 1, 2, 1], [0, 2, 2, 1], [7, 2, 2, 1], [7, 3, 2, 1], [6, 4, 2, 1], [4, 5, 3, 1], [4, 6, 2, 1], [4, 8, 2, 1], [4, 9, 2, 1]];
  const t: Array<[number, number, number, number]> = [[2, 1, 6, 1], [1, 2, 1, 2], [8, 2, 1, 2], [2, 2, 6, 3], [3, 5, 4, 1], [4, 6, 2, 1], [3, 7, 4, 1], [2, 8, 6, 1]];
  return (
    <svg viewBox={kind === 'quiz' ? '0 0 9 10' : '0 0 10 10'} width={kind === 'quiz' ? 20 : 22} height="22" aria-hidden="true" data-glyph={kind} style={{ shapeRendering: 'crispEdges' }}>
      {(kind === 'quiz' ? q : t).map(([x, y, w, h]) => (
        <rect key={`${x}-${y}-${w}`} x={x} y={y} width={w} height={h} fill="currentColor" />
      ))}
    </svg>
  );
};

const Tile: React.FC<{ kind: 'quiz' | 'trophy' }> = ({ kind }) => (
  <span className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl border border-sky-700 bg-sky-950 text-sky-200" aria-hidden="true">
    <PixelGlyph kind={kind} />
  </span>
);

/** A team's creature: a plain guest creature in the team's palette. */
export const TeamCreature: React.FC<{ palette: string; size?: number }> = ({ palette, size = 20 }) => (
  <svg viewBox="5 2.5 16 17" width={size} height={size} aria-hidden="true" style={{ imageRendering: 'pixelated', shapeRendering: 'crispEdges' }}>
    {renderCreature(paletteCreature(palette)).filter((r) => (r as React.ReactElement).key !== 'shadow')}
  </svg>
);
const teamPalette = (t: PocketTeam, i: number) => t.palette ?? TEAM_PALETTES[i % TEAM_PALETTES.length];

const StateChip: React.FC<{ block: PocketBlock }> = ({ block }) => {
  const spin = <Loader2 size={12} className="motion-safe:animate-spin" aria-hidden="true" />;
  switch (block.state) {
    case 'writing':
      return <span className="kw-chip kw-chip-info">{spin}Writing</span>;
    case 'checking':
      return <span className="kw-chip kw-chip-info">{spin}Checking</span>;
    case 'dry_run_stopped':
      return (
        <span className="kw-chip kw-chip-warn">
          <AlertTriangle size={12} aria-hidden="true" />
          Dry run stopped
        </span>
      );
    case 'ready':
      return (
        <span className="kw-chip kw-chip-ok">
          <Check size={12} aria-hidden="true" />
          Ready
        </span>
      );
    case 'live':
      return (
        <span className="flex flex-wrap justify-start gap-1.5 sm:justify-end">
          {block.paused && (
            <span className="kw-chip">
              <Pause size={12} aria-hidden="true" />
              Paused
            </span>
          )}
          {/* pa0a pocket.css `.chip.live`: the state the host glances for. White on sky-600 is 5.1:1. */}
          <span className="kw-chip border-sky-400 bg-sky-600 font-semibold text-white" data-chip="live">
            <span className={`kw-status-dot bg-white ${block.paused ? '' : 'kw-pulse'}`} aria-hidden="true" />
            Live
          </span>
        </span>
      );
    case 'ended':
      return <span className="kw-chip">{block.ended ? `Ended ${block.ended.at}` : 'Ended'}</span>;
  }
};


export { WARN_FILL };

// ---------------------------------------------------------------------------------------------------
// Join box

const JOIN_TEXT: Record<PocketJoinMode, { label: string; body: string; limit: string }> = {
  wifi: {
    label: 'Same Wi-Fi',
    body: "Friends on your Wi-Fi scan the code and play in their phone's browser. No app, no account.",
    limit: "Phones on mobile data can't reach your Wi-Fi. At a pub, pick From anywhere.",
  },
  anywhere: {
    label: 'From anywhere',
    body: "Anyone with the code joins in their phone's browser, on Wi-Fi or mobile data. No app, no account.",
    limit: 'Uses a free Cloudflare quick link: a new address each time and no uptime promise. Fine for a quiz night of up to 60 players.',
  },
};
const urlFor = (join: NonNullable<PocketBlock['join']>, mode: PocketJoinMode) => (mode === 'wifi' ? join.wifi_url : join.anywhere_url);


const JoinBox: React.FC<{ block: PocketBlock; mode: PocketJoinMode; setMode: (m: PocketJoinMode) => void }> = ({ block, mode, setMode }) => {
  const join = block.join;
  const url = join ? urlFor(join, mode) : null;
  const radios = useRef<Array<HTMLButtonElement | null>>([]);
  const labelId = useId();
  const modes: PocketJoinMode[] = ['wifi', 'anywhere'];
  const onKey = (e: React.KeyboardEvent) => {
    if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(e.key)) return;
    e.preventDefault();
    const next = modes[(modes.indexOf(mode) + 1) % 2];
    setMode(next);
    radios.current[modes.indexOf(next)]?.focus();
  };
  return (
    <div className="flex flex-col gap-4 rounded-2xl border border-sky-800/70 bg-surface-rail/40 p-3.5 sm:flex-row" data-join="full">
      {url ? (
        <div key={url} className="kw-fade-in">
          <Qr url={url} size="full" name={block.name} />
        </div>
      ) : null}
      <div className="flex min-w-0 flex-1 flex-col gap-2.5">
        <div id={labelId} className="text-[13px] font-semibold text-zinc-200 sm:text-meta">
          How friends join
        </div>
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
                {JOIN_TEXT[m].label}
              </button>
            );
          })}
        </div>
        <p className="text-[15px] leading-[22px] text-zinc-100 sm:text-body">{JOIN_TEXT[mode].body}</p>
        <p className="text-[13px] leading-5 text-zinc-400 sm:text-meta">{JOIN_TEXT[mode].limit}</p>
        {url ? <LinkField url={url} /> : <NoLinkYet mode={mode} />}
      </div>
    </div>
  );
};

/** Live: a smaller code for late arrivals, the mode in words, the link. */
const CompactJoin: React.FC<{ block: PocketBlock }> = ({ block }) => {
  const join = block.join;
  const mode = join?.mode ?? 'wifi';
  const url = join ? urlFor(join, mode) ?? urlFor(join, mode === 'wifi' ? 'anywhere' : 'wifi') : null;
  const shownMode: PocketJoinMode = join && url === join.anywhere_url && url !== join.wifi_url ? 'anywhere' : mode;
  const Icon = shownMode === 'wifi' ? Wifi : Globe;
  return (
    <div className="flex items-center gap-3.5 rounded-2xl border border-sky-800/70 bg-surface-rail/40 p-3.5" data-join="compact">
      {url && <Qr url={url} size="compact" name={block.name} />}
      <div className="flex min-w-0 flex-1 flex-col gap-2">
        <p className="flex items-center gap-1.5 text-[13px] text-zinc-300 sm:text-meta">
          <Icon size={14} className="shrink-0 text-sky-300" aria-hidden="true" />
          {JOIN_TEXT[shownMode].label} · late arrivals can still scan in
        </p>
        {url ? <LinkField url={url} /> : <NoLinkYet mode={shownMode} />}
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------------------------------
// Ask before acting: Stop and End quiz

type Confirm = { kind: 'stop' | 'end' } | null;
type Done = { kind: 'ok'; text: string } | { kind: 'error'; text: string } | null;

const ConfirmBox: React.FC<{ block: PocketBlock; kind: 'stop' | 'end'; busy: boolean; onYes: () => void; onNo: () => void }> = ({ block, kind, busy, onYes, onNo }) => {
  const yes = useRef<HTMLButtonElement | null>(null);
  useEffect(() => yes.current?.focus(), []);
  const leader = block.live?.leader;
  const text =
    kind === 'end'
      ? `Scores so far become the final results${leader ? `: ${leader.name} win with ${num(leader.points)}` : ''}. Everyone sees the winners, the link closes, and the results go to your chat.`
      : `${block.name} stops here: nothing more gets written or checked, and nobody can join. You can ask Quizmaster for a new one any time.`;
  return (
    <div
      role="alertdialog"
      aria-label={kind === 'end' ? 'End the quiz?' : `Stop ${block.name}?`}
      onKeyDown={(e) => e.key === 'Escape' && !busy && onNo()}
      className="kw-fade-in flex flex-col gap-3 rounded-xl border border-rose-300/35 bg-rose-300/[0.06] p-3.5"
      data-confirm={kind}
    >
      <p className="text-[15px] leading-[22px] text-zinc-100 sm:text-body">
        <strong className="font-semibold">{kind === 'end' ? 'End the quiz now?' : `Stop ${block.name}?`}</strong> {text}
      </p>
      <div className="grid grid-cols-1 gap-2 sm:flex sm:flex-wrap">
        <button
          ref={yes}
          type="button"
          onClick={onYes}
          disabled={busy}
          className={`inline-flex items-center justify-center gap-1.5 rounded-xl bg-rose-600 px-3.5 font-semibold text-white hover:bg-rose-700 disabled:opacity-60 kw-focus ${BTN}`}
        >
          {busy ? <Loader2 size={14} className="motion-safe:animate-spin" aria-hidden="true" /> : <Square size={14} aria-hidden="true" />}
          {kind === 'end' ? 'End and show winners' : 'Stop the quiz'}
        </button>
        <button type="button" onClick={onNo} disabled={busy} className={GHOST}>
          {kind === 'end' ? 'Keep playing' : 'Keep going'}
        </button>
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------------------------------
// The card's body per state

const roundStatus = (r: PocketRound, i: number, rounds: PocketRound[]) => {
  if (r.written >= r.total) return { icon: <Check size={13} className="text-ok" aria-hidden="true" />, text: `${r.written} of ${r.total} written` };
  if (r.written > 0) return { icon: <Loader2 size={13} className="motion-safe:animate-spin text-sky-300" aria-hidden="true" />, text: `${r.written} of ${r.total} written` };
  const firstOpen = rounds.findIndex((x) => x.written < x.total);
  return { icon: null, text: i === firstOpen || (firstOpen >= 0 && rounds[firstOpen].written > 0 && i === firstOpen + 1) ? 'Next' : 'Waiting' };
};

const Progress: React.FC<{ block: PocketBlock; verb: string }> = ({ block, verb }) => {
  const p = block.progress;
  if (!p) return null;
  return (
    <div className="flex flex-col gap-2">
      <p className="flex items-center gap-2 text-[15px] text-zinc-200 sm:text-[13px]">
        <Loader2 size={14} className="motion-safe:animate-spin text-sky-300" aria-hidden="true" />
        <span className="min-w-0">
          {verb} · <strong className="whitespace-nowrap font-semibold text-zinc-50">{num(p.done)} of {num(p.total)}</strong>
        </span>
      </p>
      <Bar value={p.done / p.total} label={`${verb}: ${p.done} of ${p.total}`} />
    </div>
  );
};

const CheckRow: React.FC<{ block: PocketBlock; aside?: React.ReactNode }> = ({ block, aside }) =>
  block.checks ? (
    <Row icon={<CheckCircle2 size={15} className="text-ok" />} aside={aside}>
      {block.state === 'checking' ? `${num(block.checks.matched)} answers match a source` : `${num(block.checks.matched)}/${num(block.checks.total)} answers checked against a source`}
    </Row>
  ) : null;

const Body: React.FC<{ block: PocketBlock; mode: PocketJoinMode; setMode: (m: PocketJoinMode) => void; closed?: boolean }> = ({ block, mode, setMode, closed }) => {
  // After a successful Stop or End the link is closed on the box: no QR or Copy link, even before a new block arrives.
  const qr = showsQr(block) && !closed;
  const dr = block.dry_run;
  switch (block.state) {
    case 'writing':
      return (
        <>
          <Progress block={block} verb="Writing questions" />
          {block.rounds.length > 0 && (
            <ul className="grid grid-cols-1 gap-x-6 gap-y-2.5 sm:grid-cols-2" aria-label="Rounds">
              {block.rounds.map((r, i) => {
                const s = roundStatus(r, i, block.rounds);
                return (
                  <li key={`${r.label}-${i}`} className="text-[15px] leading-[22px] sm:text-[13px] sm:leading-5">
                    <div className="text-zinc-200">
                      <strong className="font-semibold text-zinc-50">{r.label}</strong>
                      {r.topic ? ` · ${r.topic}` : ''}
                    </div>
                    <div className="flex items-center gap-1.5 text-zinc-400">
                      {s.icon}
                      {s.text}
                    </div>
                  </li>
                );
              })}
            </ul>
          )}
          <Callout>
            <strong className="font-semibold text-zinc-100">The QR code comes after the checks.</strong> Every answer is checked against a source, then{' '}
            {dr?.players ?? 30} pretend players run the whole quiz.{block.eta ? ` ${block.eta}` : ''}
          </Callout>
        </>
      );
    case 'checking':
      return (
        <>
          <Progress block={block} verb="Checking answers against sources" />
          <ul className="flex flex-col gap-2">
            <CheckRow block={block} />
            {block.checks && block.checks.swapped.length > 0 && (
              <Row icon={<ArrowLeftRight size={15} />} tone="warn">
                {block.checks.swapped.length} swapped: no source agreed on the answer ({block.checks.swapped.join(', ')}). Their replacements are being checked.
              </Row>
            )}
            <Row icon={<ListChecks size={15} />} muted>
              Dry run with {dr?.players ?? 30} pretend players: after the checks
            </Row>
          </ul>
          <Callout>
            <strong className="font-semibold text-zinc-100">The QR code comes after the checks.</strong>
            {block.eta ? ` ${block.eta}` : ''}
          </Callout>
        </>
      );
    case 'dry_run_stopped':
      return (
        <>
          <ul className="flex flex-col gap-2">
            <CheckRow block={block} />
            <Row icon={<AlertTriangle size={15} />} tone="warn">
              {dr?.stopped_at ? `Dry run stopped at ${dr.stopped_at}` : 'Dry run stopped before the end'}
            </Row>
          </ul>
          <Callout tone="warn">
            <p>
              <strong className="font-semibold text-zinc-50">What happened:</strong> {dr?.what_happened ?? "a pretend player couldn't finish the quiz."}
            </p>
            <p>
              <strong className="font-semibold text-zinc-50">What I'm doing:</strong>{' '}
              {dr?.what_im_doing ?? "looking into it. I'll say here when the dry run runs again."}
            </p>
          </Callout>
          <Callout>No QR code until a dry run passes, so nobody joins a quiz that breaks.</Callout>
        </>
      );
    case 'ready':
      return (
        <>
          <ul className="flex flex-col gap-2">
            <CheckRow
              block={block}
              aside={
                earnedChecked(block) ? (
                  <Sticker tone="holo" size="sm" ariaLabel="Checked: every answer has a source and the dry run passed">
                    Checked
                  </Sticker>
                ) : undefined
              }
            />
            {dr && (
              <Row icon={<ListChecks size={15} />}>
                Dry run passed: {dr.players} pretend players, every question showed, timers ended, scores added up
              </Row>
            )}
            {block.opens_line && <Row icon={<CalendarClock size={15} />}>{block.opens_line}</Row>}
          </ul>
          {qr && <JoinBox block={block} mode={mode} setMode={setMode} />}
        </>
      );
    case 'live': {
      const l = block.live;
      return (
        <>
          <ul className="flex flex-col gap-2">
            {l && (
              <Row
                icon={<Users size={15} />}
                aside={
                  l.palettes.length > 0 ? (
                    <span className="flex -space-x-0.5" aria-hidden="true">
                      {l.palettes.slice(0, 8).map((p, i) => (
                        <TeamCreature key={`${p}-${i}`} palette={p} size={18} />
                      ))}
                    </span>
                  ) : undefined
                }
              >
                <strong className="font-semibold text-zinc-50">{num(l.players)} players</strong> in {num(l.teams)} {l.teams === 1 ? 'team' : 'teams'}
              </Row>
            )}
            {l?.leader && (
              <Row icon={<Trophy size={15} />}>
                Leading: {l.leader.name}, {num(l.leader.points)}
              </Row>
            )}
            {block.paused && (
              <Row icon={<Pause size={15} />}>The timer is paused. Phones and the big screen say "Back in a moment".</Row>
            )}
            <Row icon={<Hourglass size={15} />}>This box stays awake until the quiz ends.</Row>
          </ul>
          {qr && <CompactJoin block={block} />}
        </>
      );
    }
    case 'ended': {
      const e = block.ended;
      return (
        <ul className="flex flex-col gap-2">
          {e?.winner && (
            <Row
              icon={<Trophy size={15} />}
              aside={
                <Sticker tone="holo" size="sm" icon={<Trophy size={10} aria-hidden="true" />} ariaLabel={`Winners: ${e.winner.name}`}>
                  Winners
                </Sticker>
              }
            >
              Winners: <strong className="font-semibold text-zinc-50">{e.winner.name}</strong>, {num(e.winner.points)} points
            </Row>
          )}
          {e?.stats && <Row icon={<Users size={15} />}>{e.stats}</Row>}
          <Row icon={<Lock size={15} />}>{deletionLine(block)}</Row>
        </ul>
      );
    }
  }
};

const deletionLine = (b: PocketBlock) =>
  `The link is closed. Names and answers are deleted ${b.ended?.deletes_on ? `on ${b.ended.deletes_on}` : b.kit === 'quiz' ? '7 days after the quiz' : 'when the keep time ends'}.`;

// ---------------------------------------------------------------------------------------------------
// Actions

const Actions: React.FC<{
  block: PocketBlock;
  url: string | null;
  ask: (t: string) => void;
  onStop: () => void;
  onEnd: () => void;
}> = ({ block, url, ask, onStop, onEnd }) => {
  const stop = (
    <button type="button" onClick={onStop} className={`${STOP} col-span-2 justify-self-center sm:ml-auto`} data-action="stop">
      <Square size={14} aria-hidden="true" />
      Stop
    </button>
  );
  const editWhy = block.state === 'writing' && !(block.rounds[0] && block.rounds[0].written >= block.rounds[0].total) ? LATER.editRound1 : LATER.edit;
  const edit = (
    <LaterButton icon={<Pencil size={14} aria-hidden="true" />} why={editWhy}>
      Edit questions
    </LaterButton>
  );
  const grid = 'grid grid-cols-2 gap-2 sm:flex sm:flex-wrap sm:items-center';
  const full = 'col-span-2 sm:col-span-1';
  switch (block.state) {
    case 'writing':
    case 'checking':
      return (
        <div className={grid}>
          <span className={full}>{React.cloneElement(edit, { className: 'w-full sm:w-auto' })}</span>
          {stop}
        </div>
      );
    case 'dry_run_stopped':
      return (
        <div className={grid}>
          <button type="button" className={`${GHOST} ${full}`} onClick={() => ask(`Show me the dry run report for ${block.name}`)} data-action="report">
            <ListChecks size={14} aria-hidden="true" />
            See the dry run report
          </button>
          <span className={full}>{React.cloneElement(edit, { className: 'w-full sm:w-auto' })}</span>
          {stop}
        </div>
      );
    case 'ready':
      return (
        <div className={grid}>
          {url ? (
            <a href={url} target="_blank" rel="noopener noreferrer" className={`${PRIMARY} ${full}`} data-action="try">
              <Play size={14} aria-hidden="true" />
              Try it as a player
            </a>
          ) : (
            <button type="button" aria-disabled="true" title="The link shows here once your box has one" className={`${PRIMARY} ${full} ${LATER_CLS}`} onClick={(e) => e.preventDefault()}>
              <Play size={14} aria-hidden="true" />
              Try it as a player
            </button>
          )}
          <LaterButton half icon={<Monitor size={14} aria-hidden="true" />} why={LATER.bigScreen}>
            Big screen
          </LaterButton>
          {React.cloneElement(edit, { half: true })}
          {stop}
        </div>
      );
    case 'live':
      return (
        <div className={grid}>
          <button type="button" aria-disabled="true" title={LATER.remote} aria-description={LATER.remote} data-later={LATER.remote} onClick={(e) => e.preventDefault()} className={`${PRIMARY} ${full} ${LATER_CLS}`}>
            <Smartphone size={14} aria-hidden="true" />
            Open the remote
          </button>
          <LaterButton half icon={<Monitor size={14} aria-hidden="true" />} why={LATER.bigScreen}>
            Big screen
          </LaterButton>
          <LaterButton half icon={<Pause size={14} aria-hidden="true" />} why={LATER.pause}>
            {block.paused ? 'Resume' : 'Pause'}
          </LaterButton>
          <button type="button" onClick={onEnd} className={`${STOP} col-span-2 justify-self-center sm:ml-auto`} data-action="end">
            <Square size={14} aria-hidden="true" />
            End quiz
          </button>
        </div>
      );
    case 'ended':
      return (
        <div className={grid}>
          <button type="button" className={`${PRIMARY} ${full}`} onClick={() => ask(`Show me the results of ${block.name}`)} data-action="results">
            <Trophy size={14} aria-hidden="true" />
            See results
          </button>
          <button type="button" className={`${GHOST} ${full}`} onClick={() => ask(`Run ${block.name} again next Friday with new questions`)} data-action="again">
            <Repeat size={14} aria-hidden="true" />
            Run it again next Friday
          </button>
          <LaterButton icon={<Archive size={14} aria-hidden="true" />} why={LATER.keep} className="col-span-2 border-transparent bg-transparent text-sky-300 sm:ml-auto">
            Keep the results
          </LaterButton>
        </div>
      );
  }
};

// ---------------------------------------------------------------------------------------------------
// Results card (design.md §6)

const ResultsCard: React.FC<{ block: PocketBlock; results: PocketResults; ask: (t: string) => void }> = ({ block, results, ask }) => {
  const [win, ...rest] = results.teams;
  const winPalette = teamPalette(win, 0);
  const startAt = Math.max(0, TEAM_PALETTES.indexOf(winPalette as (typeof TEAM_PALETTES)[number]));
  const faces = Math.max(1, Math.min(4, win.members.length || win.players || 1));
  return (
    <section className="pocket-card relative flex max-w-[680px] flex-col gap-4 overflow-hidden rounded-2xl border border-sky-800 bg-surface-raised p-4 shadow-card sm:p-[18px]" data-pocket="results" aria-label={`${block.name} results`}>
      <div className="pointer-events-none absolute inset-x-0 top-0 h-28 bg-gradient-to-b from-sky-500/[0.07] to-transparent" aria-hidden="true" />
      <header className="relative flex items-start gap-3">
        <Tile kind="trophy" />
        <div className="min-w-0 flex-1">
          <div className="text-2xs font-semibold text-sky-300">Pocket app · {kitLabel(block.kit)} results</div>
          <h3 className="text-[20px] font-bold leading-[26px] text-zinc-50 sm:text-[19px]">{block.name}</h3>
          {block.when && <p className="text-[13px] text-zinc-300">{block.when}</p>}
        </div>
      </header>
      <div className="relative flex flex-wrap items-center gap-3 rounded-2xl border border-sky-700 bg-sky-500/[0.08] px-3.5 py-3" data-winner>
        <span className="flex shrink-0 -space-x-0.5 [&>svg]:h-[22px] [&>svg]:w-[22px] sm:[&>svg]:h-[30px] sm:[&>svg]:w-[30px]" aria-hidden="true">
          {Array.from({ length: faces }, (_, i) => (
            <TeamCreature key={i} palette={TEAM_PALETTES[(startAt + i) % TEAM_PALETTES.length]} size={30} />
          ))}
        </span>
        <div className="min-w-[10rem] flex-1">
          <p className="text-[17px] font-bold text-zinc-50">{win.name} win</p>
          <p className="text-[13px] text-zinc-400 sm:text-meta">
            {win.members.length > 0 ? `${win.members.join(', ')} · ` : ''}
            {num(win.points)} points{results.margin !== null ? ` · ${num(results.margin)} ahead` : ''}
          </p>
        </div>
        {/* Phone: the sticker takes its own line, so the team name keeps the width. */}
        <span className="basis-full sm:basis-auto">
          <Sticker tone="holo" icon={<Trophy size={12} aria-hidden="true" />} ariaLabel={`Winners: ${win.name}`}>
            Winners
          </Sticker>
        </span>
      </div>
      <div className="relative grid grid-cols-1 gap-5 sm:grid-cols-[minmax(0,1.15fr)_minmax(0,1fr)]">
        <div className="min-w-0">
          <h4 className="mb-1.5 text-[13px] font-semibold text-zinc-200 sm:text-meta">Scores</h4>
          <table className="w-full text-[15px] sm:text-[13px]">
            <thead>
              <tr className="border-b border-hairline text-left text-meta text-zinc-400">
                <th className="w-7 py-1.5 font-medium">
                  <span className="sr-only">Place</span>
                </th>
                <th className="py-1.5 font-medium">Team</th>
                <th className="py-1.5 text-right font-medium">Players</th>
                <th className="py-1.5 pl-3 text-right font-medium">Points</th>
              </tr>
            </thead>
            <tbody>
              {[win, ...rest].map((t, i) => (
                <tr key={`${t.name}-${i}`} className={`border-b border-hairline ${i === 0 ? 'bg-sky-500/[0.10] font-semibold text-zinc-50' : 'text-zinc-200'}`}>
                  <td className="py-2 pl-2 tabular-nums">{i + 1}</td>
                  <td className="py-2">
                    <span className="flex items-center gap-2">
                      {/* The team's colour: its creature's body colour, from the avatar palettes. */}
                      <span className="inline-block h-3.5 w-1.5 shrink-0 rounded-sm" style={{ backgroundColor: paletteCreature(teamPalette(t, i)).body }} aria-hidden="true" />
                      <span className="min-w-0 truncate">{t.name}</span>
                    </span>
                  </td>
                  <td className="py-2 text-right tabular-nums">{t.players}</td>
                  <td className="py-2 pl-3 text-right tabular-nums">{num(t.points)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <div className="flex min-w-0 flex-col gap-3">
          {results.missed.length > 0 && (
            <div>
              <h4 className="mb-1.5 text-[13px] font-semibold text-zinc-200 sm:text-meta">Missed most</h4>
              <ul className="flex flex-col gap-3">
                {results.missed.map((m) => (
                  <li key={m.label} className="flex flex-col gap-1.5">
                    <div className="flex items-start gap-3 text-[13px] text-zinc-200 sm:text-meta">
                      <span className="min-w-0 flex-1">
                        {m.label} · {m.question}
                        {m.answer ? ` (${m.answer})` : ''}
                      </span>
                      <span className="shrink-0 text-zinc-400 tabular-nums">{Math.round(m.pct_right)}% right</span>
                    </div>
                    <Bar value={m.pct_right / 100} label={`${m.label}: ${Math.round(m.pct_right)}% right`} tone="warn" />
                  </li>
                ))}
              </ul>
            </div>
          )}
          <ul className="flex flex-col gap-1.5">
            {results.near_misses !== null && (
              <Row icon={<CheckCircle2 size={15} />}>
                {results.near_misses} near {results.near_misses === 1 ? 'miss' : 'misses'} settled
                {results.near_misses_accepted !== null ? `, you accepted ${results.near_misses_accepted}` : ''}
              </Row>
            )}
            <Row icon={<Zap size={15} />}>
              {block.budget ? `${num(block.budget.calls)} bot ${block.budget.calls === 1 ? 'call' : 'calls'} in all, about ${usd(block.budget.usd)}` : "Bot calls weren't counted for this one"}
            </Row>
          </ul>
        </div>
      </div>
      <div className="relative grid grid-cols-2 gap-2 sm:flex sm:flex-wrap sm:items-center">
        <button type="button" className={`${PRIMARY} col-span-2 sm:col-span-1`} onClick={() => ask(`Run ${block.name} again next Friday with new questions`)} data-action="again">
          <Repeat size={14} aria-hidden="true" />
          Run it again next Friday
        </button>
        <button type="button" className={`${GHOST} col-span-2 sm:col-span-1`} onClick={() => ask(`Reuse ${block.name} with new questions`)} data-action="reuse">
          <Pencil size={14} aria-hidden="true" />
          Reuse with new questions
        </button>
        <LaterButton icon={<Archive size={14} aria-hidden="true" />} why={LATER.keep} className="col-span-2 border-transparent bg-transparent text-sky-300 sm:ml-auto">
          Keep the results
        </LaterButton>
      </div>
      <footer className="relative -mx-4 -mb-4 flex items-start gap-2 border-t border-hairline px-4 py-3 text-meta text-zinc-400 sm:-mx-[18px] sm:-mb-[18px] sm:px-[18px]">
        <Lock size={13} className="mt-0.5 shrink-0" aria-hidden="true" />
        <span>
          {deletionLine(block)} Keeping the results saves names and scores only.
        </span>
      </footer>
    </section>
  );
};

// ---------------------------------------------------------------------------------------------------
// The card

/** The Pocket renderer: Secret Santa has its own card; every other kit uses the quiz-shaped one. */
export const Pocket: React.FC<RendererProps<PocketBlock>> = (props) => (props.block.santa ? <SantaCard block={props.block} ask={props.ask} /> : <QuizPocket {...props} />);

const QuizPocket: React.FC<RendererProps<PocketBlock>> = ({ block, ask }) => {
  const [mode, setMode] = useState<PocketJoinMode>(block.join?.mode ?? 'wifi');
  const [confirm, setConfirm] = useState<Confirm>(null);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<Done>(null);
  const opener = useRef<HTMLDivElement | null>(null);

  if (block.state === 'ended' && block.results) return <ResultsCard block={block} results={block.results} ask={ask} />;

  const url = block.join && showsQr(block) ? urlFor(block.join, mode) : null;
  const act = async (kind: 'stop' | 'end') => {
    setBusy(true);
    try {
      await pocketApi.end(block.code);
      setDone({ kind: 'ok', text: kind === 'end' ? 'Ended. Quizmaster is posting the winners in this chat.' : `Stopped. Nobody can join ${block.name} now.` });
      setConfirm(null);
    } catch (e) {
      setDone({ kind: 'error', text: `Couldn't ${kind === 'end' ? 'end the quiz' : 'stop it'}: ${e instanceof PocketApiError ? e.message : 'something went wrong.'} Try again.` });
    } finally {
      setBusy(false);
    }
  };
  const close = () => {
    setConfirm(null);
    opener.current?.querySelector<HTMLButtonElement>(`[data-action="${confirm?.kind}"]`)?.focus();
  };

  return (
    <section
      className="pocket-card relative flex max-w-[680px] flex-col gap-4 overflow-hidden rounded-2xl border border-sky-800 bg-surface-raised p-4 shadow-card sm:p-[18px]"
      data-pocket={block.state}
      data-paused={block.paused || undefined}
      aria-label={`${block.name}, pocket app`}
    >
      <div className="pointer-events-none absolute inset-x-0 top-0 h-28 bg-gradient-to-b from-sky-500/[0.07] to-transparent" aria-hidden="true" />
      <header className="relative flex items-start gap-3">
        <Tile kind="quiz" />
        <div className="min-w-0 flex-1">
          <div className="text-2xs font-semibold text-sky-300">Pocket app · {kitLabel(block.kit)}</div>
          <h3 className="text-[20px] font-bold leading-[26px] text-zinc-50 sm:text-[19px]">{block.name}</h3>
          {/* Phone: the state chip sits under the name, so a long name keeps the width. */}
          <span className="mt-1.5 block sm:hidden">
            <StateChip block={block} />
          </span>
          {(block.state === 'live' ? block.live?.now ?? block.when : block.when) && (
            <p className="text-[13px] text-zinc-300">{block.state === 'live' ? block.live?.now ?? block.when : block.when}</p>
          )}
        </div>
        <span className="hidden shrink-0 sm:block">
          <StateChip block={block} />
        </span>
      </header>
      <div className="relative flex flex-col gap-4">
        <Body block={block} mode={mode} setMode={setMode} closed={done?.kind === 'ok'} />
        {done ? (
          <p role="status" className={`flex items-start gap-2 text-[15px] sm:text-[13px] ${done.kind === 'ok' ? 'text-ok' : 'text-bad'}`} data-done={done.kind}>
            {done.kind === 'ok' ? <Check size={15} className={ICON} aria-hidden="true" /> : <AlertTriangle size={15} className={ICON} aria-hidden="true" />}
            {done.text}
          </p>
        ) : null}
        {confirm ? (
          <ConfirmBox block={block} kind={confirm.kind} busy={busy} onYes={() => act(confirm.kind)} onNo={close} />
        ) : done?.kind === 'ok' ? null : (
          <div ref={opener}>
            <Actions block={block} url={url} ask={ask} onStop={() => setConfirm({ kind: 'stop' })} onEnd={() => setConfirm({ kind: 'end' })} />
          </div>
        )}
      </div>
      <footer className="relative -mx-4 -mb-4 flex items-start gap-2 border-t border-hairline px-4 py-3 text-meta text-zinc-400 sm:-mx-[18px] sm:-mb-[18px] sm:px-[18px]" data-budget>
        <Zap size={13} className="mt-0.5 shrink-0" aria-hidden="true" />
        <span>{budgetLine(block)}</span>
      </footer>
    </section>
  );
};
