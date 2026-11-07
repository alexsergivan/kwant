// 0.7.7 T5 Needs card, T4 Undo toast and Undo confirm. The card is a UI overlay built from live state
// (Rigel's Q-T5 default), never a chat message: rows tick from events the app already gets, with no
// polling and no model call. Design: next-077/design/design.md §T5, §T4.
import React, { useEffect, useRef, useState } from 'react';
import { Check, CheckCircle2, Loader2, Undo2, X } from 'lucide-react';
import { TemplateGlyph } from './TeamPreviewSheet';
import { useEscapeLayer } from '../escape';
import { useMedia } from '../useMedia';
import { PHONE_QUERY } from '../layout';
import {
  STATUS_WORD,
  chipText,
  markNeedsDone,
  needView,
  needsSeenDone,
  needsSub,
  openCount,
  plural,
  type NeedState,
  type NeedsContext,
  type TeamTemplate,
  type TemplateNeed,
} from '../teams/teamTemplates';

const FOCUS = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400';
const DAY = 24 * 60 * 60 * 1000;

export const relTime = (from: number, now = Date.now()): string => {
  const s = Math.max(0, Math.round((now - from) / 1000));
  if (s < 60) return 'just now';
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h} h ago`;
  return `${Math.round(h / 24)} d ago`;
};

const Circle: React.FC<{ s: NeedState }> = ({ s }) => {
  const base = 'w-5 h-5 shrink-0 grid place-items-center rounded-full transition-opacity duration-[120ms] motion-reduce:transition-none';
  if (s === 'done')
    return (
      <span className={base} style={{ background: 'var(--kw-ok)' }} aria-hidden="true">
        <Check size={12} strokeWidth={3} className="text-[#120e26]" />
      </span>
    );
  if (s === 'checking')
    return (
      <span className={base} aria-hidden="true">
        <Loader2 size={14} className="text-sky-400 animate-spin motion-reduce:hidden" />
        <span className="hidden motion-reduce:inline text-sky-400">…</span>
      </span>
    );
  if (s === 'bad')
    return (
      <span className={`${base} border-[1.5px]`} style={{ borderColor: 'var(--kw-bad)', color: 'var(--kw-bad)' }} aria-hidden="true">
        <X size={11} strokeWidth={3} />
      </span>
    );
  return <span className={`${base} border-[1.5px]`} style={{ borderColor: s === 'req' ? 'var(--kw-warn)' : 'var(--t5)' }} aria-hidden="true" />;
};

const ACTION: Record<string, string> = {
  loose_ends_imap: 'Connect',
  connections: 'Connect',
  dev_mode_setup: 'Pick a repo',
  composer_prompt: 'Tell the team',
};

export interface TeamNeedsCardProps {
  template: TeamTemplate;
  handle: string;
  teamId: string;
  createdAt: number;
  ctx: NeedsContext;
  /** Opens the existing form for a need (`opens`), or fills the composer for `composer_prompt`. */
  onOpen: (need: TemplateNeed) => void;
  onAddAccount: () => void;
  /** Fills the composer; never sends. */
  onFill: (text: string) => void;
  /** "Undo team"; absent once Undo is no longer possible. */
  onUndo?: () => void;
  now?: number;
}

export const TeamNeedsCard: React.FC<TeamNeedsCardProps> = ({ template: t, handle, teamId, createdAt, ctx, onOpen, onAddAccount, onFill, onUndo, now }) => {
  const phone = useMedia(PHONE_QUERY);
  const open = openCount(t, ctx);
  // Collapsed only when the chat is opened again after everything required was done (not in the same session).
  const [collapsed, setCollapsed] = useState(() => open === 0 && needsSeenDone(teamId));
  useEffect(() => {
    if (open === 0) markNeedsDone(teamId);
  }, [open, teamId]);

  // A tick announces politely: "Connect email: done".
  const [live, setLive] = useState('');
  const prev = useRef<Record<string, NeedState>>({});
  const views = t.needs.map((n) => ({ n, v: needView(n, ctx) }));
  useEffect(() => {
    for (const { n, v } of views) {
      const was = prev.current[n.id];
      if (was && was !== 'done' && v.state === 'done') setLive(`${n.label}: done`);
      prev.current[n.id] = v.state;
    }
  });

  const [, tick] = useState(0);
  useEffect(() => {
    const id = window.setInterval(() => tick((x) => x + 1), 60_000);
    return () => window.clearInterval(id);
  }, []);
  const at = now ?? Date.now();
  const canUndo = !!onUndo && at - createdAt < DAY && !collapsed;

  if (collapsed)
    return (
      <div role="status" data-needs-card="collapsed" className="kw-card flex items-center gap-2.5 px-4 py-2.5 text-[13px] text-zinc-200">
        <CheckCircle2 size={16} style={{ color: 'var(--kw-ok)' }} aria-hidden="true" />
        <span className="flex-1 min-w-0">The {t.label} team has what it needs.</span>
        <button type="button" onClick={() => setCollapsed(false)} className={`${phone ? 'min-h-11' : 'min-h-7'} px-2 rounded-md text-[12.5px] font-medium text-sky-300 hover:bg-white/5 ${FOCUS}`}>
          Show
        </button>
      </div>
    );

  const firstReq = !ctx.canRun ? '__account' : views.find(({ n, v }) => n.required && v.state !== 'done' && v.state !== 'checking')?.n.id;
  const btn = (primary: boolean) =>
    `${phone ? 'h-11 w-full mt-2' : 'h-8'} shrink-0 px-3 rounded-lg text-[13px] font-medium disabled:opacity-50 ${primary ? 'bg-sky-600 hover:bg-sky-500 text-white' : 'border border-zinc-700 text-zinc-100 hover:bg-white/5'} ${FOCUS}`;

  const row = (key: string, s: NeedState, label: string, text: string, optional: boolean, action: React.ReactNode) => (
    <li key={key} data-need={key} data-s={s} className={`flex ${phone ? 'flex-wrap' : 'items-center'} gap-3 min-h-[52px] p-2 border-t border-white/5 first:border-t-0`}>
      <Circle s={s} />
      <div className={`${phone ? 'flex-1 min-w-0 basis-[calc(100%-44px)]' : 'flex-1 min-w-0'}`}>
        <p className={`text-[13.5px] leading-[18px] font-medium ${s === 'done' ? 'text-zinc-300' : 'text-zinc-50'}`}>
          {label}
          {optional && s !== 'done' && <span className="ml-2 inline-flex items-center h-[18px] px-1.5 rounded-[5px] text-[10.5px] font-medium text-zinc-300 bg-white/[.06] align-middle">Optional</span>}
          <span className="sr-only">, {STATUS_WORD[s]}</span>
        </p>
        <p className="text-xs leading-[17px]" style={{ color: s === 'done' ? 'var(--kw-ok)' : s === 'bad' ? 'var(--kw-bad)' : 'var(--t4)' }}>
          {text}
        </p>
      </div>
      {action && <div className={phone ? 'w-full pl-8' : ''}>{action}</div>}
    </li>
  );

  return (
    <section role="region" aria-label={`${t.label} team setup`} data-needs-card={teamId} className="kw-card p-0 overflow-hidden">
      <header className="flex items-start gap-3 px-4 pt-3.5 pb-2.5">
        <TemplateGlyph icon={t.icon} size={18} box="w-9 h-9 rounded-[10px]" />
        <div className="min-w-0">
          <h3 className="text-sm leading-5 font-semibold text-zinc-50">Your {t.label} team is set up</h3>
          <p className="text-[12.5px] leading-[18px] text-zinc-400">{needsSub(t, ctx)}</p>
        </div>
      </header>
      <ul className="px-2">
        {!ctx.canRun &&
          row(
            '__account',
            'req',
            'Add an account',
            'Your bots can answer once one works.',
            false,
            <button type="button" onClick={onAddAccount} className={btn(true)}>
              Add an account
            </button>,
          )}
        {views.map(({ n, v }) =>
          row(
            n.id,
            v.state,
            n.label,
            v.text,
            !n.required,
            v.state === 'done' ? null : (
              <button type="button" disabled={v.state === 'checking'} onClick={() => onOpen(n)} className={btn(firstReq === n.id)} aria-label={v.state === 'bad' ? `Fix ${n.label}` : undefined}>
                {v.state === 'bad' ? 'Fix' : ACTION[n.opens] ?? 'Open'}
              </button>
            ),
          ),
        )}
      </ul>
      {t.first_prompts.length > 0 && (
        <div className="px-4 pt-2.5 pb-1 border-t border-white/5">
          <p className="text-xs text-zinc-400 mb-1.5">Try asking</p>
          <div className="flex flex-wrap gap-1.5">
            {t.first_prompts.map((p) => (
              <button
                key={p}
                type="button"
                onClick={() => onFill(p)}
                className={`inline-flex items-center ${phone ? 'h-11' : 'h-[30px]'} px-3 rounded-full border border-white/10 bg-white/[.03] text-[12.5px] text-zinc-200 hover:bg-white/[.06] ${FOCUS}`}
              >
                {chipText(p)}
              </button>
            ))}
          </div>
        </div>
      )}
      <footer className="flex items-center gap-2 px-4 pt-2.5 pb-3">
        <span className="flex-1 text-xs text-zinc-400">Created {relTime(createdAt, at)}</span>
        {canUndo && (
          <button type="button" onClick={onUndo} className={`inline-flex items-center gap-1.5 ${phone ? 'min-h-11' : 'min-h-7'} px-2 rounded-md text-[12.5px] font-medium text-sky-300 hover:bg-white/5 ${FOCUS}`} aria-label={`Undo team @${handle}`}>
            <Undo2 size={13} aria-hidden="true" /> Undo team
          </button>
        )}
      </footer>
      <p className="sr-only" aria-live="polite">
        {live}
      </p>
    </section>
  );
};

/** The system line above the card. */
export const TeamCreatedLine: React.FC<{ handle: string; label: string; createdAt: number }> = ({ handle, label, createdAt }) => (
  <p className="text-center text-xs text-zinc-400 my-2">
    You created @{handle} from the {label} template · {relTime(createdAt)}
  </p>
);

// ---------------------------------------------------------------------------------------------------
// T4 toast

export const TeamCreatedToast: React.FC<{ text: string; onUndo?: () => void; onDismiss: () => void; ms?: number }> = ({ text, onUndo, onDismiss, ms = 10_000 }) => {
  const phone = useMedia(PHONE_QUERY);
  const ref = useRef<HTMLDivElement>(null);
  const btn = useRef<HTMLButtonElement>(null);
  const hold = useRef(false);
  useEffect(() => {
    btn.current?.focus({ preventScroll: true });
  }, []);
  useEffect(() => {
    // It stays 10 s, and while hovered or focused.
    let left = ms;
    let last = Date.now();
    const id = window.setInterval(() => {
      const n = Date.now();
      if (!hold.current) left -= n - last;
      last = n;
      if (left <= 0) onDismiss();
    }, 250);
    return () => window.clearInterval(id);
  }, [ms, onDismiss]);
  return (
    <div
      ref={ref}
      data-team-toast
      onMouseEnter={() => (hold.current = true)}
      onMouseLeave={() => (hold.current = !!ref.current?.contains(document.activeElement))}
      onFocus={() => (hold.current = true)}
      onBlur={(e) => (hold.current = !!ref.current?.contains(e.relatedTarget as Node))}
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          e.preventDefault();
          e.stopPropagation();
          onDismiss();
        }
      }}
      className={`kw-toast fixed z-[65] ${phone ? 'left-3 right-3 bottom-[140px]' : 'left-6 bottom-[88px] max-w-[420px]'} flex items-center gap-2.5 px-3 py-2.5 rounded-xl border border-zinc-800 bg-zinc-900 shadow-xl text-[13px] text-zinc-100`}
    >
      <CheckCircle2 size={16} style={{ color: 'var(--kw-ok)' }} className="shrink-0" aria-hidden="true" />
      <span role="status" className="flex-1 min-w-0">
        {text}
      </span>
      {onUndo && (
        <button
          ref={btn}
          type="button"
          onClick={onUndo}
          className={`inline-flex items-center gap-1 ${phone ? 'h-11' : 'h-[30px]'} px-2.5 rounded-md text-[12.5px] font-medium border border-sky-500/40 bg-sky-500/10 text-sky-200 hover:bg-sky-500/20 ${FOCUS}`}
        >
          <Undo2 size={13} aria-hidden="true" /> Undo
        </button>
      )}
    </div>
  );
};

// ---------------------------------------------------------------------------------------------------
// T4 confirm (only when the chat has messages)

export const UndoTeamConfirm: React.FC<{
  label: string;
  handle: string;
  botNames: string[];
  skills: number;
  routine: boolean;
  messages: number;
  /** The team's name: the server renames its chat to #archived-team-<name> (a number added when taken). */
  teamName: string;
  /** "Drafter is also in @support; it will leave that team." */
  alsoIn?: { bot: string; team: string }[];
  /** Starter skills the person edited; Undo removes them too (Q-T2). */
  editedSkills?: string[];
  busy?: boolean;
  error?: string | null;
  onKeep: () => void;
  onUndo: () => void;
}> = ({ label, handle, botNames, skills, routine, messages, teamName, alsoIn = [], editedSkills = [], busy, error, onKeep, onUndo }) => {
  const phone = useMedia(PHONE_QUERY);
  const ref = useRef<HTMLDivElement>(null);
  const keep = useRef<HTMLButtonElement>(null);
  useEscapeLayer(ref, true, () => !busy && onKeep(), { modal: true });
  useEffect(() => keep.current?.focus(), []);
  const b = phone ? 'h-11 flex-1' : 'h-9';
  return (
    <div className="fixed inset-0 z-[75] grid grid-cols-[minmax(0,1fr)] place-items-center p-4 bg-[rgb(6_3_20/.62)]" onClick={() => !busy && onKeep()}>
      <div
        ref={ref}
        tabIndex={-1}
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="undo-team-h"
        aria-describedby="undo-team-b"
        onClick={(e) => e.stopPropagation()}
        data-undo-confirm
        className="w-full max-w-[420px] p-5 rounded-2xl bg-zinc-900 border border-zinc-800 shadow-2xl outline-none animate-[t-rise_180ms_ease-out] motion-reduce:animate-none"
      >
        <h3 id="undo-team-h" className="text-[17px] leading-6 font-semibold text-zinc-50">
          Undo the {label} team?
        </h3>
        <div id="undo-team-b" className="mt-2 text-[13px] leading-[19px] text-zinc-300">
          <p>This removes everything the template made:</p>
          <ul className="mt-1.5 ml-4 list-disc space-y-0.5">
            <li>
              {plural(botNames.length, 'bot')}: {botNames.join(', ')}
            </li>
            <li>{plural(skills, 'skill')}</li>
            {routine && <li>1 routine (off)</li>}
            <li>the @{handle} handle</li>
          </ul>
          {alsoIn.map((a) => (
            <p key={a.bot + a.team} className="mt-2">
              {a.bot} is also in @{a.team}; it will leave that team.
            </p>
          ))}
          {editedSkills.length > 0 && (
            <p className="mt-2">
              You edited {editedSkills.join(', ')}. {editedSkills.length === 1 ? 'It’s' : 'They’re'} removed too.
            </p>
          )}
          {messages > 0 && (
            <p className="mt-2">
              The chat has {plural(messages, 'message')}, so it’s kept, not deleted: it moves under Channels as #archived-team-{teamName}.
            </p>
          )}
          {error && (
            <p role="alert" className="mt-2 text-rose-300">
              {error}
            </p>
          )}
        </div>
        <div className="mt-4 flex justify-end gap-2">
          <button ref={keep} type="button" onClick={onKeep} disabled={busy} className={`${b} px-3.5 rounded-lg border border-zinc-700 text-sm text-zinc-100 hover:bg-white/5 ${FOCUS}`}>
            Keep the team
          </button>
          <button type="button" onClick={onUndo} disabled={busy} className={`${b} inline-flex items-center justify-center gap-2 px-3.5 rounded-lg bg-[#be123c] hover:bg-rose-600 text-white text-sm font-medium disabled:opacity-60 ${FOCUS}`}>
            {busy && <Loader2 size={14} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
            Undo team
          </button>
        </div>
      </div>
    </div>
  );
};
