import React, { useCallback, useEffect, useState } from 'react';
import { AlertTriangle, Loader2, RefreshCw, UserPlus } from 'lucide-react';
import { HirePolicy, decisionError, hiresApi, parseHirePolicy } from '../botActions';
import { formatWhen } from '../continuity';

const smallBtn =
  'inline-flex items-center gap-1.5 min-h-[36px] sm:min-h-[30px] px-2.5 rounded-lg text-xs font-medium border transition-colors disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500';
const quietBtn = `${smallBtn} border-zinc-700/70 bg-zinc-800/70 text-zinc-200 hover:bg-zinc-700`;

/**
 * Space settings: whether bots here may hire new bots without asking, how many automatic hires
 * are left today, and the kinds of bot the person said no to (which they can clear).
 */
export const HirePolicyPanel: React.FC<{ spaceId: string; spaceName: string }> = ({ spaceId, spaceName }) => {
  const [policy, setPolicy] = useState<HirePolicy | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busy, setBusy] = useState<'policy' | 'clear' | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const p = parseHirePolicy(await hiresApi.policy(spaceId));
      if (!p) throw new Error('The answer made no sense.');
      setPolicy(p);
      setLoadError(null);
    } catch (e) {
      setLoadError(`Couldn’t load the hiring setting: ${e instanceof Error ? e.message : 'something went wrong.'}`);
    }
  }, [spaceId]);

  useEffect(() => {
    load();
    // A hire decided in the chat changes the count and the denials.
    const onHire = () => load();
    window.addEventListener('kwnt:hire-updated', onHire);
    return () => window.removeEventListener('kwnt:hire-updated', onHire);
  }, [load]);

  const save = async (body: { policy?: 'ask' | 'auto'; clear_denials?: boolean }, what: 'policy' | 'clear') => {
    setBusy(what);
    setError(null);
    setNotice(null);
    try {
      const p = parseHirePolicy(await hiresApi.setPolicy(spaceId, body));
      if (p) setPolicy(p);
      setNotice(what === 'clear' ? 'Cleared. Bots may suggest those kinds of bot again.' : 'Saved.');
    } catch (e) {
      setError(decisionError(e, what === 'clear' ? 'clear the list' : 'save it'));
    } finally {
      setBusy(null);
    }
  };

  const name = `hire-policy-${spaceId}`;
  const left = policy ? Math.max(0, policy.auto_hires_per_day - policy.auto_hires_last_24h) : 0;

  return (
    <section aria-labelledby={`${name}-title`} className="rounded-xl border border-zinc-800 bg-zinc-950/60 p-4 space-y-3">
      <div className="flex items-start gap-2">
        <UserPlus size={15} className="mt-0.5 text-violet-300 shrink-0" aria-hidden="true" />
        <div className="min-w-0">
          <h4 id={`${name}-title`} className="text-sm font-semibold text-zinc-100">
            Hiring new bots
          </h4>
          <p className="text-[11px] text-zinc-500 leading-relaxed">
            A bot here may propose a new bot when a specialist would clearly do better, or when you keep asking for the same kind of work. Bots
            can read this setting but can't change it.
          </p>
        </div>
      </div>

      {loadError && (
        <div className="flex flex-wrap items-center justify-between gap-2 bg-rose-500/10 border border-rose-500/20 text-rose-300 px-3 py-2 rounded-lg text-xs" role="alert">
          <span className="min-w-0 break-words">{loadError}</span>
          <button type="button" onClick={load} className={quietBtn}>
            <RefreshCw size={12} aria-hidden="true" /> Try again
          </button>
        </div>
      )}
      {!policy && !loadError && (
        <p className="flex items-center gap-2 text-xs text-zinc-400" role="status">
          <Loader2 size={14} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Loading…
        </p>
      )}

      {policy && (
        <>
          <fieldset className="space-y-1.5" disabled={busy !== null}>
            <legend className="sr-only">When a bot proposes a new bot in {spaceName}</legend>
            {(
              [
                { id: 'ask', label: 'Ask me each time', hint: 'You choose Deny, Accept once or Always accept on each proposal.' },
                {
                  id: 'auto',
                  label: 'Hire at once',
                  hint: `Proposals are hired straight away, each with Undo, up to ${policy.auto_hires_per_day} in 24 hours. After that it asks you again.`,
                },
              ] as const
            ).map((o) => (
              <label
                key={o.id}
                className={`flex items-start gap-2.5 rounded-lg border px-3 py-2 cursor-pointer transition-colors ${
                  policy.policy === o.id ? 'border-violet-500/50 bg-violet-500/10' : 'border-zinc-800 hover:border-zinc-700'
                }`}
              >
                <input
                  type="radio"
                  name={name}
                  value={o.id}
                  checked={policy.policy === o.id}
                  onChange={() => save({ policy: o.id }, 'policy')}
                  className="mt-0.5 accent-violet-500"
                />
                <span className="min-w-0">
                  <span className="block text-xs font-medium text-zinc-100">{o.label}</span>
                  <span className="block text-[11px] text-zinc-500 leading-relaxed">{o.hint}</span>
                </span>
              </label>
            ))}
          </fieldset>

          <p className="text-[11px] text-zinc-400">
            Hired automatically in the last 24 hours:{' '}
            <span className="text-zinc-100 font-medium">
              {policy.auto_hires_last_24h} of {policy.auto_hires_per_day}
            </span>
            {policy.policy === 'auto' && <span className="text-zinc-500"> · {left === 0 ? 'the next one will ask you' : `${left} left`}</span>}
          </p>

          <div className="space-y-1.5">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <h5 className="text-xs font-medium text-zinc-300">You said no to</h5>
              {policy.denied.length > 0 && (
                <button type="button" onClick={() => save({ clear_denials: true }, 'clear')} disabled={busy !== null} className={quietBtn}>
                  {busy === 'clear' && <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
                  Clear denials
                </button>
              )}
            </div>
            {policy.denied.length === 0 ? (
              <p className="text-[11px] text-zinc-500">Nothing. Bots here may propose any kind of bot.</p>
            ) : (
              <ul className="space-y-1" aria-label="Denied bots">
                {policy.denied.map((d) => (
                  <li key={d.hire_id || `${d.name}-${d.denied_at}`} className="flex flex-wrap items-baseline gap-x-2 text-[11px] rounded-lg border border-zinc-800 px-2.5 py-1.5">
                    <span className="text-zinc-100 font-medium break-words">{d.name || 'A bot'}</span>
                    {d.role && <span className="text-zinc-400 break-words">{d.role}</span>}
                    {d.denied_at && <span className="text-zinc-600 ml-auto">{formatWhen(d.denied_at)}</span>}
                  </li>
                ))}
              </ul>
            )}
            {policy.denied.length > 0 && (
              <p className="text-[11px] text-zinc-500">Bots here won't propose these kinds of bot again unless you ask for one.</p>
            )}
          </div>

          {error && (
            <p role="alert" className="flex items-start gap-1.5 text-[11px] text-rose-300 break-words">
              <AlertTriangle size={12} className="mt-px shrink-0" aria-hidden="true" /> {error}
            </p>
          )}
          {notice && !error && (
            <p role="status" className="text-[11px] text-emerald-300">
              {notice}
            </p>
          )}
        </>
      )}
    </section>
  );
};
