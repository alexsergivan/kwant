import React, { useEffect, useMemo, useRef, useState } from 'react';
import { ArrowDown, ArrowUp, GripVertical } from 'lucide-react';
import { playSfx } from '../audio/chiptune';
import type { EngineInfo } from '../engines';
import {
  FAILOVER_CAPTION,
  engineLabel,
  failoverOrder,
  moveAccount,
  moveAnnouncement,
  priorityStatus,
  priorityUpdates,
  splitRanked,
} from '../failover';
import { Profile, cardOf, profilesApi } from '../profiles';
import { StatusPill } from './EngineAccounts';

const arrowBtn =
  'inline-flex items-center justify-center w-8 h-8 sm:w-7 sm:h-7 rounded-md border border-zinc-700/70 bg-zinc-800/70 text-zinc-200 hover:bg-zinc-700 transition-colors disabled:opacity-35 disabled:hover:bg-zinc-800/70 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-sky-400';

/**
 * One list across engines, in the order turns try them. The order is the failover priority: it is
 * saved as each account's `priority` (PATCH /api/profiles/:id), so the server needs nothing new.
 * Drag a row, or use its up and down buttons (they work from the keyboard and on a phone).
 */
export const AccountPriority: React.FC<{
  profiles: Profile[];
  engines: EngineInfo[] | null;
  onChanged: () => Promise<void>;
  now?: number;
}> = ({ profiles, engines, onChanged, now = Date.now() }) => {
  // The order shown while a save is in flight; null means "what the server last said".
  const [pending, setPending] = useState<Profile[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [announce, setAnnounce] = useState('');
  const [dragId, setDragId] = useState<string | null>(null);
  const [overId, setOverId] = useState<string | null>(null);
  const [focusKey, setFocusKey] = useState<string | null>(null);
  const listRef = useRef<HTMLOListElement>(null);

  const engineOf = (p: Profile) => engines?.find((e) => e.id === cardOf(p));
  // Accounts that aren't installed or aren't signed in can't take a turn: they are listed apart, unranked
  // and not movable. Their saved priority is never touched.
  const { ranked: servedRanked, unranked } = useMemo(
    () => splitRanked(failoverOrder(profiles), (p) => engines?.find((e) => e.id === cardOf(p)), now),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [profiles, engines],
  );
  const order = pending ?? servedRanked;

  // Put keyboard focus back on the button that was used, once the row has moved.
  useEffect(() => {
    if (!focusKey) return;
    const el = listRef.current?.querySelector<HTMLButtonElement>(`[data-focus-key="${CSS.escape(focusKey)}"]`);
    if (el && !el.disabled) el.focus();
    setFocusKey(null);
  }, [focusKey, order]);

  const save = async (next: Profile[], moved: Profile, to: number) => {
    const updates = priorityUpdates(next);
    if (updates.length === 0) return;
    setError(null);
    // Show the new order at once, with the numbers it will have, so a late reload can't flip it back.
    const numbered = next.map((p) => ({ ...p, priority: updates.find((u) => u.id === p.id)?.priority ?? p.priority }));
    setPending(numbered);
    setAnnounce(moveAnnouncement(moved.kind === 'system' ? `${engineLabel(moved)} on this machine` : moved.name, to, next.length));
    try {
      await Promise.all(updates.map((u) => profilesApi.update(u.id, { priority: u.priority })));
      await onChanged();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Could not save the order.');
      setAnnounce('Could not save the order. It is back as it was.');
      await onChanged().catch(() => undefined);
    } finally {
      setPending(null);
    }
  };

  const move = (index: number, to: number, key?: string) => {
    const next = moveAccount(order, index, to);
    if (!next) return;
    playSfx('click');
    if (key) setFocusKey(key);
    void save(next, order[index], to);
  };

  const nameOf = (p: Profile) => (p.kind === 'system' ? 'This machine' : p.name);

  return (
    <section aria-labelledby="account-priority-title" className="rounded-xl border border-zinc-800 bg-zinc-900/90 p-3 text-xs">
      <h3 id="account-priority-title" className="font-semibold text-zinc-100">
        Order to use
      </h3>
      <p id="account-priority-caption" className="mt-0.5 text-[11px] text-zinc-400">
        {FAILOVER_CAPTION}
      </p>
      {order.length === 0 ? (
        <p className="mt-2 text-zinc-500">{unranked.length === 0 ? 'No accounts yet. Add one below.' : 'No account is ready to use yet.'}</p>
      ) : (
        <ol ref={listRef} aria-describedby="account-priority-caption" className="mt-2.5 space-y-1.5">
          {order.map((p, i) => {
            const status = priorityStatus(p, engineOf(p), now);
            const dragging = dragId === p.id;
            const over = overId === p.id && dragId !== p.id;
            return (
              <li
                key={p.id}
                data-account={p.id}
                draggable
                onDragStart={(e) => {
                  setDragId(p.id);
                  e.dataTransfer.effectAllowed = 'move';
                  e.dataTransfer.setData('text/plain', p.id);
                }}
                onDragOver={(e) => {
                  if (!dragId) return;
                  e.preventDefault();
                  e.dataTransfer.dropEffect = 'move';
                  if (overId !== p.id) setOverId(p.id);
                }}
                onDrop={(e) => {
                  e.preventDefault();
                  const from = order.findIndex((x) => x.id === dragId);
                  setDragId(null);
                  setOverId(null);
                  move(from, i);
                }}
                onDragEnd={() => {
                  setDragId(null);
                  setOverId(null);
                }}
                className={`flex items-center gap-2 rounded-lg border px-2 py-2 transition-colors ${
                  over ? 'border-sky-500/60 bg-sky-500/5' : 'border-zinc-800 bg-zinc-950/40'
                } ${dragging ? 'opacity-50' : ''} ${p.enabled ? '' : 'opacity-75'}`}
              >
                <GripVertical size={14} className="shrink-0 text-zinc-600 cursor-grab hidden sm:block" aria-hidden="true" />
                <span className="shrink-0 w-4 text-center text-[11px] tabular-nums text-zinc-500" aria-hidden="true">
                  {i + 1}
                </span>
                <div className="flex-1 min-w-0">
                  <p className="text-zinc-100 font-medium truncate">{nameOf(p)}</p>
                  <div className="mt-0.5 flex items-center gap-x-1.5 gap-y-1 flex-wrap">
                    <span className="text-[11px] text-zinc-500">{engineLabel(p)}</span>
                    <StatusPill tone={status.tone}>{status.label}</StatusPill>
                  </div>
                </div>
                <div className="flex shrink-0 items-center gap-1">
                  <button
                    type="button"
                    data-focus-key={`${p.id}:up`}
                    className={arrowBtn}
                    disabled={i === 0}
                    aria-label={`Move ${nameOf(p)} (${engineLabel(p)}) up`}
                    title="Use it earlier"
                    onClick={() => move(i, i - 1, i - 1 === 0 ? `${p.id}:down` : `${p.id}:up`)}
                  >
                    <ArrowUp size={13} aria-hidden="true" />
                  </button>
                  <button
                    type="button"
                    data-focus-key={`${p.id}:down`}
                    className={arrowBtn}
                    disabled={i === order.length - 1}
                    aria-label={`Move ${nameOf(p)} (${engineLabel(p)}) down`}
                    title="Use it later"
                    onClick={() => move(i, i + 1, i + 1 === order.length - 1 ? `${p.id}:up` : `${p.id}:down`)}
                  >
                    <ArrowDown size={13} aria-hidden="true" />
                  </button>
                </div>
              </li>
            );
          })}
        </ol>
      )}
      {unranked.length > 0 && (
        <div data-unranked className="mt-3">
          <h4 id="account-unranked-title" className="text-[11px] font-medium text-zinc-400">
            Not in the order
          </h4>
          <p className="mt-0.5 text-[11px] text-zinc-500">These can't run turns until they are installed or signed in.</p>
          <ul aria-labelledby="account-unranked-title" className="mt-1.5 space-y-1.5">
            {unranked.map((p) => {
              const status = priorityStatus(p, engineOf(p), now);
              return (
                <li key={p.id} data-account={p.id} data-unranked-account className="flex items-center gap-2 rounded-lg border border-dashed border-zinc-800 bg-zinc-950/20 px-2 py-2">
                  <div className="flex-1 min-w-0">
                    <p className="text-zinc-300 font-medium truncate">{nameOf(p)}</p>
                    <div className="mt-0.5 flex items-center gap-x-1.5 gap-y-1 flex-wrap">
                      <span className="text-[11px] text-zinc-500">{engineLabel(p)}</span>
                      <StatusPill tone={status.tone}>{status.label}</StatusPill>
                    </div>
                  </div>
                </li>
              );
            })}
          </ul>
        </div>
      )}
      <p role="status" aria-live="polite" className="sr-only">
        {announce}
      </p>
      {error && (
        <p role="alert" className="mt-2 text-[11px] text-rose-300">
          {error}
        </p>
      )}
    </section>
  );
};
