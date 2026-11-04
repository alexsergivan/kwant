import React, { useLayoutEffect, useState } from 'react';
import { createPortal } from 'react-dom';

/** The spot in a bot message's action row (next to "Copy markdown") where a block puts its buttons. */
export const SmartActionsSlot: React.FC<{ messageId: string }> = ({ messageId }) => (
  <span data-smart-actions={messageId} className="contents" />
);

/** The chip every action under a reply shares ("Copy markdown", "Show as text", "Copy data"): a hairline
 *  button with an icon, readable at rest and lifted on hover. 36 px tall, 44 px on touch screens. */
export const actionClass =
  'inline-flex items-center gap-1.5 min-h-9 px-3 rounded-lg border border-hairline-strong bg-white/[0.04] text-xs font-medium whitespace-nowrap shrink-0 text-zinc-200 hover:text-zinc-50 hover:bg-white/[0.08] hover:border-white/20 active:scale-[0.97] transition duration-150 motion-reduce:transition-none motion-reduce:active:scale-100 cursor-pointer focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400';
/** The same chip shape for a fact in the row ("From this reply"): an icon and text, no hover, since it does nothing. */
export const evidenceClass =
  'inline-flex items-center gap-1.5 min-h-9 min-w-0 max-w-full px-3 rounded-lg border border-hairline bg-white/[0.03] text-xs font-medium text-zinc-300 [&>svg]:text-muted';
/** A chip that is switched on (Show as text while showing text). */
export const actionOnClass = 'border-sky-500/50 bg-sky-500/15 text-sky-100 hover:bg-sky-500/20';

/**
 * A block's buttons go in its message's action row, so the reply itself keeps the streamed text's exact
 * size when the block replaces it. Outside a chat (no action row), they sit right under the block.
 */
export const SmartActions: React.FC<{ messageId: string; children: React.ReactNode }> = ({ messageId, children }) => {
  const [slot, setSlot] = useState<Element | null | undefined>(undefined);
  useLayoutEffect(() => {
    const found = [...document.querySelectorAll('[data-smart-actions]')].find((el) => el.getAttribute('data-smart-actions') === messageId);
    setSlot(found ?? null);
  }, [messageId]);
  if (slot) return createPortal(children, slot);
  if (slot === null) return <div className="mt-2 flex flex-wrap items-center gap-2 text-xs text-zinc-400">{children}</div>;
  return null;
};
