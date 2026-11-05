// The Dev desk's one status line (0.7.1, any-git S5). Dev mode turns on with no question asked; the
// server finds the repo in the space's teams and says what it can read. This line says it back, with
// at most one action: Switch (several repos found), Connect GitHub, or Type a repo (none found). Nothing here
// blocks anything: the production floor and the command guards are on in every status.

import React, { useEffect, useId, useRef, useState } from 'react';
import { Check, ChevronDown, FolderGit2, Link2, Loader2 } from 'lucide-react';
import { DevModeState, DevSource, StatusLine, deskIntent, devApi, devStore, statusLine, typedRepo, watchedSource } from '../devMode';

const focusRing = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400';

/** style-v2 §4: the LED mirrors the status. ok only while a remote repo is really watched. */
export function statusLed(line: StatusLine, state: DevModeState): 'ok' | 'warn' | 'off' {
  if (line.status === 'not_connected') return 'warn';
  if (line.status === 'watching') return watchedSource(state)?.from === 'local' ? 'off' : 'ok';
  return 'off';
}

export const sourceWhere = (s: DevSource): string => (s.from === 'repo' ? `${s.team}’s repo link` : s.from === 'local' ? `${s.team}’s folder, no remote` : `${s.team}’s folder`);

export interface DevStatusLineProps {
  spaceId: string;
  state: DevModeState | null;
  /** Opens Loose ends (Connect GitHub). Connect hides without it. */
  onConnect?: () => void;
  /** "Checked 19:58" at the right (style-v2 §4); the LED pings once each time it changes. */
  checked?: string | null;
  checkedAt?: number | null;
  /** Test hook. */
  save?: typeof devApi.putRepo;
}

export const DevStatusLine: React.FC<DevStatusLineProps> = ({ spaceId, state, onConnect, checked = null, checkedAt = null, save = devApi.putRepo }) => {
  const line = statusLine(state);
  // The toast's "Type a repo" lands here with the list open and its input focused.
  const [open, setOpen] = useState<false | 'list' | 'type'>(() => (deskIntent.take(spaceId) ? 'type' : false));
  const switchBtn = useRef<HTMLButtonElement>(null);
  // One 200 ms ping when a check lands, not on first paint.
  const firstCheck = useRef(checkedAt);
  const ping = checkedAt != null && checkedAt !== firstCheck.current;
  if (!line || !state) return null;
  const led = statusLed(line, state);
  const action =
    line.action === 'switch' || line.action === 'type' ? (
      <button
        ref={switchBtn}
        type="button"
        onClick={() => setOpen((o) => (o ? false : line.action === 'type' ? 'type' : 'list'))}
        aria-expanded={!!open}
        aria-haspopup="true"
        data-dv-status-action={line.action}
        className={`act kw-dv-status-act ${focusRing}`}
      >
        {line.action === 'type' ? 'Type a repo' : 'Switch'}
        {line.action === 'switch' && <ChevronDown size={12} aria-hidden="true" className={open ? 'rotate-180' : ''} />}
      </button>
    ) : line.action === 'connect' && onConnect ? (
      <button type="button" onClick={onConnect} data-dv-status-action="connect" className={`act kw-dv-status-act ${focusRing}`}>
        Connect GitHub
      </button>
    ) : null;
  return (
    <div className="relative" data-dv-status-wrap>
      <div role="status" className="dv-statusline kw-dv-statusline" data-dv-status={line.status}>
        <p className="txt" data-dv-status-text>
          <span key={checkedAt ?? 0} className="dv-led" data-s={led} data-ping={ping || undefined} data-dv-led={led} aria-hidden="true" />
          <span>
            {line.parts.map((p, i) =>
              p.mono ? (
                // The branch follows " on "; the repo and host read as names.
                /\bon $/.test(line.parts[i - 1]?.text ?? '') ? (
                  <code key={i} className="dv-code">
                    {p.text}
                  </code>
                ) : (
                  <b key={i} className="font-mono text-[12px] break-all">
                    {p.text}
                  </b>
                )
              ) : (
                <React.Fragment key={i}>{p.text}</React.Fragment>
              ),
            )}
          </span>
        </p>
        {action}
        {checked && (
          <span className="checked" data-dev-checked>
            Checked {checked}
          </span>
        )}
      </div>
      {open && (
        <SourcePicker
          spaceId={spaceId}
          state={state}
          save={save}
          focusInput={open === 'type'}
          onClose={(refocus) => {
            setOpen(false);
            if (refocus) switchBtn.current?.focus();
          }}
        />
      )}
    </div>
  );
};

// ---------------------------------------------------------------------------
// Switch: a one-tap list of the detected repos, and a typed repo below it.

const SourcePicker: React.FC<{ spaceId: string; state: DevModeState; save: typeof devApi.putRepo; focusInput?: boolean; onClose: (refocus: boolean) => void }> = ({
  spaceId,
  state,
  save,
  focusInput = false,
  onClose,
}) => {
  const id = useId();
  const box = useRef<HTMLDivElement>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [typed, setTyped] = useState('');
  const current = watchedSource(state)?.repo ?? state.repo;

  useEffect(() => {
    if (focusInput) box.current?.querySelector<HTMLInputElement>('input')?.focus();
    else box.current?.querySelector<HTMLButtonElement>('[aria-checked="true"]')?.focus() ?? box.current?.querySelector<HTMLButtonElement>('button')?.focus();
    const away = (e: MouseEvent) => {
      if (box.current && !box.current.contains(e.target as Node) && !(e.target as Element).closest?.('[data-dv-status-action]')) onClose(false);
    };
    document.addEventListener('mousedown', away);
    return () => document.removeEventListener('mousedown', away);
  }, [onClose]);

  const pick = (repo: string) => {
    if (busy) return;
    if (repo === current) return onClose(true);
    setBusy(repo);
    setError(null);
    save(spaceId, { on: true, repo }).then(
      ({ state: s, refused }) => {
        devStore.set(spaceId, s);
        // Dev mode is on either way; a repo it can't read keeps the list open with why (Juno's check).
        if (refused) {
          setBusy(null);
          setError(refused);
          return;
        }
        onClose(true);
      },
      (e: unknown) => {
        setBusy(null);
        setError(e instanceof Error ? e.message : 'Couldn’t switch the repo.');
      },
    );
  };

  const typedOk = typed.trim() ? typedRepo(typed) : null;
  const typedErr = typed.trim() && !typedOk ? 'Paste a repo URL, or owner/name for GitHub.' : null;

  return (
    <div
      ref={box}
      role="dialog"
      aria-label="Which repo to watch"
      data-dv-picker
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          e.stopPropagation();
          onClose(true);
        }
      }}
      className="kw-dv-picker"
    >
      {state.sources.length > 0 && (
      <p id={`${id}-h`} className="px-1 pb-1.5 text-2xs font-semibold uppercase tracking-wider text-zinc-400">
        Found in this space
      </p>
      )}
      {state.sources.length > 0 && (
      <div role="radiogroup" aria-labelledby={`${id}-h`} className="flex flex-col gap-1">
        {state.sources.map((s) => {
          const on = s.repo === current;
          return (
            <button
              key={s.repo}
              type="button"
              role="radio"
              aria-checked={on}
              disabled={!!busy}
              onClick={() => pick(s.repo)}
              data-dv-pick={s.repo}
              className={`kw-dv-pick ${focusRing}`}
            >
              <span className="kw-dv-pick-icon" aria-hidden="true">
                {s.from === 'repo' ? <Link2 size={13} /> : <FolderGit2 size={13} />}
              </span>
              <span className="min-w-0 flex-1 text-left">
                <span className="block font-mono text-[12px] text-zinc-100 truncate">{s.repo}</span>
                <span className="block text-2xs text-zinc-400 truncate">{sourceWhere(s)}</span>
              </span>
              {busy === s.repo ? (
                <Loader2 size={13} className="shrink-0 animate-spin motion-reduce:animate-none text-zinc-300" aria-hidden="true" />
              ) : on ? (
                <Check size={14} className="shrink-0 text-sky-300" aria-hidden="true" />
              ) : null}
            </button>
          );
        })}
      </div>
      )}
      <form
        className={state.sources.length ? 'mt-2 border-t border-white/10 pt-2.5 px-1' : 'px-1'}
        onSubmit={(e) => {
          e.preventDefault();
          if (typedOk) pick(typedOk);
        }}
      >
        <label htmlFor={`${id}-typed`} className="block text-2xs font-medium text-zinc-400 mb-1">
          Type a repo
        </label>
        <div className="flex gap-1.5">
          <input
            id={`${id}-typed`}
            value={typed}
            onChange={(e) => {
              setTyped(e.target.value);
              setError(null);
            }}
            placeholder="owner/repo or a git URL"
            spellCheck={false}
            autoCapitalize="off"
            autoComplete="off"
            aria-invalid={!!typedErr || undefined}
            aria-describedby={`${id}-err`}
            className="kw-dv-picker-input"
          />
          <button type="submit" disabled={!typedOk || !!busy} className={`kw-dv-status-btn kw-dv-status-btn-primary ${focusRing}`}>
            Watch
          </button>
        </div>
        <p id={`${id}-err`} className="mt-1 min-h-[1em] text-2xs text-rose-300" aria-live="polite">
          {error ?? typedErr}
        </p>
      </form>
    </div>
  );
};
