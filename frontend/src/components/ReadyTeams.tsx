// 0.7.7 Ready-made teams: the app-side glue. One provider per space keeps the template list, opens the
// preview sheet from any entry point (welcome, New team, sidebar, Manage team), shows the Created toast
// and runs Undo. `TeamNeedsTop` is the Needs card at the top of a template team's chat (Q-T5: a client
// overlay built from live state, not a message). Design: next-077/design/design.md §T, §D. API: api.md.
import React, { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react';
import { TeamPreviewSheet, type SheetMode } from './TeamPreviewSheet';
import { TeamCreatedLine, TeamCreatedToast, TeamNeedsCard, UndoTeamConfirm } from './TeamNeedsCard';
import { openForgeConnect, openGitHubConnect } from './GitHubConnect';
import { useDevMode } from './DevModeSwitch';
import { askAbout } from './results/SmartResult';
import { botActionsApi } from '../botActions';
import { looseApi, type LooseSource } from '../looseEnds';
import { connectionsApi } from '../connections';
import { devApi, devStore } from '../devMode';
import {
  SELL_STARTER,
  TeamsError,
  plural,
  teamsApi,
  undoWindowOver,
  type CreatedFrom,
  type CreatedTeam,
  type NeedsContext,
  type TeamTemplate,
  type TeamTemplateInfo,
  type TemplateNeed,
} from '../teams/teamTemplates';

// ---------------------------------------------------------------------------------------------------
// Which teams a template made. The team row says so (api.md §0.8: `created_from_template` and
// `template_action_id`); a team made here is remembered at once so its card shows before the roster reload.
// `GET /api/teams/:id/template-undo` is read only when the person presses Undo.

export interface MadeTeam {
  templateId: string | null;
  handle: string;
  createdAt: number;
  actionId: string;
  undoUntil: number;
}

const madeKey = (teamId: string) => `kwnt:team-made:${teamId}`;
const remember = (teamId: string, m: MadeTeam) => {
  try {
    localStorage.setItem(madeKey(teamId), JSON.stringify(m));
  } catch {
    /* private mode: the server read covers it */
  }
};
const recall = (teamId: string): MadeTeam | null => {
  try {
    const s = localStorage.getItem(madeKey(teamId));
    return s ? (JSON.parse(s) as MadeTeam) : null;
  } catch {
    return null;
  }
};
const forget = (teamId: string) => {
  try {
    localStorage.removeItem(madeKey(teamId));
  } catch {
    /* ignore */
  }
};
const DAY = 86_400_000;

/** A team row → what the Needs card and Undo need; null when no template made it. `undefined` when the
 * server doesn't send the fields (an older build): the local record decides then. */
export const madeFromRow = (t: ReadyTeam, local: MadeTeam | null): MadeTeam | null | undefined => {
  if (t.created_from_template === undefined) return undefined;
  if (!t.created_from_template || !t.template_action_id) return null;
  const from: CreatedFrom = t.created_from_template;
  const createdAt = local?.createdAt ?? (t.created_at ? Date.parse(t.created_at) : NaN);
  const at = Number.isFinite(createdAt) ? createdAt : Date.now();
  return {
    templateId: from === 'suggested' ? null : from,
    handle: t.name,
    createdAt: at,
    actionId: t.template_action_id,
    undoUntil: at + DAY,
  };
};

// ---------------------------------------------------------------------------------------------------

export interface ReadyTeam {
  id: string;
  name: string;
  workspace_path?: string | null;
  repo_url?: string | null;
  created_at?: string | null;
  created_from_template?: CreatedFrom;
  template_action_id?: string | null;
}

interface OpenOpts {
  /** Called when the sheet closes without creating (Esc returns to the New team form, T7.2). */
  onCancel?: () => void;
  /** Called after a create, before the app selects the team. */
  onDone?: () => void;
}

interface ReadyTeamsValue {
  space: string;
  /** null while loading; [] when the server has no templates route (an older server). */
  templates: TeamTemplate[] | null;
  available: boolean;
  open: (mode: SheetMode, opts?: OpenOpts) => void;
  made: (teamId: string) => MadeTeam | null;
  undo: (team: ReadyTeam) => void;
}

const Ctx = createContext<ReadyTeamsValue | null>(null);
export const useReadyTeams = () => useContext(Ctx);

const cache = new Map<string, TeamTemplate[]>();

export interface ReadyTeamsProviderProps {
  space: string;
  bots: { id: string; name: string; role?: string }[];
  teams: ReadyTeam[];
  /** Bots and teams changed on the server: reload them. */
  onChanged: () => void;
  /** Open the new team's chat (team, channel id). */
  onOpenTeam: (team: CreatedTeam['team'], channelId: string) => void;
  /** Message count of a team's chat, for the Undo confirm; the server's count wins when it answers. */
  children: React.ReactNode;
}

export const ReadyTeamsProvider: React.FC<ReadyTeamsProviderProps> = ({ space, bots, teams, onChanged, onOpenTeam, children }) => {
  const [templates, setTemplates] = useState<TeamTemplate[] | null>(() => cache.get(space) ?? null);
  const [templatesError, setTemplatesError] = useState(false);
  const [available, setAvailable] = useState(true);
  const [loadSeq, setLoadSeq] = useState(0);

  useEffect(() => {
    let live = true;
    setTemplates(cache.get(space) ?? null);
    setTemplatesError(false);
    teamsApi.templates(space).then(
      (t) => {
        if (!live) return;
        cache.set(space, t);
        setTemplates(t);
        setAvailable(true);
      },
      (e) => {
        if (!live) return;
        // An older server has no route: no entry points at all. Anything else: the sheet says so.
        if (e instanceof TeamsError && e.status === 404) {
          setAvailable(false);
          setTemplates([]);
        } else if (!cache.get(space)) setTemplatesError(true);
      },
    );
    return () => {
      live = false;
    };
  }, [space, loadSeq]);

  // ---- the sheet
  const [sheet, setSheet] = useState<{ mode: SheetMode; opts: OpenOpts; n: number } | null>(null);
  const open = useCallback((mode: SheetMode, opts: OpenOpts = {}) => setSheet((s) => ({ mode, opts, n: (s?.n ?? 0) + 1 })), []);
  const takenNames = useMemo(() => bots.map((b) => b.name), [bots]);
  const takenHandles = useMemo(() => teams.map((t) => t.name), [teams]);

  // ---- made teams (local first, then the server)
  const [made, setMade] = useState<Record<string, MadeTeam | null>>({});
  useEffect(() => {
    const next: Record<string, MadeTeam | null> = {};
    for (const t of teams) {
      const local = recall(t.id);
      const row = madeFromRow(t, local);
      if (row === undefined) next[t.id] = local;
      else {
        next[t.id] = row;
        if (row) remember(t.id, row);
        else if (local) forget(t.id);
      }
    }
    setMade((x) => {
      // A team made a moment ago stays until the roster reload brings its row.
      for (const [id, m] of Object.entries(x)) if (m && !(id in next)) next[id] = m;
      return next;
    });
  }, [teams]);
  const madeOf = useCallback((id: string) => made[id] ?? null, [made]);

  // ---- toast and Undo
  const [toast, setToast] = useState<{ text: string; team?: ReadyTeam; n: number } | null>(null);
  const [confirm, setConfirm] = useState<{ team: ReadyTeam; info: TeamTemplateInfo; label: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [undoError, setUndoError] = useState<string | null>(null);
  const labelOf = (id: string | null) => templates?.find((t) => t.id === id)?.label ?? 'new';

  const runUndo = useCallback(
    async (team: ReadyTeam, actionId: string, handle: string) => {
      setBusy(true);
      setUndoError(null);
      try {
        await botActionsApi.undo(actionId);
        forget(team.id);
        setMade((x) => ({ ...x, [team.id]: null }));
        setConfirm(null);
        setToast((t) => ({ text: `Removed @${handle}.`, n: (t?.n ?? 0) + 1 }));
        onChanged();
      } catch (e) {
        if (undoWindowOver(e)) {
          setConfirm(null);
          setToast((t) => ({ text: `@${handle} is more than 24 hours old, so it can’t be undone. Remove its bots in Manage team.`, n: (t?.n ?? 0) + 1 }));
          return;
        }
        const msg = e instanceof Error && e.message ? e.message : 'Something went wrong.';
        if (confirm) setUndoError(`Couldn’t undo the team: ${msg}`);
        else setToast((t) => ({ text: `Couldn’t undo @${handle}: ${msg}`, n: (t?.n ?? 0) + 1 }));
      } finally {
        setBusy(false);
      }
    },
    [onChanged, confirm],
  );

  const undo = useCallback(
    async (team: ReadyTeam) => {
      const m = made[team.id] ?? recall(team.id);
      let info: TeamTemplateInfo | null = null;
      try {
        info = await teamsApi.templateInfo(team.id, space);
      } catch {
        info = null;
      }
      if (info && !info.can_undo) {
        setToast((t) => ({ text: `@${team.name} can’t be undone any more.`, n: (t?.n ?? 0) + 1 }));
        return;
      }
      const actionId = info?.action_id ?? m?.actionId;
      if (!actionId) return;
      // A6: confirm only when the chat has messages (or a bot is also in another team).
      if (info && (info.chat_messages > 0 || info.other_teams.length > 0 || info.edited_skills.length > 0)) {
        setUndoError(null);
        setConfirm({ team, info, label: labelOf(info.template_id) });
        return;
      }
      void runUndo(team, actionId, info?.removes.handle || m?.handle || team.name);
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [made, runUndo, templates, space],
  );

  const onCreated = (r: CreatedTeam, _t: TeamTemplate, handle: string) => {
    const m: MadeTeam = { templateId: r.template_id, handle, createdAt: Date.now(), actionId: r.action_id, undoUntil: Date.parse(r.undo_until) || Date.now() + 86_400_000 };
    remember(r.team.id, m);
    setMade((x) => ({ ...x, [r.team.id]: m }));
    sheet?.opts.onDone?.();
    setSheet(null);
    onChanged();
    onOpenTeam(r.team, r.channel_id);
    // T4: "Created @inbox: 3 bots, 2 skills, 1 routine (off)."
    const added = r.bots.filter((b) => !b.existing).length;
    const parts = [plural(added, 'bot')];
    if (r.skills.length) parts.push(plural(r.skills.length, 'skill'));
    if (r.routine) parts.push('1 routine (off)');
    const text = `Created @${handle}: ${parts.join(', ')}.`;
    setToast((x) => ({ text, team: { id: r.team.id, name: handle }, n: (x?.n ?? 0) + 1 }));
  };

  const value = useMemo<ReadyTeamsValue>(() => ({ space, templates, available, open, made: madeOf, undo }), [space, templates, available, open, madeOf, undo]);

  return (
    <Ctx.Provider value={value}>
      {children}
      {sheet && (
        <TeamPreviewSheet
          key={sheet.n}
          open
          space={space}
          mode={sheet.mode}
          templates={templates}
          templatesError={templatesError}
          onRetryTemplates={() => setLoadSeq((n) => n + 1)}
          takenNames={takenNames}
          takenHandles={takenHandles}
          spaceBots={bots}
          onClose={() => {
            const cb = sheet.opts.onCancel;
            setSheet(null);
            cb?.();
          }}
          onCreated={onCreated}
          onUseCoding={() => setSheet((s) => s && { mode: { kind: 'template', templateId: 'coding' }, opts: s.opts, n: s.n + 1 })}
          onConnectForge={(url, forge) => {
            if (forge === 'GitHub') openGitHubConnect({ space });
            else openForgeConnect({ space, forge: { forge: 'gitlab', url: `https://${url.replace(/^https?:\/\//, '').split('/')[0]}` } });
          }}
        />
      )}
      {toast && (
        <TeamCreatedToast
          key={toast.n}
          text={toast.text}
          onUndo={toast.team ? () => {
            const team = toast.team!;
            setToast(null);
            void undo(team);
          } : undefined}
          onDismiss={() => setToast(null)}
        />
      )}
      {confirm && (
        <UndoTeamConfirm
          label={confirm.label}
          handle={confirm.info.removes.handle || confirm.team.name}
          botNames={confirm.info.removes.bots}
          skills={confirm.info.removes.skills}
          routine={confirm.info.removes.routines > 0}
          messages={confirm.info.chat_messages}
          alsoIn={confirm.info.other_teams}
          editedSkills={confirm.info.edited_skills}
          busy={busy}
          error={undoError}
          onKeep={() => setConfirm(null)}
          onUndo={() => void runUndo(confirm.team, confirm.info.action_id, confirm.info.removes.handle || confirm.team.name)}
        />
      )}
    </Ctx.Provider>
  );
};

// ---------------------------------------------------------------------------------------------------
// The Needs card's live state (Q-T5): no polling and no model call. It reads once, then again when the
// window gets focus back or `refreshKey` changes (Settings closed, a connection or source changed).

const imapOf = (sources: LooseSource[]): NeedsContext['imap'] => {
  const imap = sources.filter((s) => s.kind === 'imap' && s.enabled);
  if (!imap.length) return { state: 'none' };
  const host = (s: LooseSource) => (typeof s.config?.host === 'string' ? (s.config.host as string) : s.label);
  const ok = imap.find((s) => s.last_scan_at && !s.last_error && s.last_status !== 'error');
  if (ok) return { state: 'ok', detail: typeof ok.config?.username === 'string' ? (ok.config.username as string) : host(ok) };
  const bad = imap.find((s) => s.last_error || s.last_status === 'error');
  if (bad) return { state: 'failed', detail: bad.last_error ? `Couldn’t sign in to ${host(bad)}. ${bad.last_error}` : null };
  return { state: 'checking', detail: host(imap[0]) };
};

export function useNeedsContext(space: string, teamId: string, canRun: boolean, personPosted: boolean, refreshKey: unknown): NeedsContext {
  const [imap, setImap] = useState<NeedsContext['imap']>({ state: 'none' });
  const [apps, setApps] = useState<string[]>([]);
  const dev = useDevMode(space);
  const [seq, setSeq] = useState(0);
  useEffect(() => {
    const f = () => setSeq((n) => n + 1);
    window.addEventListener('focus', f);
    return () => window.removeEventListener('focus', f);
  }, []);
  useEffect(() => {
    let live = true;
    looseApi.sources().then((r) => live && setImap(imapOf(r.sources ?? [])), () => undefined);
    connectionsApi.list().then(
      (r) => live && setApps((r.connections ?? []).filter((c) => c.status === 'connected').flatMap((c) => [c.name, String(c.kind)].map((s) => s.toLowerCase()))),
      () => undefined,
    );
    if (space) devApi.get(space).then((st) => devStore.set(space, st), () => undefined);
    return () => {
      live = false;
    };
  }, [space, refreshKey, seq]);
  const row = dev?.repos.find((r) => r.team_id === teamId);
  const repo: NeedsContext['repo'] = !row
    ? { state: 'none' }
    : row.status === 'watching'
      ? { state: 'watching', detail: `${row.repo}${row.branch ? ` · ${row.branch}` : ''}` }
      : row.status === 'error' || row.poll_error
        ? { state: 'failed', detail: row.poll_error }
        : { state: 'checking', detail: row.repo };
  return { canRun, imap, apps, repo, personPosted };
}

export interface TeamNeedsTopProps {
  team: ReadyTeam;
  canRun: boolean;
  /** The person has posted in this chat. */
  personPosted: boolean;
  refreshKey?: unknown;
  onAddAccount: () => void;
  /** `loose_ends_imap`, `connections` or `dev_mode_setup`. */
  onOpenNeed: (opens: string, need: TemplateNeed) => void;
}

/** The line and the Needs card at the top of a template team's chat; nothing for other teams. */
export const TeamNeedsTop: React.FC<TeamNeedsTopProps> = ({ team, canRun, personPosted, refreshKey, onAddAccount, onOpenNeed }) => {
  const rt = useReadyTeams();
  const m = rt?.made(team.id) ?? null;
  const t = m?.templateId ? rt?.templates?.find((x) => x.id === m.templateId) : undefined;
  const ctx = useNeedsContext(rt?.space ?? '', team.id, canRun, personPosted, refreshKey);
  if (!rt || !m || !t) return null;
  const undoable = Date.now() < (m.undoUntil || m.createdAt + 86_400_000);
  return (
    <div className="w-full max-w-3xl mx-auto space-y-2" data-team-needs-top>
      <TeamCreatedLine handle={m.handle} label={t.label} createdAt={m.createdAt} />
      <TeamNeedsCard
        template={t}
        handle={m.handle}
        teamId={team.id}
        createdAt={m.createdAt}
        ctx={ctx}
        onAddAccount={onAddAccount}
        onOpen={(n) => (n.opens === 'composer_prompt' ? askAbout(SELL_STARTER) : onOpenNeed(n.opens, n))}
        onFill={askAbout}
        onUndo={undoable ? () => rt.undo(team) : undefined}
      />
    </div>
  );
};
