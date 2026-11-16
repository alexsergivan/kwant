// 22d: a customer's call nobody allowed, kept in that customer's chat as one quiet settled line.
// Nothing to answer: the call is already refused and the customer was told nothing new.
import React from 'react';
import { ShieldX } from 'lucide-react';
import { approvalAction, type PendingApproval } from './ModernApprovalCard';
import { toolAction } from '../toolLabel';
import { localClock } from '../clock';

/** kwant's own tools (the server's SELF_SERVER_NAME): the owner reads "use memory search", not "(kwnt_tools)". */
const OWN_TOOLS = 'kwnt_tools';

/** The call in the card's words, without kwant's internal server name. */
function refusedAction(approval: PendingApproval): string {
  const d = approval.details ?? {};
  const own = /^mcp__kwnt_tools__(.+)$/.exec(approval.tool_name ?? '');
  if (d.server === OWN_TOOLS || own) return toolAction(typeof d.tool === 'string' && d.tool ? d.tool : own?.[1] ?? approval.tool_name);
  return approvalAction(approval);
}

/** "A customer's message tried to <what the card would ask>. It didn't run, because nobody allowed it." */
export const refusedCallText = (approval: PendingApproval): string =>
  `A customer's message tried to ${refusedAction(approval)}. It didn't run, because nobody allowed it.`;

/** The line, then " · <time>" when the row has one, in the line's own zinc-400 (zinc-500 fails AA at 12 px). */
export const RefusedCustomerCall: React.FC<{ approval: PendingApproval }> = ({ approval }) => {
  const time = localClock(approval.asked_at ?? null);
  return (
    <div className="flex items-start gap-2 ml-0 sm:ml-[50px] max-w-3xl px-3 py-2 rounded-xl border border-zinc-800 bg-zinc-900/50 text-xs text-zinc-400" data-refused-call={approval.id}>
      <ShieldX size={14} className="mt-0.5 shrink-0 text-zinc-500" aria-hidden="true" />
      <p className="min-w-0 break-words leading-5">
        {refusedCallText(approval)}{time && <> · {time}</>}
      </p>
    </div>
  );
};
