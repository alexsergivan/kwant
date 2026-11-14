// reply-chrome part 1 (autonomy/reply-chrome/design.md): the quiet marker on a job's final reply, after the name,
// and the one check line under the card when the Inbox item says how it was checked. No motion, no action: the
// marker is a labelled image, not a control. Server text (the done line, the check line) is drawn as text only.

import React from 'react';
import { CircleCheck, Shield, ShieldCheck, ShieldHalf, ShieldX, type LucideIcon } from 'lucide-react';
import { type CheckState, splitLine } from '../doneCheck';
import { DONE_WORD, type ReplyDone, doneLabel } from '../replyDone';
import { Sticker } from './Sticker';

const PILL = 'inline-flex h-5 shrink-0 items-center gap-1 rounded-full border pl-1.5 pr-2 text-[11px] font-semibold whitespace-nowrap';

const TONE: Record<'done' | Exclude<CheckState, 'checked'>, { cls: string; Icon: LucideIcon }> = {
  done: { cls: 'border-teal-300/30 bg-teal-300/10 text-ok', Icon: CircleCheck },
  partly: { cls: 'border-amber-300/35 bg-amber-300/10 text-warn', Icon: ShieldHalf },
  failed: { cls: 'border-rose-300/35 bg-rose-300/10 text-bad', Icon: ShieldX },
  not_checked: { cls: 'border-hairline-strong bg-transparent text-zinc-300', Icon: CircleCheck },
};

/** The marker on the name line: "Done", or the check's word; holographic only for the earned "Checked". */
export const DoneMarker: React.FC<{ done: ReplyDone }> = ({ done }) => {
  const state = done.checked?.state;
  const label = doneLabel(done);
  const title = done.text ?? undefined;
  if (state === 'checked')
    return (
      <span role="img" aria-label={label} title={title} className="inline-flex shrink-0" data-done-marker="checked">
        <Sticker tone="holo" size="sm" tilt={-2} icon={<ShieldCheck size={10} aria-hidden="true" />}>
          {DONE_WORD.checked}
        </Sticker>
      </span>
    );
  const key = state ?? 'done';
  const { cls, Icon } = TONE[key];
  return (
    <span role="img" aria-label={label} title={title} className={`${PILL} ${cls}`} data-done-marker={key}>
      <Icon size={12} aria-hidden="true" className="shrink-0" />
      <span aria-hidden="true">{DONE_WORD[key]}</span>
    </span>
  );
};

const LINE: Record<CheckState, { cls: string; Icon: LucideIcon }> = {
  checked: { cls: 'text-ok', Icon: ShieldCheck },
  partly: { cls: 'text-warn', Icon: ShieldHalf },
  failed: { cls: 'text-bad', Icon: ShieldX },
  not_checked: { cls: 'text-zinc-300', Icon: Shield },
};

/** The one line under a done reply's card, only when the Inbox item carries a check (#566/#577). */
export const DoneCheckLine: React.FC<{ done: ReplyDone }> = ({ done }) => {
  const c = done.checked;
  if (!c) return null;
  const { prefix, rest } = splitLine(c.state, c.line);
  const { cls, Icon } = LINE[c.state];
  return (
    <p className="mt-2 flex max-w-full min-w-0 items-start gap-1.5 text-xs leading-snug" data-done-check-line={c.state}>
      <Icon size={13} aria-hidden="true" className={`mt-px shrink-0 ${cls}`} />
      <span className="min-w-0 break-words [overflow-wrap:anywhere] text-zinc-300">
        {prefix && <b className={`font-semibold ${cls}`}>{prefix}</b>}
        {prefix ? ' ' : ''}
        {rest}
      </span>
    </p>
  );
};
