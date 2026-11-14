// Fix it "Send to…" (Mira, autonomy/fix-it-send-to/design.md §1–§3, §6, §8; Juno's #609 API). `GoesTo` is the
// quiet line under an open finding (and in the line-notes bar) that says who Fix it goes to and opens the
// picker. `RecipientPicker` is "Who fixes this PR?": a popover on desktop, a bottom sheet on phone. Picking a
// row only sets the card's pick; it never sends. Text from the server is rendered as text.

import React, { useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { AlertTriangle, Check, ChevronDown, Search, X } from 'lucide-react';
import { ModernAvatar } from '../ModernAvatar';
import type { DevRecipients, RecipientBot } from '../../dev/types';
import {
  LIST_FAILED,
  PICKER_CAPTION,
  PICKER_TITLE,
  SEARCH_FROM,
  filterBots,
  goesLabel,
  keepPick,
  loadPick,
  noMatch,
  nobodyFooter,
  pickedSay,
  pickerRows,
  rowMeta,
  savePick,
  target,
  whyLabel,
  loadShown,
  saveShown,
} from '../../dev/recipients';

const RING = 'outline-none focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-sky-400';

/** The card's list: loading, failed, or loaded. */
export type RecipientsLoad = { s: 'loading' } | { s: 'failed' } | { s: 'ready'; r: DevRecipients };

export interface GoesToProps {
  load: RecipientsLoad;
  pick: RecipientBot | null;
  /** `sm`: the one shared line under "Matches the ticket". */
  size?: 'md' | 'sm';
  phone?: boolean;
  /** The picker is open from this trigger. */
  expanded?: boolean;
  onOpen: (el: HTMLElement) => void;
  onRetry: () => void;
}

const Skel: React.FC<{ w: number; h?: number; round?: boolean }> = ({ w, h = 10, round }) => (
  <span aria-hidden="true" className={`inline-block bg-white/[.08] ${round ? 'rounded' : 'rounded'}`} style={{ width: w, height: h }} />
);

/** "Goes to {bot} · {why} ▾" (design §1), "No default bot · Pick who fixes it ▾" (§3), and the §6 states. */
export const GoesTo: React.FC<GoesToProps> = ({ load, pick, size = 'md', phone = false, expanded = false, onOpen, onRetry }) => {
  const box = phone
    ? 'order-first w-full min-h-[44px] justify-start'
    : size === 'sm'
      ? 'min-h-[28px]'
      : 'min-h-[28px]';
  const base = `inline-flex max-w-full items-center gap-1.5 rounded-lg px-2 text-meta ${box}`;
  if (load.s === 'loading') {
    return (
      <span data-goes-to="loading" className={`${base} text-zinc-400`}>
        Goes to <Skel w={16} h={16} /> <Skel w={56} />
        <span className="sr-only">: loading who fixes go to</span>
      </span>
    );
  }
  if (load.s === 'failed') {
    return (
      <span data-goes-to="failed" className={`${base} flex-wrap text-zinc-400`}>
        <span className="min-w-0 [overflow-wrap:anywhere]">Goes to the PR’s usual bot · {LIST_FAILED}</span>
        <button type="button" onClick={onRetry} className={`kw-btn-ghost min-h-[44px] px-2 py-0.5 text-meta sm:min-h-[28px] ${RING}`}>
          Try again
        </button>
      </span>
    );
  }
  const r = load.r;
  if (r.bots.length === 0) {
    return (
      <span data-goes-to="nobody" className={`${base} text-zinc-400 [overflow-wrap:anywhere]`}>
        {r.nobody ?? 'There’s no bot in this space to send it to.'}
      </span>
    );
  }
  const to = target(r, pick);
  const why = whyLabel(r.default, pick);
  const chosen = why === 'your pick';
  const common = {
    type: 'button' as const,
    'aria-haspopup': 'listbox' as const,
    'aria-expanded': expanded,
    onClick: (e: React.MouseEvent<HTMLButtonElement>) => onOpen(e.currentTarget),
    className: `${base} text-left hover:bg-white/[.06] ${RING}`,
  };
  if (!to) {
    return (
      <button {...common} data-goes-to="none" aria-label="No default bot. Pick who fixes it">
        <AlertTriangle size={14} aria-hidden="true" className="shrink-0 text-[#fcd34d]" />
        <span className="text-zinc-300">No default bot</span>
        <span aria-hidden="true" className="text-zinc-500">·</span>
        <span className="font-semibold text-violet-300">Pick who fixes it</span>
        <ChevronDown size={14} aria-hidden="true" className="shrink-0 text-zinc-400" />
      </button>
    );
  }
  return (
    <button {...common} data-goes-to={chosen ? 'pick' : r.default?.via ?? 'pick'} data-goes-to-id={to.id} aria-label={goesLabel(to.name, why)}>
      <span className="shrink-0 text-zinc-400">Goes to</span>
      <span aria-hidden="true" className="inline-flex shrink-0">
        <ModernAvatar name={to.name} size={16} />
      </span>
      <span className="min-w-0 truncate font-semibold text-zinc-100">{to.name}</span>
      {why && <span className={`min-w-0 truncate ${chosen ? 'text-[#c4b5fd]' : 'text-zinc-400'}`}>· {why}</span>}
      <ChevronDown size={14} aria-hidden="true" className="shrink-0 text-zinc-400" />
    </button>
  );
};

/** The line-notes bar's trigger (design §7): avatar · Ana ▾, or "Pick a bot ▾". */
export const BarTrigger: React.FC<{ load: RecipientsLoad; pick: RecipientBot | null; fallback: string | null; phone: boolean; expanded: boolean; onOpen: (el: HTMLElement) => void }> = ({
  load,
  pick,
  fallback,
  phone,
  expanded,
  onOpen,
}) => {
  const r = load.s === 'ready' ? load.r : null;
  const name = target(r, pick)?.name ?? (r ? null : fallback);
  if (!r || r.bots.length === 0) {
    // No list to pick from: the name stays plain text, as before #609.
    return name ? <span className="font-semibold text-zinc-100">{name}</span> : null;
  }
  return (
    <button
      type="button"
      data-ln-to={name ? 'bot' : 'none'}
      aria-haspopup="listbox"
      aria-expanded={expanded}
      aria-label={name ? `Notes go to ${name}. Change` : 'Pick a bot for the notes'}
      onClick={(e) => onOpen(e.currentTarget)}
      className={`inline-flex items-center gap-1 rounded-lg px-1 align-middle hover:bg-white/[.06] ${phone ? 'min-h-[44px]' : 'min-h-[28px]'} ${RING}`}
    >
      {name ? (
        <>
          <span aria-hidden="true" className="inline-flex shrink-0">
            <ModernAvatar name={name} size={phone ? 22 : 18} />
          </span>
          <span className="font-semibold text-zinc-100">{name}</span>
        </>
      ) : (
        <span className="font-semibold text-violet-300">Pick a bot</span>
      )}
      <ChevronDown size={14} aria-hidden="true" className="shrink-0 text-zinc-400" />
    </button>
  );
};

// ---------------------------------------------------------------------------------------------------

export interface RecipientPickerProps {
  r: DevRecipients;
  pick: RecipientBot | null;
  anchor: HTMLElement | null;
  phone: boolean;
  /** `owner/name` for the no-default footer. */
  repo: string | null;
  onPick: (bot: RecipientBot) => void;
  onClose: () => void;
}

const POP_W = 320;

/** "Who fixes this PR?" (design §2): a listbox, keyboard and pointer. */
export const RecipientPicker: React.FC<RecipientPickerProps> = ({ r, pick, anchor, phone, repo, onPick, onClose }) => {
  const id = useId();
  const listId = `${id}-list`;
  const titleId = `${id}-title`;
  const rows = useMemo(() => pickerRows(r), [r]);
  const [q, setQ] = useState('');
  const shown = useMemo(() => filterBots(rows, q), [rows, q]);
  const current = target(r, pick)?.id ?? null;
  const [active, setActive] = useState(() => Math.max(0, rows.findIndex((b) => b.id === current)));
  const search = rows.length > SEARCH_FROM;
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top?: number; bottom?: number } | null>(null);

  // Typing keeps the active row on the first match.
  useEffect(() => setActive(0), [q]);
  useEffect(() => {
    (search ? inputRef.current : listRef.current)?.focus({ preventScroll: true });
  }, [search]);
  useEffect(() => {
    listRef.current?.querySelector<HTMLElement>(`[data-i="${active}"]`)?.scrollIntoView?.({ block: 'nearest' });
  }, [active]);

  // Desktop: under the trigger, flipped above it when there's no room below.
  useLayoutEffect(() => {
    if (phone || !anchor) return;
    const place = () => {
      const a = anchor.getBoundingClientRect();
      const vw = window.innerWidth;
      const vh = window.innerHeight;
      const left = Math.max(8, Math.min(a.left, vw - POP_W - 8));
      const h = panelRef.current?.offsetHeight ?? 420;
      if (a.bottom + 6 + h > vh - 8 && a.top - 6 - h > 8) setPos({ left, bottom: vh - a.top + 6 });
      else setPos({ left, top: a.bottom + 6 });
    };
    place();
    window.addEventListener('resize', place);
    window.addEventListener('scroll', place, true);
    return () => {
      window.removeEventListener('resize', place);
      window.removeEventListener('scroll', place, true);
    };
  }, [phone, anchor]);

  // A press outside the popover closes it (the sheet has its own backdrop).
  useEffect(() => {
    if (phone) return;
    const down = (e: PointerEvent) => {
      const t = e.target as Node;
      if (panelRef.current?.contains(t) || anchor?.contains(t)) return;
      onClose();
    };
    document.addEventListener('pointerdown', down);
    return () => document.removeEventListener('pointerdown', down);
  }, [phone, anchor, onClose]);

  const choose = (b: RecipientBot | undefined) => {
    if (b) onPick(b);
  };
  const onKey = (e: React.KeyboardEvent) => {
    const n = shown.length;
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      onClose();
    } else if (e.key === 'ArrowDown' && n) {
      e.preventDefault();
      setActive((i) => Math.min(n - 1, i + 1));
    } else if (e.key === 'ArrowUp' && n) {
      e.preventDefault();
      setActive((i) => Math.max(0, i - 1));
    } else if (e.key === 'Home' && n && e.currentTarget === listRef.current) {
      e.preventDefault();
      setActive(0);
    } else if (e.key === 'End' && n && e.currentTarget === listRef.current) {
      e.preventDefault();
      setActive(n - 1);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      choose(shown[active]);
    } else if (e.key === 'Tab' && !phone) {
      onClose();
    }
  };
  const optId = (i: number) => `${id}-o${i}`;
  const def = r.default;

  const body = (
    <>
      <div className={phone ? 'flex items-start gap-2 px-4 pt-1' : 'px-3 pt-3'}>
        <div className="min-w-0 flex-1">
          <h2 id={titleId} className="text-[15px] font-semibold leading-[22px] text-zinc-50">
            {PICKER_TITLE}
          </h2>
          <p className="text-meta text-zinc-400">{PICKER_CAPTION}</p>
        </div>
        {phone && (
          <button type="button" aria-label="Close" onClick={onClose} className={`-mr-2 -mt-1 inline-flex h-11 w-11 shrink-0 items-center justify-center rounded-xl text-zinc-300 hover:bg-white/[.06] ${RING}`}>
            <X size={18} aria-hidden="true" />
          </button>
        )}
      </div>
      {search && (
        <div className={phone ? 'px-4 pt-3' : 'px-2 pt-2.5'}>
          <label className="flex items-center gap-2 rounded-lg border border-hairline bg-[#120e26] px-2.5 focus-within:outline focus-within:outline-2 focus-within:outline-sky-400">
            <Search size={14} aria-hidden="true" className="shrink-0 text-zinc-400" />
            <span className="sr-only">Find a bot or team</span>
            <input
              ref={inputRef}
              value={q}
              onChange={(e) => setQ(e.target.value)}
              onKeyDown={onKey}
              placeholder="Find a bot or team"
              role="combobox"
              aria-expanded="true"
              aria-controls={listId}
              aria-activedescendant={shown.length ? optId(active) : undefined}
              aria-autocomplete="list"
              className={`min-w-0 flex-1 bg-transparent py-2 text-zinc-100 placeholder:text-zinc-500 outline-none ${phone ? 'text-[16px]' : 'text-[13px]'}`}
            />
          </label>
        </div>
      )}
      {shown.length === 0 ? (
        <p data-picker-empty className="px-4 py-4 text-meta text-zinc-400">
          {noMatch(q)}
        </p>
      ) : (
        <ul
          ref={listRef}
          id={listId}
          role="listbox"
          tabIndex={search ? -1 : 0}
          aria-labelledby={titleId}
          aria-activedescendant={search ? undefined : optId(active)}
          onKeyDown={search ? undefined : onKey}
          className={`mt-2 overflow-y-auto px-1.5 pb-1.5 outline-none ${phone ? 'max-h-[calc(82vh-140px)]' : 'max-h-[300px]'} focus-visible:outline focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-sky-400`}
        >
          {shown.map((b, i) => {
            const isDefault = !!def && b.id === def.id;
            const meta = rowMeta(b, def);
            const sel = b.id === current;
            return (
              <React.Fragment key={b.id}>
                <li
                  id={optId(i)}
                  data-i={i}
                  data-bot={b.id}
                  role="option"
                  aria-selected={sel}
                  onPointerMove={() => setActive(i)}
                  onClick={() => choose(b)}
                  className={`flex cursor-pointer items-center gap-2.5 rounded-lg px-2 ${phone ? 'min-h-[52px]' : 'min-h-[40px]'} py-1 ${i === active ? 'bg-white/[.07] outline outline-1 outline-sky-400/60' : ''}`}
                >
                  <span aria-hidden="true" className="inline-flex shrink-0">
                    <ModernAvatar name={b.name} size={24} />
                  </span>
                  {isDefault ? (
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-[14px] font-medium text-zinc-100">{b.name}</span>
                      <span className="block text-meta text-zinc-400 [overflow-wrap:anywhere]">{meta}</span>
                    </span>
                  ) : (
                    <>
                      <span className="min-w-0 flex-1 truncate text-[14px] font-medium text-zinc-100">{b.name}</span>
                      {meta && <span className="max-w-[45%] shrink-0 truncate text-meta text-zinc-400">{meta}</span>}
                    </>
                  )}
                  <span className="inline-flex w-4 shrink-0 justify-center">{sel && <Check size={15} aria-hidden="true" className="text-[#a78bfa]" />}</span>
                </li>
                {isDefault && !q.trim() && i === 0 && shown.length > 1 && <li role="presentation" aria-hidden="true" className="mx-2 my-1 border-t border-hairline" />}
              </React.Fragment>
            );
          })}
        </ul>
      )}
      {!def && (
        <p data-picker-footer className="border-t border-hairline px-3 py-2.5 text-meta text-zinc-400 [overflow-wrap:anywhere]">
          {nobodyFooter(repo)}
        </p>
      )}
    </>
  );

  const node = phone ? (
    <div className="fixed inset-0 z-[60]" data-recipient-picker="sheet">
      <div aria-hidden="true" className="absolute inset-0 bg-black/50" onClick={onClose} />
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        onKeyDown={(e) => e.key === 'Escape' && onKey(e)}
        className="kw-st-sheet absolute inset-x-0 bottom-0 flex max-h-[82vh] flex-col rounded-t-2xl border-t border-hairline-strong bg-surface-overlay pb-[env(safe-area-inset-bottom)] shadow-xl"
      >
        <span aria-hidden="true" className="mx-auto mb-1 mt-2 block h-1 w-9 rounded-full bg-white/20" />
        {body}
      </div>
    </div>
  ) : (
    <div
      ref={panelRef}
      role="dialog"
      aria-labelledby={titleId}
      data-recipient-picker="popover"
      style={pos ? { position: 'fixed', left: pos.left, top: pos.top, bottom: pos.bottom, width: POP_W } : { position: 'fixed', visibility: 'hidden', width: POP_W }}
      className="kw-st-pop z-[60] rounded-xl border border-hairline-strong bg-surface-overlay shadow-xl"
    >
      {body}
    </div>
  );
  return typeof document === 'undefined' ? node : createPortal(node, document.body);
};

// ---------------------------------------------------------------------------------------------------
// The card's "send to" state (design §1): the list (fetched once per card), the pick (sessionStorage), and the
// one picker, anchored to whichever trigger opened it. ReviewCard owns it; FindingRow, TicketMatches and the
// line-notes bar read it.

export interface SendTo {
  load: RecipientsLoad;
  pick: RecipientBot | null;
  /** N1: the picker has been shown on this card, or a finding went to a chosen bot: every send carries `to`. */
  pinned: boolean;
  phone: boolean;
  /** The trigger the picker is open from, if any. */
  anchor: HTMLElement | null;
  /** Open the picker from `el`; `after` runs with the pick (line notes' "Send N notes…"). */
  open: (el: HTMLElement, after?: (to: RecipientBot) => void) => void;
  retry: () => void;
  /** After a bad_recipient: refetch, and drop the pick. */
  reset: () => void;
  /** After a send_error: refetch, and keep the pick only if it's still listed. */
  refresh: () => void;
}

const PHONE = '(max-width: 639px)';

export function useSendTo(
  cardId: string,
  fetcher: ((cardId: string) => Promise<DevRecipients>) | undefined,
  repo: string | null,
  phone: boolean,
  /** The card already has a finding with `sent_to_id`. */
  sentToChosen = false,
): { st: SendTo | null; picker: React.ReactNode; say: string } {
  const [load, setLoad] = useState<RecipientsLoad>({ s: 'loading' });
  const [pick, setPickState] = useState<RecipientBot | null>(() => loadPick(cardId));
  const [shown, setShown] = useState<boolean>(() => loadShown(cardId));
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  const after = useRef<((to: RecipientBot) => void) | null>(null);
  const [say, setSay] = useState('');
  const seq = useRef(0);

  const setPick = (p: RecipientBot | null) => {
    setPickState(p);
    savePick(cardId, p);
  };

  const fetchList = (drop: boolean) => {
    if (!fetcher) return;
    const n = ++seq.current;
    if (!drop) setLoad((l) => (l.s === 'ready' ? l : { s: 'loading' }));
    fetcher(cardId).then(
      (r) => {
        if (n !== seq.current) return;
        setLoad({ s: 'ready', r });
        setPickState((p) => {
          const kept = drop ? null : keepPick(p, r);
          savePick(cardId, kept);
          return kept;
        });
      },
      () => {
        if (n === seq.current) setLoad({ s: 'failed' });
      },
    );
  };

  useEffect(() => {
    setPickState(loadPick(cardId));
    setShown(loadShown(cardId));
    fetchList(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cardId, fetcher]);

  const anchorRef = useRef<HTMLElement | null>(null);
  anchorRef.current = anchor;
  // Closing gives focus back to the trigger that opened the picker (design §2).
  const close = React.useCallback(() => {
    const a = anchorRef.current;
    setAnchor(null);
    after.current = null;
    if (a) window.setTimeout(() => a.isConnected && a.focus({ preventScroll: true }), 0);
  }, []);

  if (!fetcher) return { st: null, picker: null, say: '' };

  const r = load.s === 'ready' ? load.r : null;
  const st: SendTo = {
    load,
    pick,
    pinned: shown || sentToChosen,
    phone,
    anchor,
    open: (el, then) => {
      if (!r || r.bots.length === 0) return;
      after.current = then ?? null;
      setShown(true);
      saveShown(cardId);
      setAnchor(el);
    },
    retry: () => fetchList(false),
    reset: () => {
      setPick(null);
      fetchList(true);
    },
    refresh: () => fetchList(false),
  };

  const onPick = (b: RecipientBot) => {
    const isDefault = !!r?.default && r.default.id === b.id;
    setPick(isDefault ? null : b);
    setSay(pickedSay(b.name));
    const then = after.current;
    close();
    then?.(b);
  };

  const picker = r && anchor ? <RecipientPicker r={r} pick={pick} anchor={anchor} phone={phone} repo={repo} onPick={onPick} onClose={close} /> : null;
  return { st, picker, say };
}

export const SEND_TO_PHONE = PHONE;
