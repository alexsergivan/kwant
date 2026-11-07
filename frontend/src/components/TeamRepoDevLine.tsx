import React, { useEffect } from 'react';
import { DevModeState, DevRepo, FORGE_NAME, devApi, devStore, normaliseRepo, otherForge } from '../devMode';
import { useDevMode } from './DevModeSwitch';
import { openForgeConnect, openGitHubConnect } from './GitHubConnect';

/**
 * Team settings > Git Repo, one line under the URL (0.7.3 B2): what Dev mode does with this team's repo,
 * read from the space's Dev mode status (the same `/api/dev` the desk reads). Nothing shows for an
 * empty URL or before the status is known.
 */
export const TEAM_REPO_LINES = {
  connected: 'Dev mode watches this repo · GitHub connected',
  connect: 'Dev mode watches this repo. Connect GitHub for reviews.',
  off: 'Dev mode can watch this repo: turn it on next to the space name.',
  git: 'Dev mode reads this repo with git. Reviews need GitHub, GitLab, Gitea or Forgejo.',
  /** 0.7.4: a GitLab or Gitea repo; `{forge}` is its name. */
  forge_connected: 'Dev mode watches this repo · {forge} connected',
  forge_connect: 'Dev mode watches this repo. Connect {forge} for reviews.',
} as const;
export type TeamRepoLine = keyof typeof TEAM_REPO_LINES;

/** `host/owner/name`, lowercased, without userinfo or `.git`: the key Dev mode's repos carry. */
export function repoKeyOf(url: string): string | null {
  const gh = normaliseRepo(url);
  if (gh) return `github.com/${gh}`.toLowerCase();
  let s = url.trim();
  if (!s) return null;
  s = s.replace(/^[a-z+]+:\/\//i, '').replace(/^[^@/]+@/, '');
  // scp form: host:owner/name
  s = s.replace(/^([^/:]+):(?!\d)/, '$1/');
  s = s.replace(/[?#].*$/, '').replace(/\/+$/, '').replace(/\.git$/i, '');
  return s.includes('/') ? s.toLowerCase() : null;
}

/** The team's row in Dev mode's repos, matched by the team, then by the URL. */
export function teamRepoRow(state: DevModeState | null, url: string, teamId?: string | null): DevRepo | undefined {
  const key = repoKeyOf(url);
  if (!state || !key) return undefined;
  const same = (r: DevRepo) => r.key === key || r.key === key.replace(/^([^/]+):\d+\//, '$1/');
  const own = teamId ? state.repos.find((r) => r.team_id === teamId) : undefined;
  return own && same(own) ? own : state.repos.find(same);
}

/** Which line a team's repo gets. null: say nothing. */
export function teamRepoLine(state: DevModeState | null, url: string, teamId?: string | null): TeamRepoLine | null {
  const key = repoKeyOf(url);
  if (!state || !key) return null;
  if (!state.on) return 'off';
  // api.md B2: the team's own entry first, then the same repo (Dev mode keeps one entry per repo).
  // A URL being edited that no longer matches the team's entry is read by the URL.
  // Dev mode's key has no port (`127.0.0.1/owner/name`), a typed URL may (`127.0.0.1:9476/owner/name`).
  const row = teamRepoRow(state, url, teamId);
  // 0.7.4: GitLab, Gitea and Forgejo read reviews once Loose ends has a connection for the host.
  if (row && otherForge(row.forge)) return row.forge_connected ? 'forge_connected' : row.host ? 'forge_connect' : 'git';
  const github = row ? row.provider === 'github' : key.startsWith('github.com/');
  if (!github) return 'git';
  const notConnected = (row ? row.status === 'not_connected' : state.status === 'not_connected') || state.forge !== 'github';
  return notConnected ? 'connect' : 'connected';
}

export const TeamRepoDevLine: React.FC<{ url: string; teamId?: string | null; spaceId?: string | null }> = ({ url, teamId = null, spaceId }) => {
  const space = spaceId ?? devStore.current();
  const state = useDevMode(space);
  // Opened before the desk ever read this space: read it once.
  useEffect(() => {
    if (!space || devStore.get(space)) return;
    devApi.get(space).then((st) => devStore.set(space, st), () => undefined);
  }, [space]);
  const line = teamRepoLine(state, url, teamId);
  if (!line) return null;
  const row = line === 'forge_connect' || line === 'forge_connected' ? teamRepoRow(state, url, teamId) : undefined;
  const forgeName = row?.forge ? FORGE_NAME[row.forge] : '';
  return (
    <p className="text-[11px] text-zinc-400 mt-1" data-team-repo-dev={line} aria-live="polite">
      {line === 'connect' ? (
        <>
          Dev mode watches this repo.{' '}
          <button
            type="button"
            onClick={(e) => openGitHubConnect({ space: space ?? undefined, anchor: e.currentTarget })}
            className="font-medium text-sky-300 hover:text-sky-200 underline underline-offset-2"
            data-team-repo-connect
          >
            Connect GitHub
          </button>{' '}
          for reviews.
        </>
      ) : line === 'forge_connect' && row?.host && otherForge(row.forge) ? (
        <>
          Dev mode watches this repo.{' '}
          <button
            type="button"
            onClick={(e) => openForgeConnect({ space: space ?? undefined, anchor: e.currentTarget, forge: { forge: otherForge(row.forge)!, url: `https://${row.host}` } })}
            aria-haspopup="dialog"
            className="font-medium text-sky-300 hover:text-sky-200 underline underline-offset-2"
            data-team-repo-connect={row.forge}
          >
            Connect {forgeName}
          </button>{' '}
          for reviews.
        </>
      ) : (
        TEAM_REPO_LINES[line].replace('{forge}', forgeName)
      )}
    </p>
  );
};
