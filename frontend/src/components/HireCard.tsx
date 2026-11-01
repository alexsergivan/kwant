import React, { useId, useState } from 'react';
import { AlertTriangle, CheckCircle2, ChevronDown, ChevronRight, Loader2, MinusCircle, Undo2, UserPlus } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import { HireMeta, evidenceText, hireStateText, hireView, hireLeadIndex, hiredBotGone, isTeamHire, modelText } from '../botActions';

export type HireDecision = 'deny' | 'accept_once' | 'always_accept' | 'undo';

export interface HireCardProps {
  meta: HireMeta;
  messageId: string;
  kwantName: string;
  kwantId?: string;
  /** Newer card from `hire_updated` or a button's reply; wins over `meta` unless older. */
  live?: HireMeta | null;
  /** Resolves to an error message on failure. */
  onDecide: (hire: HireMeta, decision: HireDecision) => Promise<string | void>;
  flash?: boolean;
  /** Ids of the bots that exist now; null until loaded. A hire whose bot is gone settles. */
  botIds?: ReadonlySet<string> | null;
}

const btn =
  'inline-flex items-center justify-center gap-1.5 min-h-[40px] sm:min-h-[32px] px-3 rounded-xl text-xs font-medium transition-colors disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2';
const quiet = `${btn} text-zinc-200 border border-zinc-700 hover:bg-zinc-800 focus-visible:outline-zinc-500`;
const spin = <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />;

/**
 * A bot's proposal to hire a new bot: who it would be, which team it joins, why, and what that
 * rests on. The person denies, accepts once or always accepts; an accepted hire can be undone.
 */
export const HireCard: React.FC<HireCardProps> = ({ meta, messageId, kwantName, kwantId, live, onDecide, flash, botIds = null }) => {
  const shown = hireView(meta, live);
  const gone = hiredBotGone(shown, botIds);
  // Nothing to undo once the hired bot is deleted.
  const view = gone ? { ...shown, can_undo: false } : shown;
  const [busy, setBusy] = useState<HireDecision | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showInstructions, setShowInstructions] = useState(false);
  const instructionsId = useId();

  const decide = async (d: HireDecision) => {
    if (busy) return;
    setBusy(d);
    setError(null);
    const err = await onDecide(view, d);
    setBusy(null);
    if (err) setError(err);
  };

  const headingId = `hire-${messageId}`;
  const proposer = view.proposer_name || kwantName;
  const state = hireStateText({ ...view, proposer_name: proposer }, gone);
  const pending = view.status === 'pending';
  const done = view.status === 'denied' || view.status === 'undone' || gone;
  const evidence = evidenceText(view.evidence, view.requested_by_person);
  const border = pending ? 'border-violet-500/40' : done ? 'border-zinc-800' : 'border-emerald-500/25';
  const can = (c: string) => pending && view.choices.includes(c);
  const leadIndex = hireLeadIndex(view);
  const team = isTeamHire(view);
  const toolText = (tools: HireMeta['tools']) => (tools.length ? tools.map((t) => t.name || 'Unknown tool').join(', ') : 'None');

  return (
    <section data-message-id={messageId} aria-labelledby={headingId} className={`flex items-start gap-3.5 w-full max-w-5xl rounded-xl ${flash ? 'kw-flash' : ''}`}>
      <ModernAvatar role={kwantName} name={kwantName} size={34} kwantId={kwantId} />
      <div className={`flex-1 min-w-0 max-w-2xl rounded-2xl rounded-tl-sm border bg-zinc-900/60 shadow-sm ${border}`}>
        <div className="px-4 pt-3 pb-2 space-y-1.5">
          <span className="flex items-center gap-1.5 text-[11px] font-medium text-zinc-400">
            <UserPlus size={12} className="text-violet-300" aria-hidden="true" />
            <span>
              <span className="text-zinc-200">{proposer}</span>
              {team
                ? view.status === 'auto_accepted'
                  ? ` hired a team of ${view.members.length}`
                  : pending
                    ? ` proposes hiring a team of ${view.members.length}`
                    : ` proposed hiring a team of ${view.members.length}`
                : view.status === 'auto_accepted'
                  ? ' hired a new bot'
                  : pending
                    ? ' proposes hiring a new bot'
                    : ' proposed hiring a new bot'}
            </span>
          </span>
          {team ? (
            <h3 id={headingId} className={`text-sm font-semibold break-words leading-snug ${done ? 'text-zinc-400' : 'text-zinc-100'}`}>
              {view.new_team?.name ?? view.team?.name ?? `${view.members.length} new bots`}
            </h3>
          ) : (
            <div className="flex items-center gap-2.5 min-w-0">
              <ModernAvatar role={view.role || view.name} name={view.name} size={28} kwantId={view.hired_kwant_id ?? undefined} />
              <div className="min-w-0">
                <h3 id={headingId} className={`text-sm font-semibold break-words leading-snug ${done ? 'text-zinc-400' : 'text-zinc-100'}`}>
                  {view.name}
                </h3>
                {view.role && <p className="text-xs text-zinc-400 break-words">{view.role}</p>}
              </div>
            </div>
          )}
          {view.reason && (
            <p className="text-sm text-zinc-300 break-words leading-relaxed">
              <span className="text-zinc-500">Why: </span>
              {view.reason}
            </p>
          )}
          {evidence && (
            <p className="text-sm text-zinc-300 break-words leading-relaxed">
              <span className="text-zinc-500">Based on: </span>
              {evidence}
            </p>
          )}
        </div>

        {team ? (
          <>
            <ul aria-label="Proposed members" className="mx-4 mb-2 divide-y divide-zinc-800 rounded-xl border border-zinc-800 bg-zinc-950/70 text-xs">
              {view.members.map((m, i) => (
                <li key={`${m.name}-${i}`} className="flex items-start gap-2.5 px-3 py-2 min-w-0">
                  <ModernAvatar role={m.role || m.name} name={m.name} size={24} kwantId={view.hired_kwant_ids[i]} />
                  <div className="min-w-0 flex-1">
                    <p className={`font-medium break-words ${done ? 'text-zinc-400' : 'text-zinc-100'}`}>
                      {m.name}
                      {i === leadIndex && (
                        <span className="ml-2 rounded-full border border-violet-500/40 bg-violet-500/10 px-1.5 py-px text-[10px] font-medium text-violet-200">Lead</span>
                      )}
                    </p>
                    {m.role && <p className="text-zinc-400 break-words">{m.role}</p>}
                    <p className="text-[11px] text-zinc-500 break-words">
                      {modelText(m.model)} · Tools: {toolText(m.tools)}
                    </p>
                  </div>
                </li>
              ))}
            </ul>
            <dl className="mx-4 mb-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 px-1 text-xs">
              <dt className="text-zinc-500">Team</dt>
              <dd className="min-w-0 break-words text-zinc-200">
                {view.new_team
                  ? `New team: ${view.new_team.name}${leadIndex >= 0 ? `, led by ${view.members[leadIndex].name}` : ''}`
                  : view.team ? view.team.name : 'No team'}
              </dd>
            </dl>
          </>
        ) : (
          <dl className="mx-4 mb-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 rounded-xl border border-zinc-800 bg-zinc-950/70 px-3 py-2 text-xs">
            <dt className="text-zinc-500">Team</dt>
            <dd className="min-w-0 break-words text-zinc-200">{view.team ? view.team.name : 'No team'}</dd>
            <dt className="text-zinc-500">Model</dt>
            <dd className="min-w-0 break-words text-zinc-200">{modelText(view.model)}</dd>
            <dt className="text-zinc-500">Tools</dt>
            <dd className="min-w-0 break-words text-zinc-200">{toolText(view.tools)}</dd>
          </dl>
        )}

        {(team ? view.members.some((m) => m.instructions) : view.instructions) && (
          <div className="mx-4 mb-2">
            <button
              type="button"
              onClick={() => setShowInstructions((v) => !v)}
              aria-expanded={showInstructions}
              aria-controls={instructionsId}
              className="inline-flex items-center gap-1 min-h-[32px] text-xs text-zinc-300 hover:text-zinc-100 rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400"
            >
              {showInstructions ? <ChevronDown size={13} aria-hidden="true" /> : <ChevronRight size={13} aria-hidden="true" />}
              {team ? (showInstructions ? 'Hide their instructions' : 'Show their instructions') : showInstructions ? 'Hide its instructions' : 'Show its instructions'}
            </button>
            {showInstructions && (
              <p
                id={instructionsId}
                className="mt-1 max-h-64 overflow-y-auto rounded-xl border border-zinc-800 bg-zinc-950/70 px-3 py-2 text-xs text-zinc-300 whitespace-pre-wrap break-words leading-relaxed"
              >
                {team ? view.members.filter((m) => m.instructions).map((m) => `${m.name}\n${m.instructions}`).join('\n\n') : view.instructions}
              </p>
            )}
          </div>
        )}

        <div className="px-4 pb-3 pt-0.5 space-y-2">
          <p role="status" aria-live="polite" className="text-[11px]">
            <span className={`inline-flex items-start gap-1.5 ${state.tone === 'ok' ? 'text-emerald-300' : state.tone === 'wait' ? 'text-violet-200' : 'text-zinc-400'}`}>
              {state.tone === 'ok' ? (
                <CheckCircle2 size={13} className="mt-px shrink-0" aria-hidden="true" />
              ) : state.tone === 'off' ? (
                <MinusCircle size={13} className="mt-px shrink-0" aria-hidden="true" />
              ) : (
                <UserPlus size={13} className="mt-px shrink-0" aria-hidden="true" />
              )}
              <span>{state.label}</span>
            </span>
          </p>
          {view.note && <p className="text-[11px] text-amber-200 break-words">{view.note}</p>}

          {pending && (
            <div className="flex flex-wrap items-center gap-2">
              {can('accept_once') && (
                <button
                  type="button"
                  onClick={() => decide('accept_once')}
                  disabled={busy !== null}
                  className={`${btn} bg-violet-500 text-white hover:bg-violet-400 focus-visible:outline-violet-400`}
                >
                  {busy === 'accept_once' && spin}
                  {busy === 'accept_once' ? 'Hiring…' : team ? `Approve all ${view.members.length}` : 'Accept once'}
                </button>
              )}
              {can('always_accept') && (
                <button type="button" onClick={() => decide('always_accept')} disabled={busy !== null} className={quiet}>
                  {busy === 'always_accept' && spin}
                  {busy === 'always_accept' ? 'Hiring…' : 'Always accept'}
                </button>
              )}
              {can('deny') && (
                <button type="button" onClick={() => decide('deny')} disabled={busy !== null} className={quiet}>
                  {busy === 'deny' ? 'Saying no…' : team ? 'Decline' : 'Deny'}
                </button>
              )}
            </div>
          )}
          {pending && can('always_accept') && (
            <p className="text-[11px] text-zinc-500">
              Always accept hires later proposals in this space at once, up to 3 a day, each with Undo. You can change this in Settings › This space.
            </p>
          )}

          {view.can_undo && (
            <button type="button" onClick={() => decide('undo')} disabled={busy !== null} className={quiet}>
              {busy === 'undo' ? spin : <Undo2 size={13} aria-hidden="true" />}
              {busy === 'undo' ? 'Undoing…' : 'Undo'}
            </button>
          )}

          {error && (
            <p role="alert" className="flex items-start gap-1.5 rounded-lg border border-rose-500/30 bg-rose-500/10 px-2.5 py-1.5 text-xs text-rose-200 break-words">
              <AlertTriangle size={13} className="mt-px shrink-0" aria-hidden="true" />
              <span>{error}</span>
            </p>
          )}
        </div>
      </div>
    </section>
  );
};
