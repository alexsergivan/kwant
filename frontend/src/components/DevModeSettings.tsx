// Settings › This space › Dev mode: the repo, the branch to watch, the optional backup command
// (plan D9) and Turn off. Off keeps the settings and What landed's history.
// Dev mode visibility (autonomy/dev-mode-visibility, design.md §1): the card is first in General. Off sells
// (what you gain and one primary Turn on; an accent card naming the found repo, its fields behind Details),
// On configures (today's form).

import React, { useEffect, useRef, useState } from 'react';
import { Check, ChevronDown, Code, Loader2 } from 'lucide-react';
import { DevModeState, DevTeamRef, devApi, devStore, normaliseRepo } from '../devMode';
import { foundRepo, hasFoundCode } from '../devSuggest';
import { JiraSettings } from './JiraSettings';
import { useDevMode } from './DevModeSwitch';

const card = 'rounded-xl border border-zinc-800 bg-zinc-950/60';
/** Off with a repo found: the accent family of the primary buttons (no shadow, no glow, no holo). */
const accentCard = 'rounded-xl border border-sky-500/40 bg-accent-soft';
const rowBtn =
  'flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-xs font-medium text-zinc-300 border border-zinc-700/60 bg-zinc-800/60 hover:bg-zinc-700/70 hover:text-zinc-100 transition-colors disabled:opacity-50';
const primaryBtn = 'flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 transition-colors disabled:opacity-50';
const turnOnBtn =
  'inline-flex items-center justify-center gap-1.5 h-9 max-md:h-11 max-md:w-full px-3.5 rounded-lg text-[13px] font-semibold text-white bg-sky-600 hover:bg-sky-500 transition-colors disabled:opacity-50 kw-focus';
const input = 'w-full bg-zinc-950 border rounded-lg px-3 py-2 text-sm font-mono text-zinc-100 focus:outline-none border-zinc-800 focus:border-sky-500';
/** What turning it on gives you, the same words in the card and the #general suggestion. */
export const DEV_GAIN = 'your bots review pull requests, show what landed and guard deploys.';

/** "Found owner/name in Team and 2 more. Turn it on and …", the repo in mono, the team in medium. */
const FoundLine: React.FC<{ state: DevModeState }> = ({ state }) => {
  const r = foundRepo(state);
  if (!r) return null;
  const more = state.repos.length - 1;
  return (
    <>
      Found <span className="font-mono text-zinc-100">{r.repo}</span>
      {r.team ? (
        <>
          {' '}in <span className="font-medium text-zinc-100">{r.team}</span>
        </>
      ) : null}
      {more > 0 ? ` and ${more} more` : ''}. Turn it on and {DEV_GAIN}
    </>
  );
};

export const DevModeSettings: React.FC<{ spaceId: string; spaceName: string; approvals?: string; save?: typeof devApi.put; teams?: DevTeamRef[] }> = ({ spaceId, spaceName, save = devApi.put, teams }) => {
  const state = useDevMode(spaceId);
  const [repo, setRepo] = useState('');
  const [branch, setBranch] = useState('');
  const [backup, setBackup] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [details, setDetails] = useState(false);
  const [emptyErr, setEmptyErr] = useState(false);
  const repoInput = useRef<HTMLInputElement>(null);

  // Off with a found GitHub repo: the fields start from it, so Details shows what Turn on will watch.
  const found = hasFoundCode(state) ? foundRepo(state) : null;
  const prefill = found ? normaliseRepo(found.repo) : null;
  const baseRepo = state?.repo ?? prefill ?? '';
  const baseBranch = state?.branch ?? (prefill ? found?.branch ?? '' : '');

  useEffect(() => {
    if (!state) devApi.get(spaceId).then((s) => devStore.set(spaceId, s), (e: unknown) => setError(e instanceof Error ? e.message : 'Dev mode couldn’t load.'));
  }, [spaceId, state]);
  useEffect(() => {
    setRepo(baseRepo);
    setBranch(baseBranch);
    setBackup(state?.backup_command ?? '');
  }, [baseRepo, baseBranch, state?.backup_command]);

  // 0.9.8 (first-screen A): the header switch is gone, so this card is where Dev mode is turned on, the
  // first time too. It shows as soon as the space's state has loaded.
  if (!state) return null;

  const normal = repo.trim() ? normaliseRepo(repo) : null;
  const repoErr = repo.trim() && !normal ? 'Paste a GitHub repo, like owner/name or its URL.' : null;
  const dirty = (normal ?? '') !== baseRepo || branch.trim() !== baseBranch || backup.trim() !== (state.backup_command ?? '');
  const accent = !!found;

  const put = (body: Partial<Pick<DevModeState, 'repo' | 'branch' | 'backup_command'>> & { on: boolean }) => {
    setBusy(true);
    setError(null);
    setSaved(false);
    save(spaceId, body).then(
      (s) => {
        devStore.set(spaceId, s);
        setSaved(true);
        setBusy(false);
        setDetails(false);
      },
      (e: unknown) => {
        setError(e instanceof Error ? e.message : 'Couldn’t save Dev mode.');
        setBusy(false);
      },
    );
  };

  // Turn on sends only what the person changed: an untouched prefill isn't sent, or it would add a hand
  // row next to the team's own (the server watches the team repos it found by itself).
  const turnOn = () => {
    if (repoErr) {
      setDetails(true);
      setTimeout(() => repoInput.current?.focus(), 0);
      return;
    }
    if (!accent && !normal) {
      setEmptyErr(true);
      repoInput.current?.focus();
      return;
    }
    const body: Parameters<typeof put>[0] = { on: true };
    if ((normal ?? '') !== baseRepo) body.repo = normal;
    if (branch.trim() !== baseBranch) body.branch = branch.trim() || null;
    if (backup.trim() !== (state.backup_command ?? '')) body.backup_command = backup.trim();
    put(body);
  };
  const shownRepoErr = repoErr ?? (emptyErr && !state.on && !repo.trim() ? 'Add a repo first.' : null);

  const repoField = (
    <div>
      <label htmlFor="dv-set-repo" className="block text-xs font-medium text-zinc-300 mb-1">
        Repository
      </label>
      <input
        ref={repoInput}
        id="dv-set-repo"
        value={repo}
        onChange={(e) => {
          setRepo(e.target.value);
          setEmptyErr(false);
        }}
        placeholder={state.on ? 'owner/repo' : 'owner/name'}
        spellCheck={false}
        autoCapitalize="off"
        className={input}
        aria-invalid={!!shownRepoErr}
        aria-describedby={!state.on && !accent ? 'dv-set-repo-err dv-set-repo-help' : 'dv-set-repo-err'}
      />
      <p id="dv-set-repo-err" className="mt-1 text-[11px] text-rose-300 min-h-[1em]">
        {shownRepoErr}
      </p>
      {!state.on && !accent && (
        <p id="dv-set-repo-help" className="text-[11px] text-zinc-500">
          Paste owner/name or its URL. A repo set on a team (Team settings › Git Repo) shows up here by itself.
        </p>
      )}
    </div>
  );
  const branchField = (
    <div>
      <label htmlFor="dv-set-branch" className="block text-xs font-medium text-zinc-300 mb-1">
        Branch to watch
      </label>
      <input id="dv-set-branch" value={branch} onChange={(e) => setBranch(e.target.value)} placeholder="main" spellCheck={false} autoCapitalize="off" className={input} />
    </div>
  );
  const backupField = (
    <div>
      <label htmlFor="dv-set-backup" className="block text-xs font-medium text-zinc-300 mb-1">
        Backup command <span className="text-zinc-500 font-normal">(optional)</span>
      </label>
      <input
        id="dv-set-backup"
        value={backup}
        maxLength={2000}
        onChange={(e) => setBackup(e.target.value)}
        placeholder="./scripts/backup.sh"
        spellCheck={false}
        autoCapitalize="off"
        className={input}
        aria-describedby="dv-set-backup-help"
      />
      <p id="dv-set-backup-help" className="mt-1 text-[11px] text-zinc-500">
        Runs in the bot’s folder when you press Back up first on a production step. It should print the backup file’s path.
      </p>
    </div>
  );

  const errorLine = error && (
    <p role="alert" className="text-[11px] text-rose-300">
      {error}
    </p>
  );
  const tile = accent ? 'bg-sky-500/15 text-sky-300' : state.on ? 'bg-white/[.04] text-sky-300' : 'bg-white/[.04] text-zinc-400';

  return (
    <>
    <form
      className={`${accent ? accentCard : card} p-4 scroll-mt-4 focus:outline-none`}
      data-dev-settings
      data-dev-off={state.on ? undefined : accent ? 'found' : 'none'}
      // The #general suggestion lands here (focus goes to Turn on, or to the card itself when it's on).
      tabIndex={-1}
      aria-labelledby="dv-title"
      onSubmit={(e) => {
        e.preventDefault();
        if (!state.on) return turnOn();
        if (!dirty || repoErr || busy) return;
        put({ on: state.on, repo: normal, branch: branch.trim() || null, backup_command: backup.trim() });
      }}
    >
      <div className="flex items-center gap-2.5">
        <span className={`w-8 h-8 shrink-0 rounded-lg grid place-items-center ${tile}`}>
          <Code size={16} aria-hidden="true" />
        </span>
        <h3 id="dv-title" className="text-sm font-semibold text-zinc-100">
          Dev mode
        </h3>
        <span className={`ml-auto kw-chip ${state.on ? 'kw-chip-accent' : ''}`}>{state.on ? 'On' : 'Off'}</span>
      </div>
      {state.on ? (
        <div className="mt-3 space-y-3">
          <p id="dv-value" className="text-[11px] text-zinc-500">
            {spaceName} reads this repo to show what landed, review PRs and guard deploys. Only you can change it.
          </p>
          {repoField}
          {branchField}
          {backupField}
          {errorLine}
          <div className="flex items-center gap-2">
            <button type="button" className={rowBtn} disabled={busy} onClick={() => put({ on: false })} data-dev-turn-off>
              Turn off Dev mode
            </button>
            <span className="flex-1" />
            {saved && !dirty && (
              <span className="flex items-center gap-1 text-xs text-emerald-400" role="status">
                <Check size={13} aria-hidden="true" /> Saved
              </span>
            )}
            <button type="submit" disabled={!dirty || !!repoErr || busy} className={primaryBtn}>
              {busy ? 'Saving…' : 'Save'}
            </button>
          </div>
        </div>
      ) : (
        <>
          <p id="dv-value" className="mt-2 text-xs leading-5 text-zinc-300" data-dev-gain>
            {accent ? <FoundLine state={state} /> : `Link your project's repo and ${DEV_GAIN}`}
          </p>
          {!accent && <div className="mt-3">{repoField}</div>}
          <div className="mt-3 flex flex-wrap items-center gap-x-3 max-md:gap-y-1">
            <button type="submit" className={turnOnBtn} disabled={busy} aria-describedby="dv-value" data-dev-turn-on>
              {busy && <Loader2 size={13} aria-hidden="true" className="animate-spin motion-reduce:animate-none" />}
              {busy ? 'Turning on…' : 'Turn on Dev mode'}
            </button>
            <button
              type="button"
              className="inline-flex items-center gap-1 min-h-8 max-md:min-h-11 px-1 text-xs text-sky-300 hover:text-sky-200 kw-focus rounded"
              aria-expanded={details}
              aria-controls="dv-details"
              onClick={() => setDetails((d) => !d)}
              data-dev-details
            >
              Details
              <ChevronDown size={13} aria-hidden="true" className={`transition-transform duration-150 motion-reduce:transition-none ${details ? 'rotate-180' : ''}`} />
            </button>
            <span className="md:ml-auto max-md:w-full text-[11px] text-zinc-500">You can turn it off any time.</span>
          </div>
          {details && (
            <div id="dv-details" className="mt-3 pt-3 border-t border-white/[.08] space-y-3">
              {accent && repoField}
              {branchField}
              {backupField}
            </div>
          )}
          {error && <div className="mt-3">{errorLine}</div>}
        </>
      )}
    </form>
    {/* 0.7.6: "Work from Jira · Off · Set up" (or the On box); nothing on a server without the Jira routes. */}
    {state.on && <JiraSettings space={spaceId} teams={teams} />}
    </>
  );
};
