// Chat notices when "Run Claude inside the VM" stopped a reply (`turn_isolation_failed`) or a
// reply hit its request limit (`turn_request_limit`). Copy: design/ui.md "Chat notices" and
// "Copy for Ada's decisions 2", plus design/ui-check-446.md fix 4 (a row for every code).
// Server: src/sandbox/agent_vm/notice.rs (`IsolationFailed::to_meta`, `start_text`) and
// src/drivers/cli.rs (`isolation_notice`, `isolated_end`).
import type { ChatMessage } from './components/ChatView';
import { SETTING_HANDLE } from './claudeVm';
import { parseMetaObject } from './questions';

export type FolderFrom = 'team' | 'space' | 'goal' | 'bot';

export interface IsolationFailedMeta {
  kind: 'turn_isolation_failed';
  botName: string;
  engine: string;
  reason: string;
  phase: 'start' | 'mid_turn';
  progressPct: number | null;
  code: string | null;
  folder: string | null;
  folderFrom: FolderFrom | null;
  teamId: string | null;
  teamName: string | null;
  unsafeKind: string | null;
  /** People's words from the server for this refusal, if it sends them (`detail` or `message`). */
  detail: string | null;
}

export interface RequestLimitMeta {
  kind: 'turn_request_limit';
  botName: string;
  limit: number | null;
}

const str = (v: unknown) => (typeof v === 'string' && v.trim() ? v.trim() : null);
const FROMS: FolderFrom[] = ['team', 'space', 'goal', 'bot'];

export function parseIsolationFailed(msg: ChatMessage): IsolationFailedMeta | null {
  if (msg.sender_role !== 'kwant' && msg.sender_role !== 'system') return null;
  const m = parseMetaObject(msg.meta);
  if (m?.kind !== 'turn_isolation_failed') return null;
  const from = str(m.folder_from);
  const pct = typeof m.progress_pct === 'number' && Number.isFinite(m.progress_pct) ? Math.max(0, Math.min(100, Math.floor(m.progress_pct))) : null;
  return {
    kind: 'turn_isolation_failed',
    botName: str(m.kwant_name) ?? (msg.sender_id && msg.sender_id.length <= 40 ? msg.sender_id : 'The bot'),
    engine: str(m.engine) ?? 'claude',
    reason: str(m.reason) ?? str(m.code) ?? 'unknown',
    phase: m.phase === 'mid_turn' ? 'mid_turn' : 'start',
    progressPct: pct,
    code: str(m.code),
    folder: str(m.folder),
    folderFrom: from && (FROMS as string[]).includes(from) ? (from as FolderFrom) : null,
    teamId: str(m.team_id),
    teamName: str(m.team_name),
    unsafeKind: str(m.unsafe_kind),
    detail: str(m.detail) ?? str(m.message),
  };
}

export function parseRequestLimit(msg: ChatMessage): RequestLimitMeta | null {
  if (msg.sender_role !== 'kwant' && msg.sender_role !== 'system') return null;
  const m = parseMetaObject(msg.meta);
  if (m?.kind !== 'turn_request_limit') return null;
  const limit = typeof m.limit === 'number' && m.limit > 0 ? Math.floor(m.limit) : null;
  return { kind: 'turn_request_limit', botName: str(m.kwant_name) ?? (msg.sender_id || 'The bot'), limit };
}

/** The frontend doesn't know $HOME: `/home/<you>` and `/Users/<you>` read as `~`. */
export const shortPath = (p: string) => p.replace(/^\/(?:Users|home)\/[^/]+(?=\/|$)/, '~');

const ENGINE = (e: string) => (e === 'codex' ? 'Codex' : e === 'grok' ? 'Grok' : e === 'claude' ? 'Claude' : e);

// ---- the card's model ----

/** A run of reason text; `folder` runs are drawn in mono, `team` runs as "@Team". */
export type Seg = { text: string; folder?: boolean };

export type NoticeAction = 'retry' | 'computer' | 'accounts' | 'change_folder' | 'edit_bot';

/** Which safety line, if any. `standard` names the setting; the others are notice 6/7 and fix 4 lines. */
export type SafetyKind = 'standard' | 'nothing_ran' | 'unsafe' | 'unsafe_shared' | 'folder_used' | null;

export interface IsolationCard {
  /** Stable key for tests and `data-isolation-row`. */
  row: string;
  first: string;
  reason: Seg[];
  safety: SafetyKind;
  /** In order: the first one is primary. */
  actions: NoticeAction[];
}

/** The safety lines, split so the setting's name can be bold. */
export const SAFETY: Record<Exclude<SafetyKind, null>, [string, string, string] | [string]> = {
  standard: ['While ', SETTING_HANDLE, ' is on, Claude never runs outside its sealed box, so nothing ran on this computer. To run Claude here instead, turn off key protection in Computer settings.'],
  nothing_ran: ['Nothing ran.'],
  unsafe: ['Nothing ran. Turning off ', SETTING_HANDLE, ' would also work, but then Claude could read this whole folder from this computer.'],
  unsafe_shared: ['Nothing ran. Turning off ', SETTING_HANDLE, ' would also work, but then Claude runs on this computer again.'],
  folder_used: ['Nothing ran. Turning off ', SETTING_HANDLE, ' doesn’t change this, because the folder may already hold those settings.'],
};

const t = (text: string): Seg => ({ text });

/** Notice 6, sentence 1, by `unsafe_kind`. */
function unsafeSentence(kind: string | null, folder: string | null): Seg[] {
  const f = folder ? shortPath(folder) : null;
  const withPath = (lead: string | null, rest: string): Seg[] => (f ? [{ text: f, folder: true }, t(rest)] : [t(`${lead ?? 'This folder'}${rest}`)]);
  switch (kind) {
    case 'home':
      return withPath('This folder', ' is your home folder. In its sealed box, Claude would see all your files and sign-ins.');
    case 'contains_home':
    case 'root':
      return withPath('This folder', ' holds your home folder. In its sealed box, Claude would see all your files and sign-ins.');
    case 'sign_ins':
      return withPath('This folder', ' holds sign-ins, like your SSH keys or Claude, Codex or Grok sign-in, so kwant won’t share it with Claude’s sealed box.');
    case 'kwant_data':
      return withPath('This folder', ' holds kwant’s own data, including your sign-ins, so kwant won’t share it with Claude’s sealed box.');
    case 'docker':
      return withPath('This folder', ' holds Docker’s control socket, which would let Claude out of its sealed box.');
    case 'shared_folder':
      return [t('This is the folder teams share when they don’t have their own. If Claude took it into its sealed box, Codex and Grok bots in every team would be kept out of it.')];
    default:
      return [t('This folder holds your home folder or sign-ins, so kwant won’t share it with Claude’s sealed box.')];
  }
}

const canChangeFolder = (m: IsolationFailedMeta) => m.folderFrom === 'team' && !!m.teamId;
const team = (m: IsolationFailedMeta) => (m.teamName ? `@${m.teamName}` : null);

function unsafeFolderCard(m: IsolationFailedMeta): IsolationCard {
  const shared = m.unsafeKind === 'shared_folder';
  const s1 = unsafeSentence(m.unsafeKind, shared ? null : m.folder);
  let s2: string;
  if (shared) s2 = team(m) ? `Give ${team(m)} its own folder, then Retry.` : 'Give this team its own folder, then Retry.';
  else if (m.folderFrom === 'team') s2 = team(m) ? `Pick a project folder for ${team(m)} instead, then Retry.` : 'Pick a project folder for this team instead, then Retry.';
  else if (m.folderFrom === 'goal') s2 = 'Pick a project folder for this goal instead, then Retry.';
  else s2 = 'Pick a project folder instead, then Retry.';
  // The shared folder is a team's when it has none of its own, so Change folder fits whenever the team is known.
  const change = canChangeFolder(m) || (shared && !!m.teamId);
  return {
    row: shared ? 'unsafe_folder:shared_folder' : `unsafe_folder:${m.unsafeKind ?? 'unknown'}`,
    first: `${m.botName} didn’t reply: Claude can’t take this folder into its sealed box.`,
    reason: [...s1, t(` ${s2}`)],
    safety: shared ? 'unsafe_shared' : 'unsafe',
    actions: change ? ['change_folder', 'retry'] : ['retry'],
  };
}

function folderUsedCard(m: IsolationFailedMeta): IsolationCard {
  const E = ENGINE(m.engine);
  const bot = m.botName;
  const f: Seg = m.folder ? { text: shortPath(m.folder), folder: true } : t('this folder');
  let reason: Seg[];
  let row: string;
  if (m.folderFrom === 'bot') {
    row = 'folder_used_by_isolated_bot:bot';
    reason = [t(`${bot} used this folder as Claude in its sealed box, and it could have left settings there that ${E} would load on this computer. Switch ${bot} back to Claude to keep working here.`)];
  } else if (m.folderFrom === 'team') {
    row = 'folder_used_by_isolated_bot:team';
    const other = team(m) ? `pick another folder for ${team(m)}` : 'pick another folder';
    reason = [t('A Claude bot in its sealed box could have left settings in '), f, t(` that ${E} would load on this computer, so kwant keeps ${E} out of it for now. Switch ${bot} to Claude, or ${other}, then Retry.`)];
  } else {
    row = 'folder_used_by_isolated_bot:other';
    reason = [t('A Claude bot in its sealed box could have left settings in '), f, t(` that ${E} would load on this computer, so kwant keeps ${E} out of it for now. Switch ${bot} to Claude, then Retry.`)];
  }
  const second: NoticeAction[] = canChangeFolder(m) ? ['change_folder'] : ['retry'];
  return {
    row,
    first: `${bot} didn’t reply: ${E} won’t run in a folder Claude used in its sealed box.`,
    reason,
    safety: 'folder_used',
    actions: ['edit_bot', ...second],
  };
}

const GENERIC_FIRST = (bot: string) => `${bot} didn’t reply: Claude’s sealed box couldn’t start.`;

/**
 * The reason for a code with no row of its own. The server's own words win (a `detail` in the
 * meta, or what its saved text says beyond the template); "Something went wrong ({code})" only
 * when there's nothing else.
 */
export function unknownReason(m: IsolationFailedMeta, savedText: string | null | undefined): string {
  const code = m.code ?? m.reason;
  const tail = `Retry, or turn off ${SETTING_HANDLE} to run Claude on this computer.`;
  const own = m.detail ?? serverReason(savedText, m.botName);
  if (own) return `${own.replace(/[.\s]+$/, '')}. ${tail}`;
  return `Something went wrong (${code}). ${tail}`;
}

/**
 * The human part of a saved notice text: without its first sentence ("{Bot} didn't reply: …"),
 * the safety sentence and the generic "Something went wrong (code). Retry, or turn off…" template.
 * Null when only the template is left.
 */
export function serverReason(text: string | null | undefined, bot: string): string | null {
  if (!text) return null;
  let s = text.replace(/[‘’]/g, "'").trim();
  const lead = `${bot.replace(/[‘’]/g, "'")} didn't reply:`;
  if (s.startsWith(lead)) {
    const end = s.indexOf('.', lead.length);
    s = end >= 0 ? s.slice(end + 1) : '';
  }
  s = s
    // 0.7.7 notices stay in chats with the old name, so both wordings are stripped (vm-naming.md).
    .replace(/While Run Claude inside the VM is on, Claude never runs outside it, so nothing ran on this computer\.(?: To run Claude here instead, turn that off in Computer settings\.)?/g, '')
    .replace(/While key protection is on, Claude never runs outside its sealed box, so nothing ran on this computer\.(?: To run Claude here instead, turn off key protection in Computer settings\.)?/g, '')
    .replace(/Something went wrong \([^)]*\)\.?/g, '')
    .replace(/Retry, or turn off (?:Run Claude inside the VM|key protection) to run Claude on this computer\.?/g, '')
    .replace(/\s+/g, ' ')
    .trim();
  return s ? s.replace(/'/g, '’') : null;
}

/** Everything a `turn_isolation_failed` card shows, by reason (one row per code the server sends). */
export function isolationCard(m: IsolationFailedMeta, savedText?: string | null): IsolationCard {
  const bot = m.botName;
  const couldnt = GENERIC_FIRST(bot);
  const notReady = `${bot} didn’t reply: Claude’s sealed box isn’t ready yet.`;
  const std = (row: string, first: string, reason: string, actions: NoticeAction[] = ['retry', 'computer'], safety: SafetyKind = 'standard'): IsolationCard => ({
    row,
    first,
    reason: [t(reason)],
    safety,
    actions,
  });

  if (m.reason === 'unsafe_folder') return unsafeFolderCard(m);
  if (m.reason === 'folder_used_by_isolated_bot') return folderUsedCard(m);
  if (m.phase === 'mid_turn')
    return std(
      'mid_turn',
      `${bot}’s reply stopped partway: Claude’s sealed box lost its connection.`,
      'What it changed before that is in the task folder, and Undo still works. Retry to start the reply again.',
    );

  switch (m.reason) {
    case 'docker_unavailable':
      return std('docker_unavailable', couldnt, 'Docker isn’t running on this computer. Start Docker, then Retry.');
    case 'preparing':
      // progress_pct is always null in 0.7.7 (ui-check §4), so the no-% wording unless it's sent.
      return std(
        'preparing',
        notReady,
        m.progressPct !== null ? `The one-time setup is at ${m.progressPct}%. Retry when it’s done.` : 'The one-time setup is still running. Retry when it’s done.',
      );
    case 'download_failed':
    case 'download_needed':
      return std(m.reason, notReady, 'The one-time setup didn’t finish. Try it again in Computer settings, then Retry.');
    case 'relay_down':
      return std('relay_down', couldnt, 'kwant couldn’t connect Claude to your sign-in. Restart kwant, then Retry.');
    case 'sign_in_expired':
      return std('sign_in_expired', `${bot} didn’t reply: your Claude sign-in has expired.`, 'Sign in again in Settings › This space › Apps & accounts, then Retry.', ['retry', 'accounts'], null);
    case 'no_credential':
      return std('no_credential', `${bot} didn’t reply: there’s no Claude sign-in for it on this computer.`, 'Sign in to Claude in Settings › This space › Apps & accounts, then Retry.', ['retry', 'accounts'], null);
    case 'host_is_root':
      return std('host_is_root', couldnt, 'kwant is running as root, so Claude’s sealed box can’t run as you. Start kwant as your normal user, then Retry.');
    case 'folder_name_unsupported':
      return std(
        'folder_name_unsupported',
        `${bot} didn’t reply: Claude can’t take this folder into its sealed box.`,
        'The folder’s name has characters kwant can’t pass to Claude’s sealed box, like a line break. Rename the folder, then Retry.',
        ['retry'],
        'nothing_ran',
      );
    case 'folder_record_failed':
      return std('folder_record_failed', couldnt, 'kwant couldn’t note that this folder is used by Claude’s sealed box, so it didn’t start Claude. Retry; if it keeps happening, restart kwant.');
    case 'container_failed':
      return std('container_failed', couldnt, 'Docker couldn’t start it. Retry; if it keeps happening, restart Docker.');
    case 'unsupported_os':
    case 'not_supported_yet':
    case 'mac_later':
      return std(m.reason, couldnt, `Isolation is only on Linux for now. Turn off ${SETTING_HANDLE} to run Claude on this computer.`, ['computer']);
    default:
      return std('unknown', couldnt, unknownReason(m, savedText));
  }
}

/** Notice 5: the request limit. */
export function requestLimitText(m: RequestLimitMeta) {
  return {
    first: m.limit ? `${m.botName} paused after ${m.limit} requests to Claude in this reply.` : `${m.botName} paused: this reply reached its limit of requests to Claude.`,
    body: 'That’s the most one reply can make, so a bot stuck in a loop can’t keep using up your Claude plan. Its work so far is kept.',
  };
}

/** What Continue sends, as a visible message from the person (ui.md notice 5). */
export const CONTINUE_TEXT = 'Continue where you left off.';

/** A notice that ends a reply and offers Retry or Continue (for `retryTargets`). */
export const isIsolationNotice = (msg: ChatMessage) => !!parseIsolationFailed(msg) || !!parseRequestLimit(msg);
