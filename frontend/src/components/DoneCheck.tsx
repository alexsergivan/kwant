// 0.9.8 done-check (autonomy/done-check/design.md): one box between the chips and the answer that says whether
// anything checked a done item, with each check on demand; and the one-word marker on done rows in the list.
// Server text (line, what, how, evidence) is drawn as text only, never as links or HTML.

import React, { useId, useState } from 'react';
import { AlertTriangle, Check, ChevronDown, CircleDashed, Clock, Shield, ShieldCheck, ShieldHalf, ShieldX, X, type LucideIcon } from 'lucide-react';
import {
  ADD_A_CHECK,
  type Checked,
  type CheckState,
  EMPTY_HELP,
  ITEM_COLOUR,
  ITEM_WORD,
  type ItemKind,
  MARKER,
  SECTION_LABEL,
  STATE_STYLE,
  droppedLine,
  howText,
  itemKind,
  splitLine,
  toggleLabel,
} from '../doneCheck';
import { Sticker } from './Sticker';

const linkBtn = 'kw-focus kw-hit inline-flex items-center gap-0.5 rounded text-meta font-semibold text-violet-300 hover:text-violet-200';

const STATE_ICON: Record<CheckState, LucideIcon> = {
  checked: ShieldCheck,
  partly: ShieldHalf,
  failed: ShieldX,
  not_checked: Shield,
};

const ITEM_ICON: Record<ItemKind, { Icon: LucideIcon; cls: string }> = {
  passed: { Icon: Check, cls: 'text-ok' },
  failed: { Icon: X, cls: 'text-bad' },
  waiting: { Icon: Clock, cls: 'text-zinc-300' },
  not_checked: { Icon: CircleDashed, cls: 'text-zinc-400' },
  claim: { Icon: AlertTriangle, cls: 'text-warn' },
};

export const DoneCheck: React.FC<{
  checked: Checked & { state: CheckState };
  /** "Add a check" on an empty box: only for a routine whose editor is wired. */
  onAddCheck?: () => void;
}> = ({ checked, onAddCheck }) => {
  const listId = `${useId()}-checks`;
  const state = checked.state;
  const style = STATE_STYLE[state];
  const [open, setOpen] = useState(style.open);
  const { prefix, rest } = splitLine(state, checked.line);
  const Icon = STATE_ICON[state];
  const items = checked.items;
  const n = items.length + checked.items_dropped;
  return (
    <section aria-label={SECTION_LABEL} data-done-check={state} className={`rounded-2xl border px-4 py-3 flex flex-col gap-2 min-w-0 ${style.box}`}>
      <p className="grid grid-cols-[16px_minmax(0,1fr)] gap-x-2.5 text-body text-zinc-100" data-check-line>
        <Icon size={16} className={`mt-[3px] ${style.icon}`} aria-hidden="true" />
        <span className="min-w-0 break-words [overflow-wrap:anywhere]">
          {prefix ? (
            <>
              <b className={`font-semibold ${style.prefix}`} data-check-prefix>
                {prefix}
              </b>{' '}
              {rest}
            </>
          ) : (
            rest
          )}
        </span>
      </p>
      {items.length > 0 ? (
        <>
          <div className="ml-[26px]">
            <button type="button" aria-expanded={open} aria-controls={listId} onClick={() => setOpen((o) => !o)} className={linkBtn} data-compact data-check-toggle>
              {toggleLabel(open, n)}
              <ChevronDown size={14} aria-hidden="true" className={`transition-transform motion-reduce:transition-none ${open ? 'rotate-180' : ''}`} />
            </button>
          </div>
          <ul id={listId} hidden={!open} className={`ml-[26px] border-t border-hairline ${open ? 'kw-fade-in' : ''}`} data-check-list>
            {items.map((it, i) => {
              const kind = itemKind(it);
              const { Icon: ItemIcon, cls } = ITEM_ICON[kind];
              return (
                <li key={i} className="grid grid-cols-[14px_minmax(0,1fr)] gap-x-2.5 py-2 border-b border-hairline last:border-b-0" data-check-item={kind}>
                  <ItemIcon size={14} className={`mt-1 ${cls}`} aria-hidden="true" />
                  <div className="min-w-0">
                    <p className="text-body text-zinc-100 break-words [overflow-wrap:anywhere]" data-check-what>
                      {it.what}
                    </p>
                    <p className="text-meta break-words [overflow-wrap:anywhere]">
                      <span className={`font-semibold ${ITEM_COLOUR[kind]}`} data-check-result>
                        {ITEM_WORD[kind]}
                      </span>
                      {howText(it.how) && <span className="text-zinc-400"> · {howText(it.how)}</span>}
                    </p>
                    {typeof it.evidence === 'string' && it.evidence.trim() !== '' && (
                      <p className="mt-1 border-l-2 border-hairline-strong pl-2 text-meta text-zinc-300 whitespace-pre-wrap break-words [overflow-wrap:anywhere]" data-check-evidence>
                        {it.evidence}
                      </p>
                    )}
                  </div>
                </li>
              );
            })}
          </ul>
          {checked.items_dropped > 0 && (
            <p className="ml-[26px] text-meta text-zinc-400" data-check-dropped>
              {droppedLine(checked.items_dropped)}
            </p>
          )}
        </>
      ) : (
        <p className="ml-[26px] text-meta leading-6 text-zinc-400" data-check-help>
          {EMPTY_HELP}
          {onAddCheck && (
            <>
              {' '}
              <button type="button" onClick={onAddCheck} className={linkBtn} data-compact data-add-check>
                {ADD_A_CHECK}
              </button>
            </>
          )}
        </p>
      )}
    </section>
  );
};

/** The list row's marker before the summary: the holo sticker for Checked, words for the rest. */
export const CheckMarker: React.FC<{ state: CheckState; summary: React.ReactNode }> = ({ state, summary }) => {
  const has = summary !== null && summary !== undefined && summary !== '';
  if (state === 'checked') {
    return (
      <>
        <Sticker tone="holo" size="sm" tilt={-2} icon={<ShieldCheck size={10} aria-hidden />}>
          {MARKER.checked.word}
        </Sticker>
        {has && <> {summary}</>}
      </>
    );
  }
  const m = MARKER[state];
  return (
    <>
      <span className={m.cls} data-check-marker={state}>
        {m.word}
      </span>
      {has && <> · {summary}</>}
    </>
  );
};
