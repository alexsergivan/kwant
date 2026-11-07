// Settings › This space › Dev mode: the repo, the branch to watch, the optional backup command
// (plan D9) and Turn off. Off keeps the settings and What landed's history.

import React, { useEffect, useState } from 'react';
import { Check, Code } from 'lucide-react';
import { DevModeState, DevTeamRef, approvalsWord, devApi, devStore, normaliseRepo } from '../devMode';
import { JiraSettings } from './JiraSettings';
import { useDevMode } from './DevModeSwitch';

const card = 'rounded-xl border border-zinc-800 bg-zinc-950/60';
const rowBtn =
  'flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-xs font-medium text-zinc-300 border border-zinc-700/60 bg-zinc-800/60 hover:bg-zinc-700/70 hover:text-zinc-100 transition-colors disabled:opacity-50';
const primaryBtn = 'flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 transition-colors disabled:opacity-50';
const input = 'w-full bg-zinc-950 border rounded-lg px-3 py-2 text-sm font-mono text-zinc-100 focus:outline-none border-zinc-800 focus:border-sky-500';

export const DevModeSettings: React.FC<{ spaceId: string; spaceName: string; approvals?: string; save?: typeof devApi.put; teams?: DevTeamRef[] }> = ({ spaceId, spaceName, approvals, save = devApi.put, teams }) => {
  const state = useDevMode(spaceId);
  const [repo, setRepo] = useState('');
  const [branch, setBranch] = useState('');
  const [backup, setBackup] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    if (!state) devApi.get(spaceId).then((s) => devStore.set(spaceId, s), (e: unknown) => setError(e instanceof Error ? e.message : 'Dev mode couldn’t load.'));
  }, [spaceId, state]);
  useEffect(() => {
    setRepo(state?.repo ?? '');
    setBranch(state?.branch ?? '');
    setBackup(state?.backup_command ?? '');
  }, [state?.repo, state?.branch, state?.backup_command]);

  if (!state) return null;
  // Before the first switch-on there's nothing to set: the switch in the header opens the setup.
  if (!state.on && !state.last_seen_sha && !state.polled_at) return null;

  const normal = repo.trim() ? normaliseRepo(repo) : null;
  const repoErr = repo.trim() && !normal ? 'Paste a GitHub repo, like owner/name or its URL.' : null;
  const dirty = (normal ?? '') !== (state.repo ?? '') || branch.trim() !== (state.branch ?? '') || backup.trim() !== (state.backup_command ?? '');

  const put = (body: Partial<Pick<DevModeState, 'repo' | 'branch' | 'backup_command'>> & { on: boolean }) => {
    setBusy(true);
    setError(null);
    setSaved(false);
    save(spaceId, body).then(
      (s) => {
        devStore.set(spaceId, s);
        setSaved(true);
        setBusy(false);
      },
      (e: unknown) => {
        setError(e instanceof Error ? e.message : 'Couldn’t save Dev mode.');
        setBusy(false);
      },
    );
  };

  return (
    <>
    <form
      className={`${card} p-4 space-y-3`}
      data-dev-settings
      onSubmit={(e) => {
        e.preventDefault();
        if (!dirty || repoErr || busy) return;
        put({ on: state.on, repo: normal, branch: branch.trim() || null, backup_command: backup.trim() });
      }}
    >
      <p className="flex items-center gap-2 text-xs font-medium text-zinc-200">
        <Code size={14} className="text-zinc-400" aria-hidden="true" /> Dev mode
        <span className={`ml-auto kw-chip ${state.on ? 'kw-chip-accent' : ''}`}>{state.on ? 'On' : 'Off'}</span>
      </p>
      <p className="text-[11px] text-zinc-500">
        {state.on
          ? `${spaceName} reads this repo to show what landed, review PRs and guard deploys. Only you can change it.`
          : `Off. Reviews, the ship gate and the production floor are stopped; approvals stay on ${approvalsWord(approvals)}. Turn it on with the switch next to the space’s name.`}
      </p>
      <div>
        <label htmlFor="dv-set-repo" className="block text-xs font-medium text-zinc-300 mb-1">
          Repository
        </label>
        <input id="dv-set-repo" value={repo} onChange={(e) => setRepo(e.target.value)} placeholder="owner/repo" spellCheck={false} autoCapitalize="off" className={input} aria-invalid={!!repoErr} aria-describedby="dv-set-repo-err" />
        <p id="dv-set-repo-err" className="mt-1 text-[11px] text-rose-300 min-h-[1em]">
          {repoErr}
        </p>
      </div>
      <div>
        <label htmlFor="dv-set-branch" className="block text-xs font-medium text-zinc-300 mb-1">
          Branch to watch
        </label>
        <input id="dv-set-branch" value={branch} onChange={(e) => setBranch(e.target.value)} placeholder="main" spellCheck={false} autoCapitalize="off" className={input} />
      </div>
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
      {error && (
        <p role="alert" className="text-[11px] text-rose-300">
          {error}
        </p>
      )}
      <div className="flex items-center gap-2">
        {state.on && (
          <button type="button" className={rowBtn} disabled={busy} onClick={() => put({ on: false })} data-dev-turn-off>
            Turn off Dev mode
          </button>
        )}
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
    </form>
    {/* 0.7.6: "Work from Jira · Off · Set up" (or the On box); nothing on a server without the Jira routes. */}
    {state.on && <JiraSettings space={spaceId} teams={teams} />}
    </>
  );
};
