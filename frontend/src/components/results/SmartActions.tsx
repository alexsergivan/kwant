import React, { useLayoutEffect, useState } from 'react';
import { createPortal } from 'react-dom';

/** The spot in a bot message's action row (next to "Copy markdown") where a block puts its buttons. */
export const SmartActionsSlot: React.FC<{ messageId: string }> = ({ messageId }) => (
  <span data-smart-actions={messageId} className="contents" />
);

/** Same look as the message's "Copy markdown" button. */
export const actionClass =
  'px-2 py-1 rounded-md hover:bg-zinc-900 hover:text-zinc-300 transition-colors flex items-center gap-1.5 text-[11px] cursor-pointer focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-500/70';

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
  if (slot === null) return <div className="mt-1.5 flex items-center gap-2 text-xs text-zinc-500">{children}</div>;
  return null;
};
