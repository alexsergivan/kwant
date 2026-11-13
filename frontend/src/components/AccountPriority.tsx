import React, { useEffect, useMemo, useRef, useState } from 'react';
import { MoreHorizontal } from 'lucide-react';
import { playSfx } from '../audio/chiptune';
import type { EngineInfo } from '../engines';
import { useEscapeLayer } from '../escape';
import {
  accountGroups,
  accountWhere,
  engineLabel,
  failoverOrder,
  moveAccount,
  moveAnnouncement,
  orderIntro,
  priorityStatus,
  priorityUpdates,
} from '../failover';
import { Profile, activeLimit, cardOf, profilesApi } from '../profiles';
import { listNames } from '../appsAccounts';
import { StatusPill } from './EngineAccounts';

const smallBtn =
  'inline-flex items-center justify-center gap-1 min-h-[44px] sm:min-h-[28px] px-2.5 rounded-md border text-[11px] font-medium transition-colors disabled:opacity-40 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-sky-400';
const plainBtn = `${smallBtn} border-zinc-700/70 bg-zinc-800/70 text-zinc-100 hover:bg-zinc-700`;
const iconBtn =
  'inline-flex items-center justify-center w-11 h-11 sm:w-7 sm:h-7 rounded-md text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-sky-400';
const linkBtn = 'inline-flex items-center min-h-[44px] sm:min-h-0 text-[11px] font-medium text-sky-300 hover:text-sky-200 hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400 rounded';
const groupTitle = 'text-xs font-semibold text-zinc-200';
const quietGroupTitle = 'text-xs font-semibold text-zinc-400';

export interface AccountPriorityProps {
  profiles: Profile[];
  engines: EngineInfo[] | null;
  onChanged: () => Promise<void>;
  /** The space, for the intro line. */
  spaceName?: string;
  /** Bot names per account id ("Used by Ann, Ben"). With `pinnedOnly`, only bots pinned to the account's
   * engine: bots on auto start on the "Used first" account, so backups and off accounts don't name them. */
  usedBy?: (profileId: string, pinnedOnly?: boolean) => string[];
  /** The existing sign-in and set-up paths (the engine's card under "Sign-ins and settings"). */
  onSignIn?: (p: Profile) => void;
  onSetUp?: (p: Profile) => void;
  onAddAccount?: () => void;
  /** Turns a whole engine back on (the engine's own switch). */
  onEngineOn?: (engine: EngineInfo['id']) => Promise<boolean | void>;
  /** Turns a whole engine off: "Turn off" on the only account its engine has on. */
  onEngineOff?: (engine: EngineInfo['id']) => Promise<boolean | void>;
  now?: number;
}

/**
 * The AI accounts, the way turns use them: "Used first", then "If it runs out", then "Not used" for the
 * accounts that are off, signed out or not installed. The order is the failover priority, saved as each
 * account's `priority` (PATCH /api/profiles/:id); nothing new on the server. "Use first" is the one
 * visible move; Move up, Move down and Turn off are in each row's More menu, for the keyboard and phones.
 */
export const AccountPriority: React.FC<AccountPriorityProps> = ({
  profiles,
  engines,
  onChanged,
  spaceName,
  usedBy,
  onSignIn,
  onSetUp,
  onAddAccount,
  onEngineOn,
  onEngineOff,
  now = Date.now(),
}) => {
  // The order shown while a save is in flight; null means "what the server last said".
  const [pending, setPending] = useState<Profile[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [announce, setAnnounce] = useState('');
  const [focusKey, setFocusKey] = useState<string | null>(null);
  const rootRef = useRef<HTMLElement>(null);

  const engineOf = (p: Profile) => engines?.find((e) => e.id === cardOf(p));
  const groups = useMemo(
    () => accountGroups(failoverOrder(profiles), (p) => engines?.find((e) => e.id === cardOf(p)), now),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [profiles, engines],
  );
  const used = pending ?? groups.used;
  const rest = groups.notUsed.map((x) => x.p);

  // Keyboard focus follows the row that moved.
  useEffect(() => {
    if (!focusKey) return;
    const el = rootRef.current?.querySelector<HTMLButtonElement>(`[data-focus-key="${CSS.escape(focusKey)}"]`);
    // While the save is in flight the row's buttons are disabled: wait for them, then focus.
    if (el?.disabled) return;
    el?.focus();
    setFocusKey(null);
  }, [focusKey, used, profiles]);

  const nameOf = (p: Profile) => `${engineLabel(p)} (${p.kind === 'system' ? 'on this computer' : p.name})`;
  /** "Used by Ann" on the first row and on Not used. On a backup row the bots there start on it (pinned), so "Cody uses it first". */
  const usedLine = (p: Profile, place: 'first' | 'backup' | 'off' = 'first') => {
    const backup = place === 'backup';
    // Bots on auto start on the "Used first" account; a backup or an off account names only bots pinned to it.
    const names = usedBy?.(p.id, place !== 'first') ?? [];
    if (!names.length) return null;
    const who = names.length > 3 ? `${names.slice(0, 2).join(', ')} and ${names.length - 2} more` : listNames(names);
    return backup ? `${who} ${names.length === 1 ? 'uses' : 'use'} it first` : `Used by ${who}`;
  };

  // The server's own words never reach the person: say what didn't happen, and that nothing changed.
  const fail = (_e: unknown, what: string) => {
    // The alert is read out; the polite status stays quiet so it isn't said twice.
    setError(`${what} It is back as it was.`);
    setAnnounce('');
  };

  const saveOrder = async (next: Profile[], moved: Profile, to: number) => {
    const updates = priorityUpdates(next, rest);
    if (updates.length === 0) return;
    setError(null);
    // Shown at once, with the numbers it will have, so a late reload can't flip it back.
    setPending(next.map((p) => ({ ...p, priority: updates.find((u) => u.id === p.id)?.priority ?? p.priority })));
    setAnnounce(moveAnnouncement(nameOf(moved), to, next.length));
    try {
      await Promise.all(updates.map((u) => profilesApi.update(u.id, { priority: u.priority })));
      await onChanged();
    } catch (e) {
      fail(e, 'Could not save the order.');
      await onChanged().catch(() => undefined);
    } finally {
      setPending(null);
    }
  };

  const move = (from: number, to: number) => {
    const next = moveAccount(used, from, to);
    if (!next) return;
    playSfx('click');
    setFocusKey(`${used[from].id}:more`);
    void saveOrder(next, used[from], to);
  };

  /** On again, at the end of "If it runs out": the engine's own switch when the whole engine is off. */
  const turnOn = async (p: Profile, engineOff: boolean) => {
    setError(null);
    setBusy(p.id);
    try {
      // The engine switch says no by returning false (it puts itself back); stop before any "is on".
      if (engineOff && onEngineOn && (await onEngineOn(cardOf(p))) === false) throw new Error('engine switch failed');
      const others = rest.filter((x) => x.id !== p.id);
      const updates = priorityUpdates([...used, p], others);
      const mine = updates.find((u) => u.id === p.id);
      await Promise.all([
        ...(p.enabled && !mine ? [] : [profilesApi.update(p.id, { ...(p.enabled ? {} : { enabled: true }), ...(mine ? { priority: mine.priority } : {}) })]),
        ...updates.filter((u) => u.id !== p.id).map((u) => profilesApi.update(u.id, { priority: u.priority })),
      ]);
      await onChanged();
      setAnnounce(`${nameOf(p)} is on. Your bots use it if the ones above run out.`);
      setFocusKey(`${p.id}:more`);
    } catch (e) {
      fail(e, `Couldn’t turn on ${nameOf(p)}.`);
      await onChanged().catch(() => undefined);
      // A Not used row has no More menu: focus goes back to the row's Turn on.
      setFocusKey(`${p.id}:on`);
    } finally {
      setBusy(null);
    }
  };

  /**
   * Turn off. The server keeps one main (default) account per engine on, so for the main account:
   * another account of that engine that's on becomes the main one first (the next in the failover
   * order); with none, the engine's own switch turns the whole engine off.
   */
  const turnOff = async (p: Profile) => {
    setError(null);
    setBusy(p.id);
    const engine = cardOf(p);
    const next = p.is_default ? failoverOrder(profiles).find((x) => x.id !== p.id && x.enabled && cardOf(x) === engine) : undefined;
    let moved = false;
    try {
      if (p.is_default && !next) {
        if (!onEngineOff) throw new Error('no engine switch');
        if ((await onEngineOff(engine)) === false) throw new Error('engine switch failed');
        await onChanged();
        setAnnounce(`${nameOf(p)} is off. Your bots skip ${engineLabel(p)}.`);
      } else {
        if (next) {
          await profilesApi.update(next.id, { is_default: true });
          moved = true;
        }
        await profilesApi.update(p.id, { enabled: false });
        await onChanged();
        setAnnounce(
          next
            ? `${nameOf(p)} is off. ${next.kind === 'system' ? 'The sign-in on this computer' : next.name} is now ${engineLabel(p)}’s main account.`
            : `${nameOf(p)} is off. Your bots skip it.`,
        );
      }
      setFocusKey(`${p.id}:on`);
    } catch (e) {
      // Half done (the main account moved, this one stayed on): put the main account back.
      if (moved) await profilesApi.update(p.id, { is_default: true }).catch(() => undefined);
      fail(e, `Couldn’t turn off ${nameOf(p)}.`);
      await onChanged().catch(() => undefined);
      setFocusKey(`${p.id}:more`);
    } finally {
      setBusy(null);
    }
  };

  const first = used[0];
  const backups = used.slice(1);
  const ready = (p: Profile) => priorityStatus(p, engineOf(p), now).tone === 'ok';
  const firstOut = first && first.enabled ? activeLimit(first.limited_until, now) : null;
  const stepIn = backups.find(ready);
  const offToTurnOn = groups.notUsed.find((x) => x.reason === 'off');

  const row = (p: Profile, i: number) => {
    const status = priorityStatus(p, engineOf(p), now);
    const line = usedLine(p, i > 0 ? 'backup' : 'first');
    return (
      <li key={p.id} data-account={p.id} data-used-index={i} className="flex items-start gap-2 rounded-lg border border-zinc-800 bg-zinc-950/40 px-3 py-2.5">
        <div className="flex-1 min-w-0">
          <p className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <span className="text-xs font-medium text-zinc-100" data-account-title>
              {engineLabel(p)}
            </span>
            <StatusPill tone={status.tone}>{status.label}</StatusPill>
          </p>
          <p className="mt-0.5 text-[11px] text-zinc-400 break-words">
            {accountWhere(p)}
            {line && <span className="text-zinc-300"> · {line}</span>}
          </p>
          {i === 0 && firstOut && (
            <p className={`mt-0.5 text-[11px] ${stepIn ? 'text-zinc-300' : 'text-amber-200'}`} data-step-in>
              {stepIn ? `Bots use ${engineLabel(stepIn)} until then.` : 'Bots wait until then.'}
            </p>
          )}
        </div>
        <div className="flex shrink-0 items-center gap-1">
          {i > 0 && (
            <button type="button" className={plainBtn} data-use-first data-focus-key={`${p.id}:first`} disabled={!!pending} onClick={() => move(i, 0)}>
              Use first<span className="sr-only"> {nameOf(p)}</span>
            </button>
          )}
          <RowMenu
            label={nameOf(p)}
            focusKey={`${p.id}:more`}
            disabled={!!pending || busy === p.id}
            items={[
              { label: 'Move up', disabled: i === 0, run: () => move(i, i - 1) },
              { label: 'Move down', disabled: i === used.length - 1, run: () => move(i, i + 1) },
              { label: 'Turn off', run: () => void turnOff(p) },
            ]}
          />
        </div>
      </li>
    );
  };

  return (
    <section ref={rootRef} aria-labelledby="account-order-title" className="space-y-3 text-xs" data-account-order>
      <p id="account-order-title" className="text-[11px] text-zinc-400">
        {orderIntro(spaceName ?? 'this space')}
      </p>

      {first ? (
        <div data-group="first">
          <h4 id="account-group-first" className={groupTitle}>
            Used first
          </h4>
          <ul aria-labelledby="account-group-first" className="mt-1.5 space-y-1.5">
            {row(first, 0)}
          </ul>
        </div>
      ) : (
        <p className="text-zinc-400" data-none-ready>
          {groups.notUsed.length === 0 ? 'No AI accounts yet.' : 'No account is ready to use yet. Turn one on or sign in below.'}{' '}
          {groups.notUsed.length === 0 && onAddAccount && (
            <button type="button" className={linkBtn} onClick={onAddAccount}>
              Add an account
            </button>
          )}
        </p>
      )}

      {first && backups.length > 0 && (
        <div data-group="backup">
          <h4 id="account-group-backup" className={groupTitle}>
            If it runs out
          </h4>
          <ul aria-labelledby="account-group-backup" className="mt-1.5 space-y-1.5">
            {backups.map((p, j) => row(p, j + 1))}
          </ul>
        </div>
      )}

      {first && backups.length === 0 && (
        <div data-group="backup">
          <h4 id="account-group-backup" className={groupTitle}>
            If it runs out
          </h4>
          <p className="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1.5 rounded-lg border border-dashed border-zinc-800 px-3 py-2.5 text-[11px] text-zinc-400" data-no-backup>
          <span>
            No backup yet. If {engineLabel(first)} runs out, your bots wait until it resets.
          </span>
          {offToTurnOn ? (
            <button
              type="button"
              className={plainBtn}
              disabled={busy === offToTurnOn.p.id}
              onClick={() => void turnOn(offToTurnOn.p, offToTurnOn.engineOff)}
              data-no-backup-action
            >
              Turn on {engineLabel(offToTurnOn.p)}
            </button>
          ) : (
            onAddAccount && (
              <button type="button" className={linkBtn} onClick={onAddAccount} data-no-backup-action>
                Add an account
              </button>
            )
          )}
          </p>
        </div>
      )}

      {groups.notUsed.length > 0 && (
        <div data-group="not-used">
          <h4 id="account-group-off" className={quietGroupTitle}>
            Not used
          </h4>
          <ul aria-labelledby="account-group-off" className="mt-1.5 space-y-1.5">
            {groups.notUsed.map(({ p, reason, engineOff }) => (
              <li
                key={p.id}
                data-account={p.id}
                data-not-used={reason}
                className="flex flex-wrap items-center gap-x-2 gap-y-1.5 rounded-lg border border-dashed border-zinc-800 bg-zinc-950/20 px-3 py-2"
              >
                <div className="flex-1 min-w-[10rem]">
                  <p className="text-xs font-medium text-zinc-300" data-account-title>
                    {engineLabel(p)}
                  </p>
                  <p className="mt-0.5 text-[11px] text-zinc-500 break-words">
                    {p.kind === 'system' ? 'On this computer' : p.name} ·{' '}
                    {reason === 'off' ? 'Turned off' : reason === 'signed_out' ? 'Signed out' : 'Not installed'}
                    {usedLine(p, 'off') && <span> · {usedLine(p, 'off')}</span>}
                  </p>
                </div>
                {reason === 'off' ? (
                  <button
                    type="button"
                    className={plainBtn}
                    data-focus-key={`${p.id}:on`}
                    data-turn-on
                    disabled={busy === p.id}
                    onClick={() => void turnOn(p, engineOff)}
                  >
                    Turn on<span className="sr-only"> {nameOf(p)}</span>
                  </button>
                ) : reason === 'signed_out' ? (
                  <button
                    type="button"
                    className={usedLine(p, 'off') ? `${smallBtn} border-amber-500/40 bg-amber-500/10 text-amber-100 hover:bg-amber-500/20` : linkBtn}
                    data-sign-in
                    data-quiet-sign-in={usedLine(p, 'off') ? undefined : ''}
                    onClick={() => onSignIn?.(p)}
                    disabled={!onSignIn}
                  >
                    Sign in<span className="sr-only"> to {nameOf(p)}</span>
                  </button>
                ) : (
                  <button type="button" className={plainBtn} data-set-up onClick={() => onSetUp?.(p)} disabled={!onSetUp}>
                    Set up<span className="sr-only"> {nameOf(p)}</span>
                  </button>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}

      <p role="status" aria-live="polite" className="sr-only">
        {announce}
      </p>
      {error && (
        <p role="alert" className="text-[11px] text-rose-300" data-order-error>
          {error}
        </p>
      )}
    </section>
  );
};

/** A row's More menu: Move up, Move down, Turn off. Arrow keys move between items; Escape closes it and
 * puts focus back on its button. */
const RowMenu: React.FC<{
  label: string;
  focusKey: string;
  disabled?: boolean;
  items: Array<{ label: string; disabled?: boolean; run: () => void }>;
}> = ({ label, focusKey, disabled, items }) => {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);
  const btnRef = useRef<HTMLButtonElement>(null);
  const close = (refocus = true) => {
    setOpen(false);
    if (refocus) btnRef.current?.focus();
  };
  useEscapeLayer(wrapRef, open, () => close());

  useEffect(() => {
    if (!open) return;
    wrapRef.current?.querySelector<HTMLButtonElement>('[role="menuitem"]:not([disabled])')?.focus();
    const away = (e: MouseEvent) => {
      if (!wrapRef.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', away);
    return () => document.removeEventListener('mousedown', away);
  }, [open]);

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp' && e.key !== 'Home' && e.key !== 'End') return;
    const list = [...(wrapRef.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not([disabled])') ?? [])];
    if (!list.length) return;
    e.preventDefault();
    const at = list.indexOf(document.activeElement as HTMLButtonElement);
    const next = e.key === 'Home' ? 0 : e.key === 'End' ? list.length - 1 : (at + (e.key === 'ArrowDown' ? 1 : -1) + list.length) % list.length;
    list[next].focus();
  };

  return (
    <div ref={wrapRef} className="relative" onKeyDown={onKey}>
      <button
        ref={btnRef}
        type="button"
        className={iconBtn}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={`More for ${label}`}
        title="More"
        data-focus-key={focusKey}
        data-row-menu
        disabled={disabled}
        onClick={() => setOpen((o) => !o)}
      >
        <MoreHorizontal size={15} aria-hidden="true" />
      </button>
      {open && (
        <div role="menu" aria-label={`More for ${label}`} className="absolute right-0 top-full z-20 mt-1 w-36 rounded-lg border border-zinc-700 bg-zinc-900 p-1 shadow-xl" data-row-menu-list>
          {items.map((it) => (
            <button
              key={it.label}
              type="button"
              role="menuitem"
              disabled={it.disabled}
              className="block w-full rounded-md px-2.5 py-2 sm:py-1.5 min-h-[44px] sm:min-h-0 text-left text-xs text-zinc-200 hover:bg-zinc-800 focus:bg-zinc-800 focus:outline-none focus-visible:outline focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-sky-400 disabled:opacity-40 disabled:hover:bg-transparent"
              onClick={() => {
                close(false);
                it.run();
              }}
            >
              {it.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
};
