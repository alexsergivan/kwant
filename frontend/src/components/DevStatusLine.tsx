// The Dev desk's status (0.7.2, team repos). The repo belongs to the team, not the space: Dev mode
// watches every team's repo. One repo reads as one line, as in 0.7.1. Several repos are a list, one row
// per repo (team · owner/name · branch · status), each with at most one action: Connect GitHub, Link a
// repo (a team with none), or Remove (a repo added by hand). More than three rows fold behind
// "Watching N repos". Nothing here blocks anything: the production floor and the command guards are on
// in every status.

import React, { useEffect, useId, useRef, useState } from 'react';
import { ChevronDown } from 'lucide-react';
import { DevModeState, DevTeamRef, RepoRow, RowAction, StatusLine, deskIntent, devApi, devStore, statusLine, typedRepo } from '../devMode';

const focusRing = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400';

/** style-v2 §4: the LED mirrors the status. ok only while a remote repo is really watched. */
export const statusLed = (line: StatusLine): 'ok' | 'warn' | 'off' => line.led;

const ACTION_TEXT: Record<RowAction, string> = { connect: 'Connect GitHub', link: 'Link a repo', remove: 'Remove' };

export interface DevStatusLineProps {
  spaceId: string;
  state: DevModeState | null;
  /** The space's teams, so a team with no repo gets "Link a repo". */
  teams?: DevTeamRef[];
  /** Opens Loose ends (Connect GitHub). Connect hides without it. */
  onConnect?: () => void;
  /** "Checked 19:58" at the right (style-v2 §4); the LED pings once each time it changes. */
  checked?: string | null;
  checkedAt?: number | null;
  /** Test hooks. */
  save?: typeof devApi.linkRepo;
  remove?: typeof devApi.removeRepo;
}

/** Where the Link form is open: a team row (`team:{id}`), or the single line (`line`). */
type Linking = string | null;

export const DevStatusLine: React.FC<DevStatusLineProps> = ({ spaceId, state, teams = [], onConnect, checked = null, checkedAt = null, save = devApi.linkRepo, remove = devApi.removeRepo }) => {
  const line = statusLine(state, teams);
  const listId = useId();
  // The toast's "Type a repo" lands on the first Link form, open and focused.
  const [linking, setLinking] = useState<Linking>(() => (deskIntent.take(spaceId) ? 'intent' : null));
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<{ id: string; text: string } | null>(null);
  const [expanded, setExpanded] = useState(false);
  const wrap = useRef<HTMLDivElement>(null);
  // One 200 ms ping when a check lands, not on first paint.
  const firstCheck = useRef(checkedAt);
  const ping = checkedAt != null && checkedAt !== firstCheck.current;
  if (!line || !state) return null;

  const rows = line.rows;
  // The intent opens on the first team with no repo, or on the single line.
  const linkingId = linking === 'intent' ? rows.find((r) => r.action === 'link')?.id ?? 'line' : linking;
  const open = !line.collapsed || expanded || (linkingId !== null && linkingId !== 'line');
  const single = rows.length === 0;
  const singleRow: RepoRow | null = single && state.repos.length === 1 ? { id: state.repos[0].key, team: state.repos[0].team, teamId: null, repo: state.repos[0], state: '', led: line.led, action: line.action } : null;
  // The single line's Link: the one team with none, if there is exactly one; else a repo added by hand.
  const loneTeam = single && line.action === 'link' ? teams.find((t) => !state.repos.some((r) => r.team === t.name)) ?? null : null;

  const focusAfter = (id: string) => {
    requestAnimationFrame(() => {
      const el = [...(wrap.current?.querySelectorAll<HTMLElement>('[data-dv-row-action], [data-dv-status-action]') ?? [])].find((b) => b.dataset.dvRowAction === id || (id === 'line' && b.dataset.dvStatusAction));
      el?.focus();
    });
  };

  const doRemove = (row: RepoRow) => {
    if (!row.repo || busy) return;
    setBusy(row.id);
    setError(null);
    remove(spaceId, row.repo.key).then(
      (s) => {
        devStore.set(spaceId, s);
        setBusy(null);
        requestAnimationFrame(() => wrap.current?.querySelector<HTMLElement>('[data-dv-row-action], [data-dv-status-action], [data-dv-disclosure]')?.focus());
      },
      (e: unknown) => {
        setBusy(null);
        setError({ id: row.id, text: e instanceof Error ? e.message : 'Couldn’t remove the repo.' });
      },
    );
  };

  const act = (row: RepoRow | null, action: RowAction, id: string) => {
    if (action === 'connect') return onConnect?.();
    if (action === 'link') {
      setError(null);
      return setLinking((l) => (l === id || (l === 'intent' && linkingId === id) ? null : id));
    }
    if (row) doRemove(row);
  };

  const actionButton = (row: RepoRow | null, action: RowAction | null, id: string, label: string, isLine: boolean) => {
    if (!action || (action === 'connect' && !onConnect)) return null;
    const expandedAttr = action === 'link' ? linkingId === id : undefined;
    return (
      <button
        type="button"
        onClick={() => act(row, action, id)}
        disabled={busy === id}
        aria-expanded={expandedAttr}
        aria-label={action === 'remove' ? `Remove ${label}` : action === 'link' && row?.team ? `Link a repo for ${row.team}` : undefined}
        {...(isLine ? { 'data-dv-status-action': action } : { 'data-dv-row-action': id, 'data-action': action })}
        className={`act kw-dv-status-act ${focusRing}`}
      >
        {busy === id && action === 'remove' ? 'Removing…' : ACTION_TEXT[action]}
      </button>
    );
  };

  const linkForm = (id: string, team: DevTeamRef | null, after: string) => (
    <LinkForm
      key={id}
      team={team}
      onCancel={() => {
        setLinking(null);
        focusAfter(after);
      }}
      onSubmit={async (repo) => {
        const { state: s, refused } = await save(spaceId, { repo, team: team?.id ?? null });
        devStore.set(spaceId, s);
        if (refused) return refused;
        setLinking(null);
        return null;
      }}
    />
  );

  return (
    <div ref={wrap} className="relative" data-dv-status-wrap>
      <div role="status" className="dv-statusline kw-dv-statusline" data-dv-status={line.status} data-dv-repos={state.repos.length}>
        <p className="txt" data-dv-status-text>
          <span key={checkedAt ?? 0} className="dv-led" data-s={line.led} data-ping={ping || undefined} data-dv-led={line.led} aria-hidden="true" />
          {line.collapsed ? (
            <button
              type="button"
              aria-expanded={open}
              aria-controls={listId}
              onClick={() => {
                if (open) setLinking(null);
                setExpanded(!open);
              }}
              data-dv-disclosure
              className={`kw-dv-disclosure ${focusRing}`}
            >
              <span>{line.parts.map((p) => p.text).join('')}</span>
              <ChevronDown size={13} aria-hidden="true" className={open ? 'rotate-180' : ''} />
            </button>
          ) : (
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
          )}
        </p>
        {single && actionButton(singleRow, line.action, singleRow?.id ?? 'line', singleRow?.repo?.repo ?? '', true)}
        {checked && (
          <span className="checked" data-dev-checked>
            Checked {checked}
          </span>
        )}
      </div>
      {single && linkingId === 'line' && line.action === 'link' && <div className="kw-dv-repo-form-wrap">{linkForm('line', loneTeam, 'line')}</div>}
      {single && error && <p className="kw-dv-repo-err" role="alert">{error.text}</p>}
      {!single && open && (
        <ul id={listId} className="kw-dv-repos" aria-label="Repos Dev mode watches" data-dv-repos-list>
          {rows.map((r) => (
            <li key={r.id} className="kw-dv-repo" data-dv-repo-row={r.id} data-s={r.repo?.status ?? 'no_repo'}>
              <div className="kw-dv-repo-line">
                <span className="dv-led" data-s={r.led} aria-hidden="true" />
                <span className="kw-dv-repo-cells">
                  <span className="kw-dv-repo-team">{r.team ?? 'Added by hand'}</span>
                  {r.repo ? (
                    <>
                      <span className="kw-dv-repo-sep" aria-hidden="true">·</span>
                      <b className="kw-dv-repo-name">{r.repo.repo}</b>
                      {r.repo.branch && r.repo.status !== 'other_host' && r.repo.status !== 'too_many' && (
                        <>
                          <span className="kw-dv-repo-sep" aria-hidden="true">·</span>
                          <code className="dv-code">{r.repo.branch}</code>
                        </>
                      )}
                    </>
                  ) : null}
                  <span className="kw-dv-repo-sep" aria-hidden="true">·</span>
                  <span className="kw-dv-repo-state" data-dv-repo-state>
                    {r.state}
                  </span>
                </span>
                {actionButton(r, r.action, r.id, r.repo?.repo ?? r.team ?? '', false)}
              </div>
              {r.action === 'link' && linkingId === r.id && linkForm(r.id, r.teamId && r.team ? { id: r.teamId, name: r.team } : null, r.id)}
              {error?.id === r.id && <p className="kw-dv-repo-err" role="alert">{error.text}</p>}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
};

// ---------------------------------------------------------------------------
// Link a repo: one field. With a team it becomes that team's repo link (PUT {repo, team}); without one
// it is added by hand (PUT {repo}).

const LinkForm: React.FC<{ team: DevTeamRef | null; onSubmit: (repo: string) => Promise<string | null>; onCancel: () => void }> = ({ team, onSubmit, onCancel }) => {
  const id = useId();
  const input = useRef<HTMLInputElement>(null);
  const [typed, setTyped] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => input.current?.focus(), []);
  const ok = typed.trim() ? typedRepo(typed) : null;
  const typedErr = typed.trim() && !ok ? 'Paste a repo URL, or owner/name for GitHub.' : null;
  return (
    <form
      className="kw-dv-repo-form"
      data-dv-link-form={team?.id ?? 'hand'}
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          e.stopPropagation();
          onCancel();
        }
      }}
      onSubmit={(e) => {
        e.preventDefault();
        if (!ok || busy) return;
        setBusy(true);
        setError(null);
        onSubmit(ok).then(
          (refused) => {
            setBusy(false);
            if (refused) setError(refused);
          },
          (err: unknown) => {
            setBusy(false);
            setError(err instanceof Error ? err.message : 'Couldn’t link the repo.');
          },
        );
      }}
    >
      <label htmlFor={`${id}-repo`} className="kw-dv-repo-form-label">
        {team ? `Repo for ${team.name}` : 'Repo to watch'}
      </label>
      <div className="kw-dv-repo-form-row">
        <input
          ref={input}
          id={`${id}-repo`}
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
        <button type="submit" disabled={!ok || busy} className={`kw-dv-status-btn kw-dv-status-btn-primary ${focusRing}`}>
          {busy ? 'Linking…' : 'Link'}
        </button>
        <button type="button" onClick={onCancel} className={`kw-dv-status-btn ${focusRing}`}>
          Cancel
        </button>
      </div>
      <p id={`${id}-err`} className="kw-dv-repo-form-err" aria-live="polite">
        {error ?? typedErr}
      </p>
    </form>
  );
};
