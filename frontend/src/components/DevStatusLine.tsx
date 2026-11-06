// The Dev desk's status (0.7.2, team repos; copy: dev-mode-explainer/spec.md §6 and §9b). The repo belongs
// to the team, not the space: Dev mode watches every team's repo. One repo reads as one line, as in 0.7.1.
// Several repos are a list, one row per repo (team · owner/name · branch · status, then where it comes
// from), each with at most one action: Connect GitHub, Link a repo (a team with none), or Remove (a repo
// added by you). A team's repo has no Remove: an info note says where to change it. More than three rows
// fold behind "Watching N repos" with Show repos / Hide repos. Teams with no repo get rows only while no
// repo is watched; otherwise they are one quiet last line. Nothing here blocks anything: the production
// floor and the command guards are on in every status.

import React, { useEffect, useId, useRef, useState } from 'react';
import { ChevronDown, Info } from 'lucide-react';
import {
  DevModeError,
  DevModeState,
  DevTeamRef,
  LINK_ERROR,
  LINK_HELP,
  LINK_HELP_HAND,
  LINK_PLACEHOLDER,
  READY,
  READY_TITLE,
  UNDO_TOAST_MS,
  clearedText,
  RemovedRepo,
  RepoRow,
  RowAction,
  StatusLine,
  deskIntent,
  devApi,
  devStore,
  noRepoTeamsText,
  removedText,
  statusLine,
  teamNoteText,
  typedRepo,
  undoBody,
} from '../devMode';

const focusRing = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400';

/** style-v2 §4: the LED mirrors the status. ok only while a remote repo is really watched. */
export const statusLed = (line: StatusLine): StatusLine['led'] => line.led;

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

/** Where the Link form is open: a team row (`team:{id}`), the quiet no-repo line (`norepo`), or the single line (`line`). */
type Linking = string | null;

/** A refused or unknown repo reads as Mira's one error; anything else keeps the server's plain sentence. */
const linkError = (e: unknown): string => {
  if (e instanceof DevModeError && e.code && e.code !== 'bad_repo' && e.code !== 'no_branch') return e.message;
  if (e instanceof DevModeError && !e.code && e.status === 0) return e.message;
  return LINK_ERROR;
};

export const DevStatusLine: React.FC<DevStatusLineProps> = ({ spaceId, state, teams = [], onConnect, checked = null, checkedAt = null, save = devApi.linkRepo, remove = devApi.removeRepo }) => {
  const line = statusLine(state, teams);
  const listId = useId();
  // The header's "Link a repo" / the toast's "Type a repo" open the first Link form, focused; "See repos"
  // opens the list and scrolls to it.
  const [intent] = useState(() => deskIntent.take(spaceId));
  const [linking, setLinking] = useState<Linking>(() => (intent === 'type' ? 'intent' : null));
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<{ id: string; text: string } | null>(null);
  const [expanded, setExpanded] = useState(intent === 'repos');
  const [note, setNote] = useState<string | null>(null);
  const [removed, setRemoved] = useState<(RemovedRepo & { busy: boolean; error: string | null }) | null>(null);
  const wrap = useRef<HTMLDivElement>(null);
  // One 200 ms ping when a check lands, not on first paint.
  const firstCheck = useRef(checkedAt);
  const ping = checkedAt != null && checkedAt !== firstCheck.current;
  useEffect(() => {
    if (intent === 'repos') requestAnimationFrame(() => wrap.current?.scrollIntoView?.({ block: 'start' }));
  }, [intent]);
  // The Remove / Clear toast leaves after UNDO_TOAST_MS unless the person is on it.
  useEffect(() => {
    if (!removed || removed.busy || removed.error) return;
    const t = setTimeout(() => setRemoved(null), UNDO_TOAST_MS);
    return () => clearTimeout(t);
  }, [removed]);
  if (!line || !state) return null;

  const rows = line.rows;
  const noRepo = line.noRepoTeams;
  // The intent opens on the first team with no repo (a row, or the quiet line), or on the single line.
  const linkingId = linking === 'intent' ? rows.find((r) => r.action === 'link')?.id ?? (noRepo.length ? 'norepo' : 'line') : linking;
  const open = !line.collapsed || expanded || (linkingId !== null && linkingId !== 'line' && linkingId !== 'norepo');
  const single = rows.length === 0;
  const singleRow: RepoRow | null =
    single && state.repos.length === 1
      ? { id: state.repos[0].key, team: state.repos[0].team, teamId: null, repo: state.repos[0], state: '', origin: null, teamNote: null, title: null, led: line.led, action: line.action }
      : null;
  // The single line's Link: the one team with none, if there is exactly one; else a repo added by hand.
  const loneTeam = single && line.action === 'link' ? teams.find((t) => !state.repos.some((r) => r.team === t.name)) ?? null : null;

  const focusAfter = (id: string) => {
    requestAnimationFrame(() => {
      const el = [...(wrap.current?.querySelectorAll<HTMLElement>('[data-dv-row-action], [data-dv-status-action], [data-dv-norepo-action]') ?? [])].find(
        (b) => b.dataset.dvRowAction === id || (id === 'line' && b.dataset.dvStatusAction) || (id === 'norepo' && b.dataset.dvNorepoAction !== undefined),
      );
      el?.focus();
    });
  };

  const doRemove = (row: RepoRow) => {
    if (!row.repo || busy) return;
    const repo = row.repo;
    setBusy(row.id);
    setError(null);
    remove(spaceId, repo.key).then(
      (s) => {
        devStore.set(spaceId, s);
        setBusy(null);
        // A team's repo link comes back as that team's link; a hand row as a hand row (answer 5).
        const team = repo.added_by === 'team' && repo.from === 'repo' ? row.teamId : null;
        setRemoved({ repo: repo.repo, team, teamName: team ? row.team : null, busy: false, error: null });
        requestAnimationFrame(() => wrap.current?.querySelector<HTMLElement>('[data-dv-undo]')?.focus());
      },
      (e: unknown) => {
        setBusy(null);
        // 409 team_repo: the row comes from a team; say where to change it (spec §6).
        const text = e instanceof DevModeError && e.code === 'team_repo' ? teamNoteText(row.team ?? 'this team', repo.from) : e instanceof Error ? e.message : 'Couldn’t remove the repo.';
        setError({ id: row.id, text });
      },
    );
  };

  const undo = () => {
    if (!removed || removed.busy) return;
    setRemoved({ ...removed, busy: true, error: null });
    save(spaceId, undoBody(removed)).then(
      ({ state: s, refused }) => {
        devStore.set(spaceId, s);
        setRemoved(refused ? { ...removed, busy: false, error: refused } : null);
      },
      (e: unknown) => setRemoved({ ...removed, busy: false, error: e instanceof Error ? e.message : 'Couldn’t add it back.' }),
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
        aria-label={action === 'remove' ? `Stop watching ${label}` : action === 'link' && row?.team ? `Link a repo for ${row.team}` : undefined}
        {...(isLine ? { 'data-dv-status-action': action } : { 'data-dv-row-action': id, 'data-action': action })}
        className={`act kw-dv-status-act ${focusRing}`}
      >
        {busy === id && action === 'remove' ? 'Removing…' : ACTION_TEXT[action]}
      </button>
    );
  };

  const linkForm = (id: string, team: DevTeamRef | null, after: string, pick?: DevTeamRef[]) => (
    <LinkForm
      key={id}
      team={team}
      pick={pick}
      onCancel={() => {
        setLinking(null);
        focusAfter(after);
      }}
      onSubmit={async (repo, t) => {
        const { state: s, refused } = await save(spaceId, { repo, team: t?.id ?? null });
        devStore.set(spaceId, s);
        if (refused) return LINK_ERROR;
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
          <span className="min-w-0">
            {line.parts.map((p, i) =>
              p.branch ? (
                <code key={i} className="dv-code">
                  {p.text}
                </code>
              ) : p.mono ? (
                <b key={i} className="font-mono text-[12px] break-all">
                  {p.text}
                </b>
              ) : (
                <React.Fragment key={i}>{p.text}</React.Fragment>
              ),
            )}
          </span>
          {line.collapsed && (
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
              <span>{open ? 'Hide repos' : 'Show repos'}</span>
              <ChevronDown size={13} aria-hidden="true" className={open ? 'rotate-180' : ''} />
            </button>
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
      {single && error && (
        <p className="kw-dv-repo-err" role="alert">
          {error.text}
        </p>
      )}
      {!single && open && (
        <ul id={listId} className="kw-dv-repos" aria-label="Repos Dev mode watches" data-dv-repos-list>
          {rows.map((r) => (
            <li key={r.id} className="kw-dv-repo" data-dv-repo-row={r.id} data-s={r.repo?.status ?? 'no_repo'}>
              <div className="kw-dv-repo-line">
                <span className="dv-led" data-s={r.led} aria-hidden="true" />
                <span className="kw-dv-repo-cells">
                  {r.team && <span className="kw-dv-repo-team">{r.team}</span>}
                  {r.repo ? (
                    <>
                      {r.team && (
                        <span className="kw-dv-repo-sep" aria-hidden="true">
                          ·
                        </span>
                      )}
                      <b className="kw-dv-repo-name">{r.repo.repo}</b>
                      {r.repo.branch && r.repo.status !== 'other_host' && r.repo.status !== 'too_many' && (
                        <>
                          <span className="kw-dv-repo-sep" aria-hidden="true">
                            ·
                          </span>
                          <code className="dv-code">{r.repo.branch}</code>
                        </>
                      )}
                    </>
                  ) : null}
                  <span className="kw-dv-repo-sep" aria-hidden="true">
                    ·
                  </span>
                  <span
                    className="kw-dv-repo-state"
                    data-dv-repo-state
                    data-tone={r.state === READY ? 'neutral' : undefined}
                    title={r.state === READY ? READY_TITLE : r.title ?? undefined}
                    aria-label={r.state === READY ? READY_TITLE : undefined}
                  >
                    {r.state}
                  </span>
                </span>
                {r.teamNote && (
                  <button
                    type="button"
                    aria-label={r.teamNote}
                    aria-expanded={note === r.id}
                    title={r.teamNote}
                    onClick={() => setNote((n) => (n === r.id ? null : r.id))}
                    data-dv-team-note={r.id}
                    className={`kw-dv-info ${focusRing}`}
                  >
                    <Info size={14} aria-hidden="true" />
                  </button>
                )}
                {actionButton(r, r.action, r.id, r.repo?.repo ?? r.team ?? '', false)}
              </div>
              {r.origin && (
                <p className="kw-dv-repo-origin" data-dv-repo-origin>
                  {r.origin}
                </p>
              )}
              {note === r.id && r.teamNote && (
                <p className="kw-dv-repo-origin" data-dv-team-note-text>
                  {r.teamNote}
                </p>
              )}
              {r.action === 'link' && linkingId === r.id && linkForm(r.id, r.teamId && r.team ? { id: r.teamId, name: r.team } : null, r.id)}
              {error?.id === r.id && (
                <p className="kw-dv-repo-err" role="alert">
                  {error.text}
                </p>
              )}
            </li>
          ))}
        </ul>
      )}
      {noRepo.length > 0 && (
        <div className="kw-dv-norepo" data-dv-norepo={noRepo.length}>
          <p className="kw-dv-norepo-line">
            <span>{noRepoTeamsText(noRepo.length)}</span>
            <span className="kw-dv-repo-sep" aria-hidden="true">
              ·
            </span>
            <button
              type="button"
              aria-expanded={linkingId === 'norepo'}
              onClick={() => setLinking((l) => (l === 'norepo' || (l === 'intent' && linkingId === 'norepo') ? null : 'norepo'))}
              data-dv-norepo-action
              className={`kw-dv-norepo-act ${focusRing}`}
            >
              Link a repo
            </button>
          </p>
          {linkingId === 'norepo' && linkForm('norepo', noRepo.length === 1 ? noRepo[0] : null, 'norepo', noRepo.length > 1 ? noRepo : undefined)}
        </div>
      )}
      {removed && (
        <div role="status" className="kw-dv-removed" data-dv-removed>
          <span>{removed.error ?? (removed.team ? clearedText(removed.teamName || 'the team') : removedText(removed.repo))}</span>
          {!removed.error && (
            <button type="button" onClick={undo} disabled={removed.busy} data-dv-undo className={`kw-dv-norepo-act ${focusRing}`}>
              {removed.busy ? 'Adding back…' : 'Undo'}
            </button>
          )}
        </div>
      )}
    </div>
  );
};

// ---------------------------------------------------------------------------
// Link a repo (spec §6): one field. With a team it becomes that team's repo link (PUT {repo, team});
// with several teams to choose from (the quiet "N teams have no repo" line) a team picker comes first;
// with none it is added by hand (PUT {repo}).

const LinkForm: React.FC<{ team: DevTeamRef | null; pick?: DevTeamRef[]; onSubmit: (repo: string, team: DevTeamRef | null) => Promise<string | null>; onCancel: () => void }> = ({
  team,
  pick,
  onSubmit,
  onCancel,
}) => {
  const id = useId();
  const first = useRef<HTMLSelectElement & HTMLInputElement>(null);
  const [picked, setPicked] = useState<DevTeamRef | null>(team ?? pick?.[0] ?? null);
  const [typed, setTyped] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => first.current?.focus(), []);
  const ok = typed.trim() ? typedRepo(typed) : null;
  const typedErr = typed.trim() && !ok ? LINK_ERROR : null;
  const shown = error ?? typedErr;
  return (
    <form
      className="kw-dv-repo-form"
      data-dv-link-form={picked?.id ?? 'hand'}
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
        onSubmit(ok, picked).then(
          (refused) => {
            setBusy(false);
            if (refused) setError(refused);
          },
          (err: unknown) => {
            setBusy(false);
            setError(linkError(err));
          },
        );
      }}
    >
      {pick && pick.length > 1 && (
        <div className="kw-dv-repo-form-pick">
          <label htmlFor={`${id}-team`} className="kw-dv-repo-form-label">
            Team
          </label>
          <select
            ref={first}
            id={`${id}-team`}
            value={picked?.id ?? ''}
            onChange={(e) => setPicked(pick.find((t) => t.id === e.target.value) ?? null)}
            className="kw-dv-picker-input kw-dv-team-pick"
            data-dv-team-pick
          >
            {pick.map((t) => (
              <option key={t.id} value={t.id}>
                {t.name}
              </option>
            ))}
          </select>
        </div>
      )}
      <label htmlFor={`${id}-repo`} className="kw-dv-repo-form-label">
        {picked ? `Repo for ${picked.name}` : 'Repo to watch'}
      </label>
      <div className="kw-dv-repo-form-row">
        <input
          ref={pick && pick.length > 1 ? undefined : first}
          id={`${id}-repo`}
          value={typed}
          onChange={(e) => {
            setTyped(e.target.value);
            setError(null);
          }}
          placeholder={LINK_PLACEHOLDER}
          spellCheck={false}
          autoCapitalize="off"
          autoComplete="off"
          aria-invalid={!!shown || undefined}
          aria-describedby={`${id}-help ${id}-err`}
          className="kw-dv-picker-input"
        />
        <button type="submit" disabled={!ok || busy} className={`kw-dv-status-btn kw-dv-status-btn-primary ${focusRing}`}>
          {busy ? 'Saving…' : 'Save'}
        </button>
        <button type="button" onClick={onCancel} className={`kw-dv-status-btn ${focusRing}`}>
          Cancel
        </button>
      </div>
      <p id={`${id}-help`} className="kw-dv-repo-form-help">
        {picked ? LINK_HELP : LINK_HELP_HAND}
      </p>
      <p id={`${id}-err`} className="kw-dv-repo-form-err" aria-live="polite">
        {shown}
      </p>
    </form>
  );
};
