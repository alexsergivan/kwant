// Swarm Auditor on the task panel (Phase D2, docs/mission-control.md §18): each audit with who
// challenged, how far it got and its tokens; each risk with its evidence, the owner's answer and
// whether it's still open. Only the person closes a risk, and closing one never moves the task.
import React from 'react';
import { MessageSquare, ShieldAlert } from 'lucide-react';
import {
  Audit,
  AuditWaiting,
  NOT_A_PASS_TEXT,
  Risk,
  RiskResolution,
  TaskRisks,
  auditStatusText,
  auditTokens,
  riskState,
  triggerLabel,
  unresolved,
} from '../auditor';
import { formatTokens } from '../missions';

const TONE: Record<string, string> = {
  amber: 'bg-amber-500/15 text-amber-200 border-amber-500/30',
  rose: 'bg-rose-500/15 text-rose-200 border-rose-500/30',
  emerald: 'bg-emerald-500/15 text-emerald-200 border-emerald-500/30',
  zinc: 'bg-zinc-800 text-zinc-300 border-zinc-700',
};
const btn =
  'inline-flex items-center justify-center gap-1.5 min-h-[40px] sm:min-h-[32px] px-3 rounded-lg text-xs font-medium disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500';

export const WAITING_TITLE = 'Audit waiting for your answer';

const RiskItem: React.FC<{
  r: Risk;
  owner: string;
  canResolve: boolean;
  busy: boolean;
  onResolve?: (r: Risk, resolution: RiskResolution) => void;
}> = ({ r, owner, canResolve, busy, onResolve }) => {
  const st = riskState(r.status);
  const open = unresolved(r);
  return (
    <li className={`px-3 py-2 rounded-lg border text-xs space-y-1.5 ${open ? 'border-amber-500/25 bg-amber-500/5' : 'border-zinc-800 bg-zinc-900/30'}`} data-risk={r.id} data-risk-status={r.status}>
      <div className="flex items-start gap-2">
        <span className={`shrink-0 px-1.5 py-0.5 rounded border text-[10px] font-medium ${TONE[st.tone]}`}>{st.label}</span>
        <p className={`flex-1 min-w-0 break-words ${open ? 'text-zinc-100' : 'text-zinc-400'}`}>{r.risk}</p>
      </div>
      <p className="text-[11px] text-zinc-400 break-words">
        <span className="font-medium text-zinc-300">Evidence: </span>
        {r.evidence}
      </p>
      {r.answer && (
        <div className="text-[11px] text-zinc-400 break-words border-l-2 border-zinc-700 pl-2" data-rebuttal>
          <span className="font-medium text-zinc-300">
            {owner} {r.stance === 'disputed' ? 'disputes it' : 'accepts it'}:{' '}
          </span>
          {r.answer}
          {r.answer_evidence && (
            <span className="block mt-0.5">
              <span className="font-medium text-zinc-300">Their evidence: </span>
              {r.answer_evidence}
            </span>
          )}
        </div>
      )}
      {!open && (
        <p className="text-[11px] text-zinc-500" data-closed>
          You {r.status === 'dismissed' ? 'dismissed' : 'resolved'} it{r.note ? `: ${r.note}` : '.'}
        </p>
      )}
      {open && canResolve && onResolve && (
        <div className="flex flex-wrap gap-2 pt-0.5" data-risk-actions>
          <button type="button" disabled={busy} onClick={() => onResolve(r, 'resolved')} className={`${btn} bg-emerald-600 text-white hover:bg-emerald-500`}>
            Resolve
          </button>
          <button type="button" disabled={busy} onClick={() => onResolve(r, 'dismissed')} className={`${btn} border border-zinc-700 bg-zinc-800/80 text-zinc-100 hover:bg-zinc-700`}>
            Dismiss
          </button>
        </div>
      )}
    </li>
  );
};

const AuditBlock: React.FC<{
  a: Audit;
  names: Record<string, string>;
  canResolve: boolean;
  busy: boolean;
  onResolve?: (r: Risk, resolution: RiskResolution) => void;
}> = ({ a, names, canResolve, busy, onResolve }) => {
  const challenger = names[a.challenger_kwant_id] || 'A bot';
  const owner = names[a.owner_kwant_id] || 'The owner';
  const tokens = auditTokens(a);
  return (
    <li className="space-y-2" data-audit={a.id} data-audit-status={a.status}>
      <div className="text-[11px] text-zinc-400 break-words">
        <span className="font-medium text-zinc-200">Audit by {challenger}</span>
        <span> · {triggerLabel(a.trigger)}</span>
        {tokens > 0 && <span className="tabular-nums"> · {formatTokens(tokens)} tokens</span>}
        <span className="block" data-audit-line>
          {auditStatusText(a, names)}
        </span>
      </div>
      {a.risks.length > 0 && (
        <ul className="space-y-1.5">
          {a.risks.map((r) => (
            <RiskItem key={r.id} r={r} owner={owner} canResolve={canResolve} busy={busy} onResolve={onResolve} />
          ))}
        </ul>
      )}
    </li>
  );
};

/** The task's audits; nothing at all when it has never been audited. */
export const RisksSection: React.FC<{
  data: TaskRisks | null;
  names: Record<string, string>;
  /** False after a 403: the person-only buttons go. */
  canResolve?: boolean;
  busy?: boolean;
  waiting?: AuditWaiting | null;
  onResolve?: (r: Risk, resolution: RiskResolution) => void;
  onOpenWaiting?: (w: AuditWaiting) => void;
}> = ({ data, names, canResolve = true, busy = false, waiting = null, onResolve, onOpenWaiting }) => {
  if (!data || data.audits.length === 0) return null;
  const open = data.open_risks;
  return (
    <section aria-labelledby="risks-h" className="space-y-2" data-risks>
      <h4 id="risks-h" className="flex items-center gap-1.5 text-xs font-semibold text-zinc-200">
        <ShieldAlert size={13} className={open > 0 ? 'text-amber-300' : 'text-zinc-400'} aria-hidden="true" />
        Open risks
        <span className={`min-w-[18px] px-1 rounded-full text-[10px] text-center tabular-nums ${open > 0 ? 'bg-amber-500 text-zinc-950' : 'bg-zinc-800 text-zinc-300'}`} data-open-risks={open}>
          {open}
        </span>
      </h4>
      {waiting && (
        <div role="status" className="px-3 py-2 rounded-lg border border-amber-500/30 bg-amber-500/10 flex flex-wrap items-center gap-2 text-xs text-amber-100" data-audit-waiting>
          <span className="flex-1 min-w-[10rem]">
            <span className="block font-medium">{WAITING_TITLE}</span>
            <span className="block text-[11px] text-amber-200/80">
              {names[waiting.kwant_id] || 'A bot'} asked for your OK. The audit stays paused until you answer.
            </span>
          </span>
          {onOpenWaiting && (
            <button type="button" onClick={() => onOpenWaiting(waiting)} className={`${btn} border border-amber-500/40 hover:bg-amber-500/10`} data-open-audit-card>
              <MessageSquare size={13} aria-hidden="true" /> Answer in chat
            </button>
          )}
        </div>
      )}
      <ul className="space-y-3">
        {data.audits.map((a) => (
          <AuditBlock key={a.id} a={a} names={names} canResolve={canResolve} busy={busy} onResolve={onResolve} />
        ))}
      </ul>
      <p className="text-[11px] text-zinc-500" data-not-a-pass>
        {NOT_A_PASS_TEXT}
      </p>
    </section>
  );
};
