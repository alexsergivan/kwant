import React, { useCallback, useEffect, useState } from 'react';
import { Hash, Plus, CheckCircle2, KeyRound, Loader2, Users, ChevronDown, Settings, Settings2, X, Crown, UserPlus, Pencil, Inbox, Search, ShieldOff, Split, Hand, MoreHorizontal, UserMinus, MessageSquare, KanbanSquare } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import { playSfx } from '../audio/chiptune';
import { badgeText } from '../away';
import { canRunNow, engineTip, isUsable } from '../engines';
import { statusOf, statusText, type BotStatus } from '../botStatus';
import { EnginesApi, useEngines } from '../useEngines';
import { claudeNeedsSignIn } from '../claudeLogin';
import { Space, spaceSummary } from '../spaces';
import { SpaceSwitcher, SpaceTile } from './Spaces';
import { isMac } from './LayaStatus';
import { ApprovalsOffBadge } from './ApprovalsSetting';
import { approvalsOff, spaceOffLine } from '../approvalMode';
import { teamChannelOf } from '../teamChannel';
import { layaRoutes } from '../channels';
import { InstallAppButton } from './InstallAppButton';

export interface Kwant {
  id: string;
  name: string;
  title: string;
  persona_prompt: string;
  soul?: string;
  avatar_sprite: string;
  driver_type: string;
  preferred_model: string;
  /** Pinned LLM profile; null uses the engine's default. */
  profile_id?: string | null;
  /** Whether turns may move to the engine's next profile when this one is out of usage. */
  profile_failover?: boolean;
  /** Its own approvals setting, and what it comes to with the space's; absent on older servers. */
  approvals?: 'inherit' | 'ask' | 'smart' | 'off';
  approvals_effective?: 'ask' | 'smart' | 'off';
}

export interface Channel {
  id: string;
  name: string;
  description?: string;
  lead_kwant_id: string;
  member_kwants: string[];
  routing_mode: string;
  /** Pinned for every turn in this channel; null when unset. */
  instructions?: string | null;
}

export interface Team {
  id: string;
  name: string;
  description?: string;
  workspace_path?: string;
  repo_url?: string;
  icon: string;
  lead_kwant_id: string;
  member_kwants: string[];
  /** Pinned for every turn in the team's channels; null when unset. */
  instructions?: string | null;
}

/** What the sidebar shows: everything, or one phone tab. */
export type SidebarSection = 'all' | 'chats' | 'bots';

export interface SidebarProps {
  space: Space;
  spaces: Space[];
  onSwitchSpace: (id: string) => void;
  onCreateSpace: () => void;
  onOpenSpaceSettings: () => void;
  onOpenArchived: () => void;
  onOpenSearch: () => void;
  /** Phones show one tab at a time, full width. */
  section?: SidebarSection;
  phone?: boolean;
  kwants: Kwant[];
  channels: Channel[];
  teams: Team[];
  selectedId: string;
  isChannel: boolean;
  selectedTeamId?: string | null;
  /** What busy bots are doing ("Working on …", "Working here"), shown under their name. */
  busyKwants?: Record<string, string>;
  /** Busy bots' status for the avatar dot; bots not listed are Available (or Can't run). */
  botStatus?: Record<string, BotStatus>;
  /** Open browser steps per bot ("Needs you" pill). */
  needsYou?: Record<string, number>;
  onSelectKwant: (kwant: Kwant) => void;
  onSelectChannel: (channel: Channel) => void;
  onSelectTeam: (team: Team) => void;
  onManageTeam?: (team: Team) => void;
  onUpdateTeamMembers?: (team: Team, memberIds: string[]) => void;
  onEditKwant?: (kwant: Kwant) => void;
  /** Opens the new-bot form; with a team, the new bot joins it. */
  onSummonKwant: (team?: Team) => void;
  onCreateTeam: () => void;
  /** Opens the new-channel form. */
  onCreateChannel?: () => void;
  /** Inbox: unseen count for the badge, whether it's open, and how to open it. */
  inboxCount?: number;
  inboxActive?: boolean;
  onOpenInbox?: () => void;
  /** Parallel Universe, behind "More". */
  parallelActive?: boolean;
  onOpenParallel?: () => void;
  /** Shows the Mission board row; the server's mission_board setting decides. */
  missionBoard?: boolean;
  missionsActive?: boolean;
  onOpenMissions?: () => void;
  /** Opens Space settings → Accounts from the footer. */
  onOpenAccounts?: () => void;
  /** Opens Accounts at "Sign in with Claude", when Claude being signed out is why nothing runs. */
  onSignInClaude?: () => void;
  /** Changes when Space settings closes, so the footer reads account status again. */
  enginesRefreshKey?: number;
  /** The space's engine checks when the app already loads them; the sidebar loads its own otherwise. */
  engines?: EnginesApi;
}

const navRow = (active: boolean) =>
  `w-full flex items-center gap-2.5 px-2.5 py-1.5 rounded-lg text-xs font-medium transition-all text-left focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500 ${
    active ? 'bg-zinc-800 text-zinc-100 shadow-sm border border-zinc-700/50' : 'text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800/50 border border-transparent'
  }`;

export const memberIdsOf = (t: Team): string[] =>
  Array.isArray(t.member_kwants)
    ? t.member_kwants
    : typeof t.member_kwants === 'string'
    ? (JSON.parse(t.member_kwants || '[]') as string[])
    : [];

/**
 * The bot list: one group per team (its members, lead first), then the bots in no team. A bot in
 * two teams shows under each, because that's where you look for it.
 */
export function botGroups(teams: Team[], kwants: Kwant[]): Array<{ team: Team | null; bots: Kwant[] }> {
  const byId = new Map(kwants.map((k) => [k.id, k]));
  const placed = new Set<string>();
  const groups: Array<{ team: Team | null; bots: Kwant[] }> = teams.map((t) => {
    const ids = memberIdsOf(t);
    const ordered = [...ids.filter((id) => id === t.lead_kwant_id), ...ids.filter((id) => id !== t.lead_kwant_id)];
    const bots = ordered.map((id) => byId.get(id)).filter((k): k is Kwant => !!k);
    for (const k of bots) placed.add(k.id);
    return { team: t, bots };
  });
  groups.push({ team: null, bots: kwants.filter((k) => !placed.has(k.id)) });
  return groups;
}

/** "More": the places used less often, in a small menu. */
const MoreMenu: React.FC<{
  className: string;
  active: boolean;
  items: Array<{ label: string; icon: React.ReactNode; active?: boolean; onClick: () => void }>;
}> = ({ className, active, items }) => {
  const [open, setOpen] = useState(false);
  const ref = React.useRef<HTMLDivElement>(null);
  const menuId = React.useId();
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.stopPropagation();
      setOpen(false);
      ref.current?.querySelector<HTMLElement>('button')?.focus();
    };
    const onDown = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && setOpen(false);
    window.addEventListener('keydown', onKey, true);
    window.addEventListener('mousedown', onDown);
    ref.current?.querySelector<HTMLElement>('[role=menuitem]')?.focus();
    return () => {
      window.removeEventListener('keydown', onKey, true);
      window.removeEventListener('mousedown', onDown);
    };
  }, [open]);
  return (
    <div ref={ref} className="relative" data-more-menu>
      <button
        type="button"
        onClick={() => {
          playSfx('click');
          setOpen((v) => !v);
        }}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        className={className}
      >
        <MoreHorizontal size={15} className={active ? 'text-sky-400' : 'text-zinc-500'} aria-hidden="true" />
        <span className="truncate flex-1">More</span>
        <ChevronDown size={12} className={`shrink-0 text-zinc-500 transition-transform ${open ? 'rotate-180' : ''}`} aria-hidden="true" />
      </button>
      {open && (
        <div id={menuId} role="menu" aria-label="More" className="absolute left-0 right-0 top-full mt-1 z-30 rounded-lg border border-zinc-700/70 bg-zinc-900 p-1 shadow-xl">
          {items.map((it) => (
            <button
              key={it.label}
              type="button"
              role="menuitem"
              onClick={() => {
                setOpen(false);
                it.onClick();
              }}
              aria-current={it.active ? 'page' : undefined}
              className={`w-full flex items-center gap-2.5 px-2.5 py-1.5 rounded-md text-xs font-medium text-left focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500 ${
                it.active ? 'bg-zinc-800 text-zinc-100' : 'text-zinc-300 hover:bg-zinc-800 hover:text-zinc-100'
              }`}
            >
              <span className={it.active ? 'text-sky-400' : 'text-zinc-500'}>{it.icon}</span>
              {it.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
};

export const Sidebar: React.FC<SidebarProps> = ({
  space,
  spaces,
  onSwitchSpace,
  onCreateSpace,
  onOpenSpaceSettings,
  onOpenArchived,
  onOpenSearch,
  section = 'all',
  phone = false,
  kwants,
  channels,
  teams = [],
  selectedId,
  isChannel,
  selectedTeamId,
  busyKwants = {},
  botStatus = {},
  needsYou = {},
  onSelectKwant,
  onSelectChannel,
  onSelectTeam,
  onManageTeam,
  onUpdateTeamMembers,
  onEditKwant,
  onSummonKwant,
  onCreateTeam,
  onCreateChannel,
  inboxCount = 0,
  inboxActive = false,
  onOpenInbox,
  parallelActive = false,
  onOpenParallel,
  missionBoard = false,
  missionsActive = false,
  onOpenMissions,
  onOpenAccounts,
  onSignInClaude,
  enginesRefreshKey = 0,
  engines: sharedEngines,
}) => {
  const ownEngines = useEngines(!sharedEngines);
  const engines = sharedEngines ?? ownEngines;
  const { reload: reloadEngines } = engines;
  useEffect(() => {
    if (enginesRefreshKey) reloadEngines();
  }, [enginesRefreshKey, reloadEngines]);
  const ready = engines.engines?.filter(isUsable) ?? null;
  const readyCount = ready?.length ?? null;
  const runsOn = ready?.map((e) => e.uses?.label ?? e.name) ?? [];
  // Nothing can run and Claude only needs signing in: the footer goes straight to that.
  const offerClaude = readyCount === 0 && !!onSignInClaude && claudeNeedsSignIn(engines.engines);
  const canRun = canRunNow(engines.engines);
  const [collapsedTeams, setCollapsedTeams] = useState<Set<string>>(new Set());
  const [addingTo, setAddingTo] = useState<string | null>(null);
  const [switcherOpen, setSwitcherOpen] = useState(false);
  /** Phones: the member whose actions sheet is open, and whether "Remove" is waiting for a confirm. */
  const [memberMenu, setMemberMenu] = useState<{ team: Team; kwant: Kwant; confirmRemove?: boolean } | null>(null);
  const closeSwitcher = useCallback(() => setSwitcherOpen(false), []);
  const showChats = section !== 'bots';
  const showBots = section !== 'chats';

  const toggleTeam = (id: string) =>
    setCollapsedTeams((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  // Channels to display: hide auto-created team channels to avoid duplication with Teams section
  const displayChannels = channels.filter(
    (c) => !teams.some((t) => teamChannelOf(channels, t)?.id === c.id)
  );

  const isTeamActive = (team: Team) => {
    if (selectedTeamId !== undefined && selectedTeamId !== null) {
      return selectedTeamId === team.id;
    }
    return (
      (isChannel && selectedId === team.id) ||
      (!!selectedId && teamChannelOf(channels, team)?.id === selectedId)
    );
  };

  const isChannelActive = (c: Channel) => {
    if (!isChannel) return false;
    if (selectedTeamId && teams.some((t) => t.id === selectedTeamId)) {
      return false;
    }
    return selectedId === c.id;
  };

  const offLine = spaceOffLine(space.approvals, kwants);
  // Phones: section titles stay pinned while their list scrolls, so they are never cut off
  const sectionHead = `px-2 py-1 flex items-center justify-between text-[11px] font-semibold text-zinc-500 uppercase tracking-wider${
    phone ? ' kw-sticky-head sticky top-0 z-10' : ''
  }`;
  // Phones: "New …" is a plain + icon (44px to tap) instead of a boxed button
  const phoneAdd = (label: string, onClick: () => void) => (
    <button
      type="button"
      onClick={onClick}
      aria-label={label}
      title={label}
      className="-mr-1 rounded-full text-sky-400 hover:text-sky-300 hover:bg-sky-500/10 active:bg-sky-500/15 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
    >
      <Plus size={18} aria-hidden="true" />
    </button>
  );

  // One row per bot: avatar with its status dot, name, what it is doing, and its actions. In a
  // team the lead is marked and others can be removed; on phones those sit in a sheet.
  const botRow = (k: Kwant, team?: Team) => {
    const active = !isChannel && selectedId === k.id;
    const status = statusOf(k.id, botStatus, canRun);
    const isLead = !!team && k.id === team.lead_kwant_id;
    const memberIds = team ? memberIdsOf(team) : [];
    return (
      <div
        key={`${team?.id ?? 'none'}:${k.id}`}
        role="button"
        tabIndex={0}
        onClick={() => onSelectKwant(k)}
        onKeyDown={(e) => e.key === 'Enter' && onSelectKwant(k)}
        data-bot-row={k.id}
        className={`group/fleet w-full flex items-center gap-2.5 px-2 py-1.5 rounded-xl text-left cursor-pointer transition-all ${
          active ? 'bg-zinc-800/90 text-zinc-100 shadow-sm border border-zinc-700/60' : 'text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800/40 border border-transparent'
        }`}
      >
        <span className="shrink-0" title={status === 'cant_run' ? "Can't run yet: no account works here" : statusText(k.name, status)}>
          <ModernAvatar role={k.avatar_sprite} name={k.name} size={28} status={status} kwantId={k.id} />
        </span>
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-1">
            {/* The full name; what it runs on is in the tooltip */}
            <span className="text-xs font-semibold text-zinc-200 truncate" title={`${k.name} · ${engineTip(k.preferred_model)}`}>
              {k.name}
            </span>
            {isLead && (
              <span className="shrink-0 flex items-center gap-0.5 text-[9px] uppercase tracking-wide text-amber-300/90" title={`Leads @${team!.name}`}>
                <Crown size={10} aria-hidden="true" />
                lead
              </span>
            )}
            {needsYou[k.id] > 0 && (
              <span
                className="shrink-0 inline-flex items-center gap-1 px-1.5 py-px rounded-full text-[10px] font-medium bg-amber-400/15 text-amber-200 border border-amber-400/30"
                title={`${k.name} needs you in the browser`}
              >
                <Hand size={10} aria-hidden="true" />
                Needs you{needsYou[k.id] > 1 ? ` · ${needsYou[k.id]}` : ''}
              </span>
            )}
            {approvalsOff(k) && <ApprovalsOffBadge compact title={`${k.name} acts without asking. Payments still ask you.`} />}
            {!phone && (
              <span className="ml-auto flex items-center gap-0.5">
                {onEditKwant && (
                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation();
                      onEditKwant(k);
                    }}
                    className="opacity-0 group-hover/fleet:opacity-100 focus-visible:opacity-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400 p-0.5 rounded text-zinc-500 hover:text-sky-300 hover:bg-sky-500/10 transition-all"
                    title={`Edit or delete ${k.name}`}
                    aria-label={`Edit or delete ${k.name}`}
                  >
                    <Pencil size={11} />
                  </button>
                )}
                {team && !isLead && onUpdateTeamMembers && (
                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation();
                      onUpdateTeamMembers(team, memberIds.filter((id) => id !== k.id));
                    }}
                    className="opacity-0 group-hover/fleet:opacity-100 focus-visible:opacity-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400 p-0.5 rounded text-zinc-500 hover:text-rose-300 hover:bg-rose-500/10 transition-all"
                    title={`Remove ${k.name} from @${team.name}`}
                    aria-label={`Remove ${k.name} from @${team.name}`}
                  >
                    <X size={11} />
                  </button>
                )}
              </span>
            )}
          </div>
          {busyKwants[k.id] ? (
            <p className="text-[11px] text-sky-300 truncate" title={busyKwants[k.id]}>
              {busyKwants[k.id]}
            </p>
          ) : (
            <p className="text-[11px] text-zinc-500 truncate">{k.title}</p>
          )}
        </div>
        {/* Phones: one button at the row's end; in a team it opens the member's actions */}
        {phone && team && (onEditKwant || (onUpdateTeamMembers && !isLead)) && (
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              playSfx('click');
              setMemberMenu({ team, kwant: k });
            }}
            aria-haspopup="dialog"
            aria-label={`More for ${k.name}`}
            className="-mr-1 shrink-0 rounded-lg text-zinc-500 hover:text-zinc-200 hover:bg-zinc-800 active:bg-zinc-800 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400"
          >
            <MoreHorizontal size={16} />
          </button>
        )}
        {phone && !team && onEditKwant && (
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              onEditKwant(k);
            }}
            className="shrink-0 -my-1 rounded-lg text-zinc-400 hover:text-sky-300 hover:bg-sky-500/10 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400"
            title={`Edit or delete ${k.name}`}
            aria-label={`Edit or delete ${k.name}`}
          >
            <Pencil size={16} />
          </button>
        )}
      </div>
    );
  };

  // A team's chat row: opens the team chat; collapses its members, opens its settings.
  const teamRow = (t: Team, withMembers: boolean) => {
    const active = isTeamActive(t);
    const count = memberIdsOf(t).length;
    const expanded = withMembers && !collapsedTeams.has(t.id);
    return (
      <div
        role="button"
        tabIndex={0}
        onClick={() => onSelectTeam(t)}
        onKeyDown={(e) => e.key === 'Enter' && onSelectTeam(t)}
        data-team-row={t.id}
        title={`Open the @${t.name} team chat`}
        className={`w-full flex items-center justify-between px-1.5 py-1.5 rounded-lg text-xs font-medium transition-all text-left group cursor-pointer ${
          active ? 'bg-zinc-800 text-zinc-100 shadow-sm border border-zinc-700/50' : 'text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800/50 border border-transparent'
        }`}
      >
        <div className="flex items-center gap-1.5 min-w-0">
          {withMembers && (
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                toggleTeam(t.id);
              }}
              aria-expanded={expanded}
              aria-label={expanded ? `Hide @${t.name} members` : `Show @${t.name} members`}
              className="p-0.5 rounded text-zinc-500 hover:text-zinc-200 hover:bg-zinc-700/60"
            >
              <ChevronDown size={12} className={`transition-transform duration-150 ${expanded ? '' : '-rotate-90'}`} />
            </button>
          )}
          <Users size={14} className={active ? 'text-sky-400' : 'text-zinc-500 group-hover:text-zinc-400'} aria-hidden="true" />
          <span className="truncate">@{t.name}</span>
        </div>
        <div className="flex items-center gap-1 flex-shrink-0">
          {(!expanded || !withMembers) && (
            <span className="text-[10px] px-1.5 py-0.5 rounded-full tabular-nums bg-zinc-800/80 text-zinc-400 border border-zinc-800" title={`${count} ${count === 1 ? 'member' : 'members'}`}>
              {count}
            </span>
          )}
          {onManageTeam && (
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                playSfx('click');
                onManageTeam(t);
              }}
              className="opacity-0 group-hover:opacity-100 focus-visible:opacity-100 max-md:opacity-100 p-1 hover:text-zinc-100 hover:bg-zinc-700/60 rounded transition-all text-zinc-400"
              title={`Team settings for @${t.name}`}
              aria-label={`Team settings for @${t.name}`}
            >
              <Settings2 size={12} />
            </button>
          )}
        </div>
      </div>
    );
  };

  const addMember = (t: Team) => {
    const memberIds = memberIdsOf(t);
    const candidates = kwants.filter((k) => !memberIds.includes(k.id));
    if (!onUpdateTeamMembers) return null;
    if (addingTo !== t.id) {
      return (
        <button
          type="button"
          onClick={() => {
            setAddingTo(t.id);
            playSfx('click');
          }}
          className="w-full flex items-center gap-2 px-2 py-1 rounded-md text-[11px] text-sky-400/90 hover:text-sky-300 hover:bg-sky-500/10 transition-colors"
        >
          <span className="w-[18px] h-[18px] flex items-center justify-center rounded border border-dashed border-sky-500/40">
            <Plus size={11} />
          </span>
          <span>Add member</span>
        </button>
      );
    }
    return (
      <div className="rounded-lg border border-zinc-700/60 bg-zinc-900/90 p-1 space-y-0.5">
        <div className="flex items-center justify-between px-1.5 pt-0.5 pb-1 text-[10px] uppercase tracking-wider text-zinc-500">
          <span>Add to @{t.name}</span>
          <button type="button" onClick={() => setAddingTo(null)} className="p-0.5 rounded hover:text-zinc-200 hover:bg-zinc-700/60" title="Cancel" aria-label="Cancel">
            <X size={11} />
          </button>
        </div>
        {candidates.map((k) => (
          <button
            key={k.id}
            type="button"
            onClick={() => {
              onUpdateTeamMembers(t, [...memberIds, k.id]);
              setAddingTo(null);
            }}
            className="w-full flex items-center gap-2 px-1.5 py-1 rounded-md text-[11px] text-zinc-300 hover:bg-sky-500/10 hover:text-zinc-100 text-left"
          >
            <ModernAvatar role={k.avatar_sprite} name={k.name} size={18} kwantId={k.id} />
            <span className="truncate flex-1">{k.name}</span>
            <span className="text-[10px] text-zinc-500 truncate max-w-[80px]">{k.title}</span>
          </button>
        ))}
        {candidates.length === 0 && <div className="px-1.5 py-1 text-[11px] text-zinc-500">Every bot is already in this team.</div>}
        <button
          type="button"
          onClick={() => {
            setAddingTo(null);
            onSummonKwant(t);
          }}
          className="w-full flex items-center gap-2 px-1.5 py-1 rounded-md text-[11px] text-sky-400 hover:bg-sky-500/10 border-t border-zinc-800 mt-0.5 pt-1.5"
        >
          <UserPlus size={13} />
          <span>Create a new bot…</span>
        </button>
      </div>
    );
  };

  const groups = botGroups(teams, kwants);
  const headButton = (label: string, title: string, onClick: () => void) =>
    phone ? (
      phoneAdd(title, onClick)
    ) : (
      <button
        type="button"
        onClick={onClick}
        className="flex items-center gap-1 text-[10px] font-medium text-sky-400 hover:text-sky-300 hover:bg-sky-500/10 px-1.5 py-0.5 rounded border border-sky-500/20 transition-all normal-case tracking-normal focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
        title={title}
      >
        <Plus size={11} />
        <span>{label}</span>
      </button>
    );

  return (
    <aside
      aria-label={`${space.name} chats and kwant bots`}
      className={`${phone ? 'w-full flex-1 min-h-0' : 'w-64 h-full border-r border-zinc-800/80'} bg-zinc-900/40 flex flex-col select-none flex-shrink-0`}
    >
      {/* The open space. Desktop switches spaces on the rail, so this is its name and settings;
          phones have no rail, so here it is their one switcher. */}
      <div className="relative p-2 border-b border-zinc-800/60">
        {phone ? (
          <button
            type="button"
            onClick={() => {
              playSfx('click');
              setSwitcherOpen((v) => !v);
            }}
            data-space-toggle
            aria-haspopup="menu"
            aria-expanded={switcherOpen}
            className="w-full flex items-center gap-2.5 px-1.5 py-1.5 rounded-lg text-left hover:bg-zinc-800/60 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
          >
            <SpaceTile space={space} size={32} />
            <span className="flex-1 min-w-0">
              <span className="block text-sm font-semibold text-zinc-100 truncate">{space.name}</span>
              <span className="block text-[11px] text-zinc-500 truncate">{spaceSummary(space.counts)}</span>
            </span>
            <ChevronDown size={14} className={`shrink-0 text-zinc-500 transition-transform ${switcherOpen ? 'rotate-180' : ''}`} aria-hidden="true" />
          </button>
        ) : (
          <div className="flex items-center gap-2.5 px-1.5 py-1.5" data-space-header>
            <SpaceTile space={space} size={32} />
            <span className="flex-1 min-w-0">
              <h1 className="text-sm font-semibold text-zinc-100 truncate">{space.name}</h1>
              <span className="block text-[11px] text-zinc-500 truncate">{spaceSummary(space.counts)}</span>
            </span>
            <button
              type="button"
              onClick={() => {
                playSfx('click');
                onOpenSpaceSettings();
              }}
              data-space-settings-open
              aria-label={`Settings for ${space.name}`}
              title={`Settings for ${space.name}`}
              className="shrink-0 p-1.5 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
            >
              <Settings size={15} aria-hidden="true" />
            </button>
          </div>
        )}
        {offLine && (
          <button
            type="button"
            onClick={onOpenSpaceSettings}
            title="Bots here act without asking, including deletions, publishing and sign-ins. Payments still ask you. Change it in Settings › This space."
            className="mt-1 w-full flex items-center gap-1.5 px-2 py-1 rounded-md border border-amber-500/30 bg-amber-500/10 text-[11px] font-medium text-amber-200 hover:bg-amber-500/20 focus-visible:outline focus-visible:outline-2 focus-visible:outline-amber-400"
          >
            <ShieldOff size={12} aria-hidden="true" className="shrink-0" />
            <span className="truncate">{offLine}</span>
          </button>
        )}
        {phone && switcherOpen && (
          <SpaceSwitcher
            spaces={spaces}
            currentId={space.id}
            onSwitch={onSwitchSpace}
            onCreate={onCreateSpace}
            onOpenSpaceSettings={onOpenSpaceSettings}
            onOpenArchived={onOpenArchived}
            onClose={closeSwitcher}
            sheet={phone}
          />
        )}
      </div>

      <div className="flex-1 overflow-y-auto p-2 space-y-4">
        {showChats && (
        <div className="space-y-0.5" data-utility-rows>
          <button type="button" onClick={onOpenSearch} className={navRow(false)}>
            <Search size={15} className="text-zinc-500" aria-hidden="true" />
            <span className="truncate flex-1">Search {space.name}</span>
            {!phone && <kbd className="shrink-0 text-[10px] font-sans text-zinc-500">{isMac ? '⌘K' : 'Ctrl K'}</kbd>}
          </button>
          {/* Phones have Inbox as a tab */}
          {onOpenInbox && !phone && (
          <button
            type="button"
            onClick={onOpenInbox}
            aria-current={inboxActive ? 'page' : undefined}
            className={navRow(inboxActive)}
          >
            <Inbox size={15} className={inboxActive ? 'text-sky-400' : 'text-zinc-500'} aria-hidden="true" />
            <span className="truncate flex-1">Inbox</span>
            {inboxCount > 0 && (
              <span className="shrink-0 min-w-[18px] px-1.5 py-px rounded-full bg-amber-500 text-zinc-950 text-[10px] font-semibold text-center">
                {badgeText(inboxCount)}
                <span className="sr-only"> new</span>
              </span>
            )}
          </button>
          )}
          {missionBoard && onOpenMissions && (
          <button type="button" onClick={onOpenMissions} aria-current={missionsActive ? 'page' : undefined} className={navRow(missionsActive)}>
            <KanbanSquare size={15} className={missionsActive ? 'text-sky-400' : 'text-zinc-500'} aria-hidden="true" />
            <span className="truncate flex-1">Mission board</span>
          </button>
          )}
          {onOpenParallel && (
            <MoreMenu
              className={navRow(parallelActive)}
              active={parallelActive}
              items={[{ label: 'Parallel Universe', icon: <Split size={15} aria-hidden="true" />, active: parallelActive, onClick: onOpenParallel }]}
            />
          )}
        </div>
        )}

        {/* Channels Section */}
        {showChats && (
        <div>
          <div className={sectionHead}>
            <span>Channels</span>
            {onCreateChannel && kwants.length > 0 && headButton('New channel', `New channel in ${space.name}`, onCreateChannel)}
          </div>
          <div className="mt-1 space-y-0.5">
            {displayChannels.map((c) => {
              const active = isChannelActive(c);
              return (
                <button
                  key={c.id}
                  onClick={() => onSelectChannel(c)}
                  // The channel's header says it too; no pill on every row
                  title={layaRoutes(c, kwants.length) ? 'Laya picks who answers here' : undefined}
                  className={`w-full flex items-center gap-2.5 px-2.5 py-1.5 rounded-lg text-xs font-medium transition-all text-left ${
                    active
                      ? 'bg-zinc-800 text-zinc-100 shadow-sm border border-zinc-700/50'
                      : 'text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800/50'
                  }`}
                >
                  <Hash size={15} className={active ? 'text-sky-400' : 'text-zinc-500'} />
                  <span className="truncate flex-1">{c.name}</span>
                </button>
              );
            })}
            {displayChannels.length === 0 && (
              <div className="px-2.5 py-1.5 text-[11px] text-zinc-500">#general opens once {space.name} has a bot.</div>
            )}
          </div>
        </div>
        )}

        {/* Phones' Chats tab: team chats are chats too; their members live on the Bots tab */}
        {section === 'chats' && teams.length > 0 && (
        <div>
          <div className={sectionHead}>
            <span>Team chats</span>
          </div>
          <div className="mt-1 space-y-0.5">
            {teams.map((t) => (
              <React.Fragment key={t.id}>{teamRow(t, false)}</React.Fragment>
            ))}
          </div>
        </div>
        )}

        {/* Every bot once, grouped by team; bots in no team last */}
        {showBots && (
        <div data-bot-list>
          <div className={sectionHead}>
            <h2>Bots ({kwants.length})</h2>
            <span className="flex items-center gap-1">
              {headButton('New team', `New team in ${space.name}`, onCreateTeam)}
              {headButton('New bot', `New bot in ${space.name}`, () => onSummonKwant())}
            </span>
          </div>
          <div className="mt-1 space-y-2">
            {kwants.length === 0 && teams.length === 0 && (
              <div className="px-2.5 py-1.5 text-[11px] text-zinc-500">No bots in {space.name} yet.</div>
            )}
            {groups.map((g) =>
              g.team ? (
                <div key={g.team.id} data-team-group={g.team.id}>
                  {teamRow(g.team, true)}
                  {!collapsedTeams.has(g.team.id) && (
                    <div className="ml-[13px] pl-1.5 border-l border-zinc-800 mt-0.5 space-y-0.5">
                      {g.bots.map((k) => botRow(k, g.team!))}
                      {addMember(g.team)}
                    </div>
                  )}
                </div>
              ) : (
                g.bots.length > 0 && (
                  <div key="no-team" data-team-group="none">
                    {teams.length > 0 && (
                      <div className="px-2 pt-1 pb-0.5 text-[10px] font-semibold uppercase tracking-wider text-zinc-500">No team</div>
                    )}
                    <div className="space-y-0.5">{g.bots.map((k) => botRow(k))}</div>
                  </div>
                )
              ),
            )}
          </div>
        </div>
        )}

        {/* Desktop: a one-time tip; Settings › All spaces keeps "Install app" for later */}
        {section === 'all' && <InstallAppButton tip className={navRow(false)} />}
      </div>

      {/* Footer: what this space's bots run on, from the real checks */}
      {/* On the phone's Chats tab only while nothing can run, so setup is never hidden there */}
      {engines.engines?.length !== 0 && (section !== 'chats' || canRun === false) ? (
        <div className="p-3 border-t border-zinc-800/60 bg-zinc-950/40 text-[11px] text-zinc-500">
          <button
            type="button"
            onClick={offerClaude ? onSignInClaude : onOpenAccounts}
            disabled={!onOpenAccounts && !offerClaude}
            className="w-full flex items-center gap-1.5 min-w-0 hover:text-zinc-300 disabled:hover:text-zinc-500 transition-colors"
            title={runsOn.length > 1 ? `Runs on ${runsOn.join(', ')}` : undefined}
          >
            {readyCount === null ? (
              <Loader2 size={13} className="text-sky-400 animate-spin motion-reduce:animate-none" aria-hidden="true" />
            ) : readyCount > 0 ? (
              <CheckCircle2 size={13} className="text-emerald-400" aria-hidden="true" />
            ) : (
              <KeyRound size={13} className="text-violet-300" aria-hidden="true" />
            )}
            {/* The prompt wraps, so its fix is never cut off */}
            <span className={readyCount === 0 ? 'text-left' : 'truncate'}>
              {readyCount === null
                ? 'Checking accounts…'
                : readyCount === 0
                  ? offerClaude
                    ? 'No account yet · Sign in with Claude'
                    : 'No account yet · Add an account'
                  : `Runs on ${runsOn[0]}${runsOn.length > 1 ? ` +${runsOn.length - 1}` : ''}`}
            </span>
          </button>
        </div>
      ) : null}
      {memberMenu && (
        <MemberSheet
          team={memberMenu.team}
          kwant={memberMenu.kwant}
          isLead={memberMenu.kwant.id === memberMenu.team.lead_kwant_id}
          confirmRemove={!!memberMenu.confirmRemove}
          onClose={() => setMemberMenu(null)}
          onChat={() => {
            setMemberMenu(null);
            onSelectKwant(memberMenu.kwant);
          }}
          onEdit={
            onEditKwant &&
            (() => {
              setMemberMenu(null);
              onEditKwant(memberMenu.kwant);
            })
          }
          onAskRemove={onUpdateTeamMembers && (() => setMemberMenu({ ...memberMenu, confirmRemove: true }))}
          onRemove={
            onUpdateTeamMembers &&
            (() => {
              const { team, kwant } = memberMenu;
              setMemberMenu(null);
              onUpdateTeamMembers(team, memberIdsOf(team).filter((id) => id !== kwant.id));
            })
          }
        />
      )}
    </aside>
  );
};

/** Phones: a team member's actions as a bottom sheet. Removing asks once more, away from the
 * other buttons, so a stray tap can't take a bot out of its team. */
const MemberSheet: React.FC<{
  team: Team;
  kwant: Kwant;
  isLead: boolean;
  confirmRemove: boolean;
  onClose: () => void;
  onChat: () => void;
  onEdit?: () => void;
  onAskRemove?: () => void;
  onRemove?: () => void;
}> = ({ team, kwant, isLead, confirmRemove, onClose, onChat, onEdit, onAskRemove, onRemove }) => {
  const ref = React.useRef<HTMLDivElement>(null);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);
  useEffect(() => {
    ref.current?.querySelector<HTMLElement>('button')?.focus();
  }, [confirmRemove]);
  const item = 'w-full flex items-center gap-3 px-3 py-2.5 rounded-lg text-left text-sm transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400';
  const titleId = `member-sheet-${kwant.id}`;
  return (
    <div className="fixed inset-0 z-50 bg-black/60 flex items-end" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="kw-sheet w-full max-h-[80vh] overflow-y-auto rounded-t-2xl bg-zinc-900 border-t border-zinc-800 p-3 shadow-2xl"
      >
        <div className="mx-auto mb-2 h-1 w-10 rounded-full bg-zinc-700" aria-hidden="true" />
        <div className="flex items-center gap-3 px-3 pb-3">
          <ModernAvatar role={kwant.avatar_sprite} name={kwant.name} size={32} kwantId={kwant.id} />
          <div className="min-w-0">
            <p id={titleId} className="text-sm font-semibold text-zinc-100 truncate">{kwant.name}</p>
            <p className="text-xs text-zinc-500 truncate">
              {isLead ? `Leads @${team.name}` : `In @${team.name}`}
              {kwant.title ? ` · ${kwant.title}` : ''}
            </p>
          </div>
        </div>
        {confirmRemove && onRemove ? (
          <div className="px-3 pb-1 space-y-3">
            <p className="text-sm text-zinc-300">
              Remove {kwant.name} from @{team.name}? The bot stays in this space and keeps its chats.
            </p>
            <div className="grid grid-cols-2 gap-2">
              <button type="button" onClick={onClose} className="rounded-lg border border-zinc-700 px-3 py-2 text-sm text-zinc-200 hover:bg-zinc-800">
                Cancel
              </button>
              <button type="button" onClick={onRemove} className="rounded-lg bg-rose-600 px-3 py-2 text-sm font-medium text-white hover:bg-rose-500">
                Remove
              </button>
            </div>
          </div>
        ) : (
          <div className="space-y-0.5">
            <button type="button" onClick={onChat} className={`${item} text-zinc-200 hover:bg-zinc-800`}>
              <MessageSquare size={16} className="text-zinc-400" aria-hidden="true" /> Chat with {kwant.name}
            </button>
            {onEdit && (
              <button type="button" onClick={onEdit} className={`${item} text-zinc-200 hover:bg-zinc-800`}>
                <Pencil size={16} className="text-zinc-400" aria-hidden="true" /> Edit or delete {kwant.name}
              </button>
            )}
            {onAskRemove && !isLead && (
              <>
                <div className="my-1 border-t border-zinc-800" />
                <button type="button" onClick={onAskRemove} className={`${item} text-rose-300 hover:bg-rose-500/10`}>
                  <UserMinus size={16} aria-hidden="true" /> Remove from @{team.name}
                </button>
              </>
            )}
            {isLead && (
              <p className="px-3 pt-1 text-xs text-zinc-500">The lead can&apos;t be removed. Pick another lead in team settings first.</p>
            )}
          </div>
        )}
      </div>
    </div>
  );
};
