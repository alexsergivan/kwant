// 0.8.0 import sources (release-080/import-contract.md §2–§4, Mira's a4-import.md): Found on this
// computer, a picked folder, and a GitHub link. Pure helpers only; the steps are in ImportSources.tsx.

import { ImportError, Preview, importErrorText, plural } from './imports';
import { PICK_LIMITS, storedZip, zipOverhead } from './importZip';

// ------------------------------------------------------------------ Found on this computer

export interface LocalSource {
  id: string;
  kind: string;
  label: string;
  /** As the server shows it: `~/.codex`, or `~\.codex` on Windows. Shown exactly as sent. */
  path: string;
  bots: number;
  skills: number;
  /** Memory files (MEMORY.md, USER.md), not entries. */
  memories: number;
  names: string[];
  too_large: boolean;
}

export interface LocalList {
  available: boolean;
  reason?: string;
  sources: LocalSource[];
}

/** The five usual places the server looks (shown when nothing was found). */
export const USUAL_PLACES = ['~/.openclaw', '~/.hermes', '~/.claude/agents', '~/.claude/skills', '~/.codex'] as const;

/** Sources that can be ticked: not too big. */
export const tickable = (list: ReadonlyArray<LocalSource>) => list.filter((s) => !s.too_large);

/** "OpenClaw, Claude Code and Codex", or "OpenClaw, Hermes Agent, Codex and 2 more". */
export function foundHint(list: ReadonlyArray<LocalSource>): string {
  const labels = [...new Set(list.map((s) => s.label))];
  if (labels.length === 0) return '';
  if (labels.length === 1) return labels[0];
  if (labels.length <= 3) return `${labels.slice(0, -1).join(', ')} and ${labels[labels.length - 1]}`;
  return `${labels.slice(0, 3).join(', ')} and ${labels.length - 3} more`;
}

/** "1 bot · 3 skills · 2 memory files", leaving out zeros. */
export function countsLine(s: Pick<LocalSource, 'bots' | 'skills' | 'memories'>): string {
  return [s.bots ? plural(s.bots, 'bot') : '', s.skills ? plural(s.skills, 'skill') : '', s.memories ? plural(s.memories, 'memory file') : '']
    .filter(Boolean)
    .join(' · ');
}

/** The ErrorBox copy for `POST /api/import/local/preview`, from its code. Nothing raw reaches the person. */
export function localErrorText(err: unknown, ticked: ReadonlyArray<LocalSource>): { title: string; detail: string } {
  const code = err instanceof ImportError ? err.code : '';
  const label = ticked.map((s) => s.label).join(' or ') || 'these';
  switch (code) {
    case 'not_found':
      return { title: 'One of these isn’t there any more. kwant looked again.', detail: '' };
    case 'too_large':
      return { title: 'Together these are too big (over 500 files or 50 MB). Tick fewer.', detail: '' };
    case 'unsupported':
      return { title: `kwant found nothing it can bring in ${label}.`, detail: '' };
    case 'invalid':
      return { title: `kwant couldn’t read ${label}.`, detail: '' };
    case 'network':
    case 'person_cancelled':
      return importErrorText(err);
    default:
      return { title: 'Couldn’t read these. Nothing was imported.', detail: '' };
  }
}

/** The ErrorBox copy when the list itself couldn't load (F4). */
export function localListErrorText(err: unknown): { title: string; detail: string } {
  const code = err instanceof ImportError ? err.code : '';
  return {
    title: 'Couldn’t look on this computer',
    detail: code === 'network' ? 'kwant couldn’t reach its server. Nothing was read.' : 'Nothing was read. Try again in a moment.',
  };
}

export interface LocalOrigin {
  /** Every bot came from this one source: "From OpenClaw · ~/.openclaw/workspace" on each bot. */
  botSource: LocalSource | null;
  /** The shared skills all came from this source: "Also: 7 shared skills from ~/.claude/skills." */
  skillSource: LocalSource | null;
}

/**
 * Where the preview's bots and shared skills came from, when the ticked sources say it for sure (the
 * preview doesn't name a source per bot): one ticked source with bots, one with only skills.
 */
export function localOrigin(ticked: ReadonlyArray<LocalSource>): LocalOrigin {
  const withBots = ticked.filter((s) => s.bots > 0);
  const skillsOnly = ticked.filter((s) => s.bots === 0 && s.skills > 0);
  return { botSource: withBots.length === 1 ? withBots[0] : null, skillSource: skillsOnly.length === 1 ? skillsOnly[0] : null };
}

// ------------------------------------------------------------------ Folder

/** Can this browser pick a folder? iOS Safari can't, even where the property exists. */
export function folderPickSupported(
  proto: object | undefined = typeof HTMLInputElement !== 'undefined' ? HTMLInputElement.prototype : undefined,
  nav: { userAgent?: string; maxTouchPoints?: number } | undefined = typeof navigator !== 'undefined' ? navigator : undefined,
): boolean {
  if (!proto || !('webkitdirectory' in proto)) return false;
  const ua = nav?.userAgent ?? '';
  if (/iPhone|iPad|iPod/.test(ua)) return false;
  // iPadOS asks for the desktop site: a "Macintosh" with a touch screen.
  if (/Macintosh/.test(ua) && (nav?.maxTouchPoints ?? 0) > 1) return false;
  return true;
}

export interface PickedEntry {
  /** Relative path including the picked folder's own name: "support-skills/seo/SKILL.md". */
  path: string;
  size: number;
  file: Blob;
}

/**
 * What a picked folder may send. The folder is zipped and uploaded to the server, which may be remote,
 * so only files an import format reads are packed: the same allowlist as `wanted_in_repo`
 * (src/import/archive.rs) plus the OpenClaw extras of `wanted` (src/import/local.rs). Key stores are
 * refused first, by name, so a sign-in file never leaves this computer even if it looks like a .json.
 */
const READ_NAMES = new Set([
  'skill.md', 'agents.md', 'agents.override.md', 'soul.md', 'identity.md', 'user.md', 'memory.md', 'company.md',
  'team.md', 'task.md', 'employee.json', 'schedule.md', '.paperclip.yaml', '.bundled_manifest', 'boot.md', 'bootstrap.md',
]);
/** The only hidden files kept (Paperclip and Hermes markers). */
const READ_DOTFILES = new Set(['.paperclip.yaml', '.bundled_manifest']);
const NOT_BOT_JSON = new Set(['package.json', 'package-lock.json', 'tsconfig.json']);
/** Sign-in and settings files of OpenClaw, Hermes and Codex (Ada's #455 review): never packed. */
const KEY_STORE_FILES = new Set(['openclaw.json', 'auth.json', 'config.toml', 'config.yaml', 'state.db']);
const KEY_STORE_DIRS = new Set(['credentials', 'sessions']);

const isLicence = (base: string) => {
  const stem = base.split('.')[0];
  return ['license', 'licence', 'copying'].includes(stem) && (base === stem || base.endsWith('.md') || base.endsWith('.txt'));
};

/** Whether some format reads this file (lowercased path below the picked folder). */
function formatReads(inner: string[]): boolean {
  const base = inner[inner.length - 1] ?? '';
  const parent = inner[inner.length - 2] ?? '';
  if (READ_NAMES.has(base) || isLicence(base)) return true;
  if (base.endsWith('.md') && ['agents', 'memories', 'memory'].includes(parent)) return true;
  return base.endsWith('.json') && !NOT_BOT_JSON.has(base);
}

type LeftKind = 'secrets' | 'git' | 'node_modules' | 'keys' | 'hidden' | 'unread' | 'big';
const LEFT_LABELS: Record<LeftKind, string> = {
  secrets: 'Sign-in and settings files (never sent)',
  git: '.git/',
  node_modules: 'node_modules/',
  keys: 'Keys (.pem, .key, id_rsa)',
  hidden: 'Other hidden files',
  unread: 'Files no import reads',
  big: 'Files over 4 MB',
};
const LEFT_ORDER: LeftKind[] = ['secrets', 'keys', 'git', 'node_modules', 'hidden', 'big', 'unread'];

/** Why a file is left out before packing, or null to keep it. */
export function leftOutKind(path: string, size: number): LeftKind | null {
  const parts = path.split(/[\\/]/).filter(Boolean);
  // Below the picked folder, which may itself be hidden (".openclaw").
  const inner = parts.slice(1).map((p) => p.toLowerCase());
  const base = inner[inner.length - 1] ?? '';
  const dirs = inner.slice(0, -1);
  if (dirs.includes('.git')) return 'git';
  if (dirs.includes('node_modules')) return 'node_modules';
  if (base.startsWith('.env') || KEY_STORE_FILES.has(base) || base.startsWith('state.db') || dirs.some((d) => KEY_STORE_DIRS.has(d)))
    return 'secrets';
  if (base.endsWith('.pem') || base.endsWith('.key') || base.startsWith('id_rsa')) return 'keys';
  if (dirs.some((d) => d.startsWith('.')) || (base.startsWith('.') && !READ_DOTFILES.has(base))) return 'hidden';
  if (!formatReads(inner)) return 'unread';
  if (size > PICK_LIMITS.fileBytes) return 'big';
  return null;
}

/** A Codex `config.toml` that isn't in a hidden folder: sent by name only (see `markers`). */
const isCodexMarker = (path: string) => {
  const inner = path.split(/[\\/]/).filter(Boolean).slice(1);
  return inner[inner.length - 1]?.toLowerCase() === 'config.toml' && !inner.slice(0, -1).some((d) => d.startsWith('.') || d === 'node_modules');
};

export interface FolderPlan {
  /** The picked folder's name. */
  name: string;
  kept: PickedEntry[];
  keptBytes: number;
  /** Bytes of the zip that would be sent. */
  packedBytes: number;
  left: Array<{ label: string; count: number }>;
  /** Why it can't be sent (rose, role=alert), or null. */
  problem: string | null;
  /**
   * Codex `config.toml` files, sent as empty entries: the server only checks the name is there next to
   * AGENTS.md (formats.rs `codex_roots`) and never opens it, so its contents (which can hold keys) never
   * leave this computer.
   */
  markers: string[];
}

export const formatBytes = (n: number): string =>
  n < 1024 * 1024 ? `${Math.max(1, Math.round(n / 1024))} KB` : `${(n / (1024 * 1024)).toFixed(1).replace(/\.0$/, '')} MB`;

/** What a picked folder becomes: what's kept, what's left out (and why), and whether it fits. */
export function folderPlan(entries: ReadonlyArray<PickedEntry>): FolderPlan {
  const name = entries[0]?.path.split(/[\\/]/)[0] || 'folder';
  const kept: PickedEntry[] = [];
  const markers: string[] = [];
  const counts = new Map<LeftKind, number>();
  for (const e of entries) {
    const why = leftOutKind(e.path, e.size);
    if (isCodexMarker(e.path)) markers.push(e.path);
    if (why) counts.set(why, (counts.get(why) ?? 0) + 1);
    else kept.push(e);
  }
  const keptBytes = kept.reduce((n, e) => n + e.size, 0);
  const packedBytes = keptBytes + zipOverhead([...kept.map((e) => e.path), ...markers]);
  const left = LEFT_ORDER.filter((k) => counts.has(k)).map((k) => ({ label: LEFT_LABELS[k], count: counts.get(k)! }));
  const hasSkills = kept.some((e) => e.path.split('/').slice(1, -1).includes('skills'));
  const smaller = `Pick a smaller part of it${hasSkills ? ', such as its skills folder' : ''}.`;
  let problem: string | null = null;
  if (entries.length > 0 && kept.length === 0) problem = 'Nothing in that folder can be brought in: no SKILL.md, AGENTS.md, SOUL.md, agents/*.md or bot .json.';
  else if (packedBytes > PICK_LIMITS.zipBytes)
    problem = `That folder is ${Math.ceil(packedBytes / (1024 * 1024))} MB once packed, over the 20 MB limit. ${smaller}`;
  else if (kept.length > PICK_LIMITS.files) problem = `That folder has ${kept.length} readable files, over the 500 limit. ${smaller}`;
  return { name, kept, keptBytes, packedBytes, left, problem, markers };
}

/** The zip a folder plan sends: kept files with their relative paths, plus empty Codex markers. */
export async function folderZip(plan: FolderPlan): Promise<{ body: Blob; filename: string }> {
  const data = await Promise.all(plan.kept.map(async (e) => new Uint8Array(await e.file.arrayBuffer())));
  const zip = storedZip([
    ...plan.kept.map((e, i) => ({ name: e.path.replace(/\\/g, '/'), data: data[i] })),
    ...plan.markers.map((m) => ({ name: m.replace(/\\/g, '/'), data: new Uint8Array(0) })),
  ]);
  return { body: new Blob([zip.buffer as ArrayBuffer], { type: 'application/zip' }), filename: `${plan.name}.zip` };
}

// ------------------------------------------------------------------ GitHub link

export interface GithubLicence {
  spdx: string | null;
  name: string;
  file: string;
}

export interface GithubInfo {
  owner: string;
  repo: string;
  ref: string;
  path: string | null;
  commit: string | null;
  licence: GithubLicence | null;
}

export type GithubPreview = Preview & { github: GithubInfo };

const SEG = /^[A-Za-z0-9._-]{1,100}$/;

/**
 * The client's check of a GitHub link (contract §3): github.com or www.github.com over https (or no
 * scheme), no user or port, and one of the three shapes. Null when it looks fine; the server checks again.
 */
export function githubLinkOk(raw: string): { owner: string; repo: string } | null {
  const s = raw.trim();
  if (!s) return null;
  let u: URL;
  try {
    u = new URL(/^[a-z][a-z0-9+.-]*:\/\//i.test(s) ? s : `https://${s}`);
  } catch {
    return null;
  }
  if (u.protocol !== 'https:' || u.username || u.password || u.port) return null;
  if (!['github.com', 'www.github.com'].includes(u.hostname.toLowerCase())) return null;
  if (/%2e|%2f|%5c|\\/i.test(s)) return null;
  const parts = u.pathname.replace(/\/+$/, '').split('/').slice(1);
  if (parts.length < 2 || parts.some((p) => p === '' || p === '.' || p === '..')) return null;
  const [owner, rawRepo, kind, ref, ...rest] = parts;
  const repo = rawRepo.replace(/\.git$/, '');
  if (!SEG.test(owner) || !SEG.test(repo)) return null;
  if (parts.length === 2) return { owner, repo };
  if (kind === 'tree' && ref && SEG.test(ref)) return { owner, repo };
  if (kind === 'blob' && ref && SEG.test(ref) && rest.length > 0) return { owner, repo };
  return null;
}

/** The server's 20 MB line (src/import/github.rs), in the app's typography. */
export const GITHUB_OVER_20MB = 'This repository is larger than 20 MB, so kwant can’t fetch it. Download the folder you want, zip it and import the zip instead.';

export const NOT_GITHUB = 'Only github.com links work here. kwant doesn’t fetch from other sites.';

/** The ErrorBox copy for `POST /api/import/github/preview` (G4), from its code. */
export function githubErrorText(err: unknown): { title: string; detail: string } {
  const code = err instanceof ImportError ? err.code : '';
  switch (code) {
    case 'not_found':
      return { title: 'No public repo at that link', detail: 'It may be private, or the branch or link may be wrong. Only public repos work for now.' };
    case 'too_large':
      // The whole download was over 20 MB (#455 11efe7d2): a folder link wouldn't help, a zip does.
      if (err instanceof ImportError && /larger than 20 MB/.test(err.message))
        return { title: 'That repo is too big', detail: GITHUB_OVER_20MB };
      return { title: 'That repo is too big', detail: 'Link to the folder you need instead, like …/tree/main/skills. (20 MB, or over 500 readable files.)' };
    case 'unsupported':
      return { title: 'Nothing kwant can bring here', detail: 'No SKILL.md, AGENTS.md or bot files at that link.' };
    case 'redirect':
      return { title: 'GitHub sent kwant somewhere else', detail: 'kwant never follows a redirect. Check the link, or try the repo’s new address.' };
    case 'invalid':
      return { title: 'That download wasn’t a safe zip', detail: 'Nothing was imported.' };
    case 'fetch_failed':
      return { title: 'GitHub didn’t answer', detail: 'Check your connection and try again. Nothing was imported.' };
    case 'not_github':
      return { title: NOT_GITHUB, detail: '' };
    case 'network':
    case 'person_cancelled':
      return importErrorText(err);
    default:
      return { title: 'Something went wrong', detail: 'Nothing was imported. Try again in a moment.' };
  }
}

/** "main · commit 3f2a91c · /skills", or "default branch" for HEAD. */
export function githubRefLine(g: Pick<GithubInfo, 'ref' | 'commit' | 'path'>): string {
  return [g.ref === 'HEAD' ? 'default branch' : g.ref, g.commit ? `commit ${g.commit.slice(0, 7)}` : '', g.path ? `/${g.path.replace(/^\/+/, '')}` : '']
    .filter(Boolean)
    .join(' · ');
}

/** The licence chip and its one plain-words line. `warn` is the amber "No licence" chip. */
export function licenceLine(l: GithubLicence | null): { chip: string; line: string; warn: boolean } {
  if (!l) return { chip: 'No licence', line: '', warn: true };
  const s = (l.spdx ?? '').toUpperCase();
  if (!l.spdx) return { chip: 'Unknown licence', line: 'Read the licence before you share these.', warn: false };
  let line = '';
  if (/^(MIT|BSD|ISC|APACHE|UNLICENSE|CC0)/.test(s)) line = 'Free to reuse. Keep the author’s notice.';
  else if (/^(GPL|LGPL|AGPL|MPL)/.test(s)) line = `If you share these on, share them under ${l.spdx} too.`;
  else if (/^CC-BY-SA/.test(s)) line = 'Free to reuse with credit, under the same licence.';
  else if (/^CC-BY/.test(s)) line = 'Free to reuse with credit.';
  return { chip: `${l.spdx} licence`, line, warn: false };
}

export const githubRepoUrl = (g: Pick<GithubInfo, 'owner' | 'repo'>) => `https://github.com/${encodeURIComponent(g.owner)}/${encodeURIComponent(g.repo)}`;

/** The licence file on github.com, at the fetched commit when known. */
export function githubLicenceUrl(g: GithubInfo): string | null {
  if (!g.licence?.file) return null;
  const at = g.commit || g.ref || 'HEAD';
  const dir = g.path ? `${g.path.replace(/^\/+|\/+$/g, '')}/` : '';
  const file = g.licence.file.includes('/') ? g.licence.file : `${dir}${g.licence.file}`;
  return `${githubRepoUrl(g)}/blob/${encodeURIComponent(at)}/${file.split('/').map(encodeURIComponent).join('/')}`;
}

/** "From github.com/owner/repo (MIT)" for the Done step. */
export const githubFromLine = (g: GithubInfo) =>
  `From github.com/${g.owner}/${g.repo}${g.licence ? ` (${g.licence.spdx ?? g.licence.name})` : ''}.`;

/** The server's attribution lines in `omitted` ("From github.com/…"): the source block already says it. */
export const isAttribution = (line: string) => /^From github\.com\//.test(line);
