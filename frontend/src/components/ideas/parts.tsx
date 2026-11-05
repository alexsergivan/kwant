import React, { useEffect, useId, useRef, useState } from 'react';
import { CircleHelp, CirclePause, Info, Loader2, MoreHorizontal, ThumbsUp, TriangleAlert } from 'lucide-react';
import { OwnerAvatar } from '../MissionBoardView';
import { EFFORT_PIPS, EFFORT_WORD, IDEA_MAX, type IdeaEffort, type IdeaVerdict, VERDICT_WORD } from '../../ideas';

export const focusRing = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500 focus-visible:outline-offset-2';
const btnBase = `inline-flex items-center justify-center gap-1.5 rounded-lg text-[13px] font-medium disabled:opacity-50 disabled:cursor-not-allowed aria-disabled:opacity-50 aria-disabled:cursor-not-allowed ${focusRing}`;
/** Phone targets are 44 px; desktop buttons 32 px (pointer). */
export const btn = {
  primary: `${btnBase} kw-btn-primary min-h-[44px] sm:min-h-[32px] px-3.5`,
  secondary: `${btnBase} min-h-[44px] sm:min-h-[32px] px-3 border border-white/[0.14] bg-white/[0.03] text-[#ecebfa] hover:bg-white/[0.07]`,
  quiet: `${btnBase} min-h-[44px] sm:min-h-[32px] px-2.5 text-[#bcb7de] hover:text-[#f7f6ff] hover:bg-white/[0.06]`,
};

/** A busy icon: spins, except under reduced motion (spec §7). */
export const Spinner: React.FC<{ size?: number }> = ({ size = 14 }) => <Loader2 size={size} aria-hidden="true" className="animate-spin motion-reduce:animate-none" />;

const VERDICT_STYLE: Record<IdeaVerdict, { cls: string; Icon: typeof ThumbsUp }> = {
  worth_it: { cls: 'text-teal-300 border-teal-400/30 bg-teal-400/10', Icon: ThumbsUp },
  maybe: { cls: 'text-amber-300 border-amber-300/35 bg-amber-300/10', Icon: CircleHelp },
  not_now: { cls: 'text-zinc-300 border-zinc-700 bg-zinc-800', Icon: CirclePause },
};

/** Icon and word, never colour alone (1.4.1). */
export const VerdictPill: React.FC<{ verdict: IdeaVerdict; small?: boolean }> = ({ verdict, small = false }) => {
  const { cls, Icon } = VERDICT_STYLE[verdict];
  return (
    <span
      aria-label={`Verdict: ${VERDICT_WORD[verdict]}`}
      data-verdict={verdict}
      className={`inline-flex items-center gap-1.5 shrink-0 rounded-lg border font-semibold ${small ? 'h-6 px-2 text-[12px]' : 'h-7 px-2.5 text-[13px]'} ${cls}`}
    >
      <Icon size={small ? 13 : 14} aria-hidden="true" />
      <span aria-hidden="true">{VERDICT_WORD[verdict]}</span>
    </span>
  );
};

/** "Effort ▮▮▮ L · large"; the pips are decoration, "Effort: large" is what's read. */
export const Effort: React.FC<{ effort: IdeaEffort; label?: boolean; word?: boolean }> = ({ effort, label = true, word = true }) => (
  <span className="inline-flex items-center gap-1.5 text-[12px] text-[#a39dcc] shrink-0" data-effort={effort}>
    <span className="sr-only">Effort: {EFFORT_WORD[effort]}</span>
    {label && <span aria-hidden="true">Effort</span>}
    <span aria-hidden="true" className="inline-flex gap-[2px]">
      {[1, 2, 3].map((p) => (
        <span key={p} className="inline-block w-[6px] h-[10px] rounded-[1px]" style={{ background: p <= EFFORT_PIPS[effort] ? '#b8adff' : '#3a3366' }} />
      ))}
    </span>
    <span aria-hidden="true" className="font-semibold text-[#ecebfa]">
      {effort}
    </span>
    {word && <span aria-hidden="true">· {EFFORT_WORD[effort]}</span>}
  </span>
);

/** The bot's avatar, with the working ring while it peeks (static: no loop). */
export const BotFace: React.FC<{ kwantId: string | null; name: string; size?: number; peeking?: boolean }> = ({ kwantId, name, size = 20, peeking = false }) => (
  <span className="relative inline-flex shrink-0" data-peeking={peeking || undefined}>
    <OwnerAvatar kwantId={kwantId} name={name} size={size} />
    {peeking && <span aria-hidden="true" className="absolute -inset-[3px] rounded-[9px] pointer-events-none" style={{ boxShadow: '0 0 0 2px #9d8fff, 0 0 8px rgb(157 143 255 / .55)' }} />}
  </span>
);

/**
 * The idea as typed: plain text only (never Markdown or HTML). `@Name` of a bot in the space is tinted,
 * display only. Long ideas clamp to `lines` with "more".
 */
export const IdeaText: React.FC<{ text: string; botNames: string[]; lines?: number; className?: string; id?: string }> = ({ text, botNames, lines = 4, className = '', id }) => {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLParagraphElement | null>(null);
  const [clamped, setClamped] = useState(false);
  useEffect(() => {
    const el = ref.current;
    if (el && !open) setClamped(el.scrollHeight > el.clientHeight + 1);
  }, [text, open]);
  const parts = splitMentions(text, botNames);
  return (
    <div className="min-w-0">
      <p
        id={id}
        ref={ref}
        className={`whitespace-pre-wrap break-words ${className}`}
        style={open ? undefined : { display: '-webkit-box', WebkitLineClamp: lines, WebkitBoxOrient: 'vertical', overflow: 'hidden' }}
        data-idea-text
      >
        {parts.map((p, i) =>
          p.mention ? (
            <span key={i} className="text-[#b8adff] font-medium" data-mention>
              {p.text}
            </span>
          ) : (
            <React.Fragment key={i}>{p.text}</React.Fragment>
          ),
        )}
      </p>
      {(clamped || open) && (
        <button type="button" onClick={() => setOpen((v) => !v)} aria-expanded={open} className={`mt-0.5 text-[12px] text-[#b8adff] hover:underline rounded ${focusRing}`}>
          {open ? 'less' : 'more'}
        </button>
      )}
    </div>
  );
};

/** Splits on `@Name` for names of bots in the space (longest first), case-insensitive. */
export function splitMentions(text: string, names: string[]): { text: string; mention: boolean }[] {
  const ns = [...new Set(names.filter((n) => n && n.trim()))].sort((a, b) => b.length - a.length);
  if (!ns.length || !text.includes('@')) return [{ text, mention: false }];
  const esc = ns.map((n) => n.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
  const re = new RegExp(`@(?:${esc.join('|')})(?![\\p{L}\\p{N}_])`, 'giu');
  const out: { text: string; mention: boolean }[] = [];
  let last = 0;
  for (const m of text.matchAll(re)) {
    const at = m.index ?? 0;
    if (at > 0 && /[\p{L}\p{N}_]/u.test(text[at - 1])) continue;
    if (at > last) out.push({ text: text.slice(last, at), mention: false });
    out.push({ text: m[0], mention: true });
    last = at + m[0].length;
  }
  if (last < text.length) out.push({ text: text.slice(last), mention: false });
  return out.length ? out : [{ text, mention: false }];
}

/** Sky info (409s, cap reached), zinc (off), rose (errors). Status lines are polite; errors are alerts. */
export const InfoLine: React.FC<{ tone: 'sky' | 'zinc' | 'rose'; children: React.ReactNode; action?: React.ReactNode; className?: string }> = ({ tone, children, action, className = '' }) => {
  const cls =
    tone === 'sky'
      ? 'border-sky-300/30 bg-sky-300/10 text-sky-100'
      : tone === 'rose'
        ? 'border-rose-300/35 bg-rose-300/[0.12] text-rose-100'
        : 'border-white/[0.14] bg-white/[0.03] text-[#bcb7de]';
  const Icon = tone === 'rose' ? TriangleAlert : Info;
  return (
    <div role={tone === 'rose' ? 'alert' : 'status'} className={`flex items-start gap-2 px-3 py-2 rounded-lg border text-[13px] leading-5 ${cls} ${className}`} data-info={tone}>
      <Icon size={14} aria-hidden="true" className={`mt-[3px] shrink-0 ${tone === 'sky' ? 'text-sky-300' : tone === 'rose' ? 'text-rose-300' : 'text-zinc-400'}`} />
      <span className="min-w-0 flex-1 break-words">{children}</span>
      {action}
    </div>
  );
};

/** The slip/card menu: Edit and Delete (or what's passed). A plain disclosure (S3): aria-expanded on ⋯, Tab
 * between items, Esc closes and returns focus, focus leaving it closes it. Busy is aria-disabled, so a
 * focused ⋯ keeps focus (M3). */
export const IdeaMenu: React.FC<{ label: string; items: { key: string; label: string; onSelect: () => void; danger?: boolean }[]; disabled?: boolean; buttonRef?: React.Ref<HTMLButtonElement> }> = ({
  label,
  items,
  disabled = false,
  buttonRef,
}) => {
  const [open, setOpen] = useState(false);
  const btnRef = useRef<HTMLButtonElement | null>(null);
  const menuId = useId();
  const listRef = useRef<HTMLDivElement | null>(null);
  const wrapRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!open) return;
    listRef.current?.querySelector<HTMLButtonElement>('button')?.focus();
    const onDoc = (e: MouseEvent) => {
      if (!listRef.current?.contains(e.target as Node) && !btnRef.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', onDoc);
    return () => document.removeEventListener('mousedown', onDoc);
  }, [open]);
  const setRef = (el: HTMLButtonElement | null) => {
    btnRef.current = el;
    if (typeof buttonRef === 'function') buttonRef(el);
    else if (buttonRef) (buttonRef as React.MutableRefObject<HTMLButtonElement | null>).current = el;
  };
  return (
    <div
      ref={wrapRef}
      className="relative shrink-0"
      onBlur={(e) => {
        if (open && !wrapRef.current?.contains(e.relatedTarget as Node | null) && e.relatedTarget) setOpen(false);
      }}
    >
      <button
        ref={setRef}
        type="button"
        aria-disabled={disabled || undefined}
        aria-label={label}
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        onClick={() => {
          if (!disabled) setOpen((v) => !v);
        }}
        className={`grid place-items-center w-11 h-11 sm:w-8 sm:h-8 rounded-lg text-[#a39dcc] hover:text-[#f7f6ff] hover:bg-white/[0.06] aria-disabled:opacity-50 aria-disabled:cursor-not-allowed ${focusRing}`}
        data-idea-menu
      >
        <MoreHorizontal size={16} aria-hidden="true" />
      </button>
      {open && (
        <div
          ref={listRef}
          id={menuId}
          role="group"
          aria-label={label}
          onKeyDown={(e) => {
            if (e.key === 'Escape') {
              e.stopPropagation();
              setOpen(false);
              btnRef.current?.focus();
            }
          }}
          className="absolute right-0 top-full mt-1 z-20 min-w-[140px] p-1 rounded-xl border border-white/[0.14] bg-[#231d4b] shadow-2xl shadow-black/50"
        >
          {items.map((it) => (
            <button
              key={it.key}
              type="button"
              onClick={() => {
                setOpen(false);
                it.onSelect();
              }}
              className={`w-full text-left px-3 min-h-[44px] sm:min-h-[32px] rounded-lg text-[13px] ${it.danger ? 'text-rose-300 hover:bg-rose-400/10' : 'text-[#ecebfa] hover:bg-white/[0.07]'} ${focusRing}`}
              data-menu-item={it.key}
            >
              {it.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
};

/** Edit in place (states §9). Esc cancels; Save is off when empty or too long. */
export const EditIdea: React.FC<{ initial: string; hasTake: boolean; busy: boolean; error: string | null; onSave: (text: string) => void; onCancel: () => void }> = ({
  initial,
  hasTake,
  busy,
  error,
  onSave,
  onCancel,
}) => {
  const [text, setText] = useState(initial);
  const ref = useRef<HTMLTextAreaElement | null>(null);
  const id = useId();
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.focus();
    el.setSelectionRange(el.value.length, el.value.length);
  }, []);
  const len = [...text].length;
  const over = len > IDEA_MAX;
  const empty = !text.trim();
  return (
    <form
      className="flex flex-col gap-2"
      onSubmit={(e) => {
        e.preventDefault();
        if (!over && !empty && !busy) onSave(text.trim());
      }}
      data-idea-edit
    >
      <label htmlFor={id} className="sr-only">
        Edit your idea
      </label>
      <textarea
        id={id}
        ref={ref}
        value={text}
        rows={3}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Escape') {
            e.preventDefault();
            e.stopPropagation();
            onCancel();
          }
        }}
        className="w-full rounded-xl border border-[#7c6cf0] bg-[#0f0b22] px-3 py-2 text-[15px] leading-[22px] text-[#f7f6ff] focus:outline-none focus:ring-1 focus:ring-[#7c6cf0] resize-y"
      />
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <span className={`text-[12px] tabular-nums ${over ? 'text-rose-300' : 'text-[#8d87b8]'}`} data-counter>
          {len.toLocaleString('en-US')} / {IDEA_MAX.toLocaleString('en-US')}
          {over ? ' · too long for the jar' : ''}
        </span>
        <span className="text-[12px] text-[#a39dcc] flex-1 min-w-[180px]">{hasTake ? 'Saving clears this take, and a free bot looks again.' : 'Saving puts it back in the jar.'}</span>
        <button type="button" onClick={onCancel} className={btn.quiet}>
          Cancel
        </button>
        <button type="submit" disabled={over || empty} aria-disabled={busy || undefined} className={btn.primary} data-save>
          {busy && <Spinner />}
          Save and put back
        </button>
      </div>
      {error && <InfoLine tone="rose">{error}</InfoLine>}
    </form>
  );
};
