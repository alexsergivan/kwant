import React, { useCallback, useEffect, useState } from 'react';
import { Hash, Plus, CheckCircle2, AlertTriangle, Loader2, Users, ChevronDown, Settings2, X, Crown, UserPlus, Pencil, Inbox, Search, CalendarClock, ShieldOff, ListChecks, Split, Hand, Plug, MoreHorizontal, UserMinus, MessageSquare, KanbanSquare } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import { playSfx } from '../audio/chiptune';
import { badgeText } from '../away';
import { canRunNow, isUsable, pickLabel } from '../engines';
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
  onOpenRoutines: () => void;
  /** Phones show one tab at a time, full width. */
  section?: SidebarSection;
  phone?: boolean;
  kwants: Kwant[];
  channels: Channel[];
  teams: Team[];
  selectedId: string;
  isChannel: boolean;
  selectedTeamId?: string | null;
  /** Bots busy outside the open chat -> "Working on …", shown under their name. */
  busyKwants?: Record<string, string>;
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
  /** "While you were away": unseen count for the badge, whether it's open, and how to open it. */
  awayCount?: number;
  awayActive?: boolean;
  looseActive?: boolean;
  connectionsActive?: boolean;
  onOpenConnections?: () => void;
  onOpenLooseEnds?: () => void;
  parallelActive?: boolean;
  onOpenParallel?: () => void;
  /** Shows the Mission board row; the server's mission_board setting decides. */
  missionBoard?: boolean;
  missionsActive?: boolean;
  onOpenMissions?: () => void;
  onOpenAway?: () => void;
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

export const Sidebar: React.FC<SidebarProps> = ({
  space,
  spaces,
  onSwitchSpace,
  onCreateSpace,
  onOpenSpaceSettings,
  onOpenArchived,
  onOpenSearch,
  onOpenRoutines,
  section = 'all',
  phone = false,
  kwants,
  channels,
  teams = [],
  selectedId,
  isChannel,
  selectedTeamId,
  busyKwants = {},
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
  awayCount = 0,
  awayActive = false,
  looseActive = false,
  connectionsActive = false,
  onOpenConnections,
  parallelActive = false,
  onOpenParallel,
  missionBoard = false,
  missionsActive = false,
  onOpenMissions,
  onOpenLooseEnds,
  onOpenAway,
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
  const [teamsOpen, setTeamsOpen] = useState(true);
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

  return (
    <aside
      aria-label={`${space.name} chats and kwant bots`}
      className={`${phone ? 'w-full flex-1 min-h-0' : 'w-64 h-full border-r border-zinc-800/80'} bg-zinc-900/40 flex flex-col select-none flex-shrink-0`}
    >
      {/* The open space: tap to switch, create or open its settings */}
      <div className="relative p-2 border-b border-zinc-800/60">
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
        {offLine && (
          <button
            type="button"
            onClick={onOpenSpaceSettings}
            title="Bots here run commands, file writes and tools without asking. Change it in Space settings."
            className="mt-1 w-full flex items-center gap-1.5 px-2 py-1 rounded-md border border-amber-500/30 bg-amber-500/10 text-[11px] font-medium text-amber-200 hover:bg-amber-500/20 focus-visible:outline focus-visible:outline-2 focus-visible:outline-amber-400"
          >
            <ShieldOff size={12} aria-hidden="true" className="shrink-0" />
            <span className="truncate">{offLine}</span>
          </button>
        )}
        {switcherOpen && (
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
        <div className="space-y-0.5">
          <button type="button" onClick={onOpenSearch} className={navRow(false)}>
            <Search size={15} className="text-zinc-500" aria-hidden="true" />
            <span className="truncate flex-1">Search {space.name}</span>
            {!phone && <kbd className="shrink-0 text-[10px] font-sans text-zinc-500">{isMac ? '⌘K' : 'Ctrl K'}</kbd>}
          </button>
          {onOpenAway && !phone && (
          <button
            type="button"
            onClick={onOpenAway}
            aria-current={awayActive ? 'page' : undefined}
            className={navRow(awayActive)}
          >
            <Inbox size={15} className={awayActive ? 'text-sky-400' : 'text-zinc-500'} aria-hidden="true" />
            <span className="truncate flex-1">While you were away</span>
            {awayCount > 0 && (
              <span className="shrink-0 min-w-[18px] px-1.5 py-px rounded-full bg-amber-500 text-zinc-950 text-[10px] font-semibold text-center">
                {badgeText(awayCount)}
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
          {onOpenLooseEnds && (
          <button type="button" onClick={onOpenLooseEnds} aria-current={looseActive ? 'page' : undefined} className={navRow(looseActive)}>
            <ListChecks size={15} className={looseActive ? 'text-sky-400' : 'text-zinc-500'} aria-hidden="true" />
            <span className="truncate flex-1">Loose ends</span>
          </button>
          )}
          {onOpenParallel && (
          <button type="button" onClick={onOpenParallel} aria-current={parallelActive ? 'page' : undefined} className={navRow(parallelActive)}>
            <Split size={15} className={parallelActive ? 'text-sky-400' : 'text-zinc-500'} aria-hidden="true" />
            <span className="truncate flex-1">Parallel Universe</span>
          </button>
          )}
          {onOpenConnections && (
          <button type="button" onClick={onOpenConnections} aria-current={connectionsActive ? 'page' : undefined} className={navRow(connectionsActive)}>
            <Plug size={15} className={connectionsActive ? 'text-sky-400' : 'text-zinc-500'} aria-hidden="true" />
            <span className="truncate flex-1">Connections</span>
          </button>
          )}
          <button type="button" onClick={onOpenRoutines} className={navRow(false)}>
            <CalendarClock size={15} className="text-zinc-500" aria-hidden="true" />
            <span className="truncate flex-1">Routines</span>
          </button>
          <InstallAppButton className={navRow(false)} />
        </div>
        )}

        {/* Channels Section */}
        {showChats && (
        <div>
          <div className={sectionHead}>
            <span>Channels</span>
            {onCreateChannel && kwants.length > 0 && (phone ? phoneAdd(`New channel in ${space.name}`, onCreateChannel) :
              <button
                type="button"
                onClick={onCreateChannel}
                className="flex items-center gap-1 text-[10px] font-medium text-sky-400 hover:text-sky-300 hover:bg-sky-500/10 px-1.5 py-0.5 rounded border border-sky-500/20 transition-all normal-case tracking-normal focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
                title={`New channel in ${space.name}`}
              >
                <Plus size={11} />
                <span>New channel</span>
              </button>
            )}
          </div>
          <div className="mt-1 space-y-0.5">
            {displayChannels.map((c) => {
              const active = isChannelActive(c);
              return (
                <button
                  key={c.id}
                  onClick={() => onSelectChannel(c)}
                  className={`w-full flex items-center gap-2.5 px-2.5 py-1.5 rounded-lg text-xs font-medium transition-all text-left ${
                    active
                      ? 'bg-zinc-800 text-zinc-100 shadow-sm border border-zinc-700/50'
                      : 'text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800/50'
                  }`}
                >
                  <Hash size={15} className={active ? 'text-sky-400' : 'text-zinc-500'} />
                  <span className="truncate flex-1">{c.name}</span>
                  {layaRoutes(c, kwants.length) && (
                    <span className="shrink-0 text-[10px] px-1.5 py-px rounded-full border border-violet-500/30 bg-violet-500/10 text-violet-200" title="Laya picks who answers here">
                      Laya
                    </span>
                  )}
                </button>
              );
            })}
            {displayChannels.length === 0 && (
              <div className="px-2.5 py-1.5 text-[11px] text-zinc-500">#general opens once {space.name} has a bot.</div>
            )}
          </div>
        </div>
        )}

        {/* Teams Section */}
        {showChats && (
        <div>
          <div className={sectionHead}>
            <button
              type="button"
              onClick={() => setTeamsOpen(!teamsOpen)}
              className="flex items-center gap-1 hover:text-zinc-300 transition-colors"
            >
              <ChevronDown
                size={12}
                className={`transition-transform duration-150 ${teamsOpen ? '' : '-rotate-90'}`}
              />
              <span>Teams ({teams.length})</span>
            </button>
            {phone ? phoneAdd(`New team in ${space.name}`, onCreateTeam) : (
            <button
              type="button"
              onClick={onCreateTeam}
              className="flex items-center gap-1 text-[10px] font-medium text-sky-400 hover:text-sky-300 hover:bg-sky-500/10 px-1.5 py-0.5 rounded border border-sky-500/20 transition-all normal-case tracking-normal"
              title={`New team in ${space.name}`}
            >
              <Plus size={11} />
              <span>New team</span>
            </button>
            )}
          </div>

          {teamsOpen && (
            <div className="mt-1 space-y-0.5">
              {teams.map((t) => {
                const active = isTeamActive(t);
                const memberIds = memberIdsOf(t);
                const members = memberIds
                  .map((id) => kwants.find((k) => k.id === id))
                  .filter((k): k is Kwant => !!k);
                const candidates = kwants.filter((k) => !memberIds.includes(k.id));
                const expanded = !collapsedTeams.has(t.id);
                const picking = addingTo === t.id;

                return (
                  <div key={t.id}>
                    <div
                      role="button"
                      tabIndex={0}
                      onClick={() => onSelectTeam(t)}
                      onKeyDown={(e) => e.key === 'Enter' && onSelectTeam(t)}
                      className={`w-full flex items-center justify-between px-1.5 py-1.5 rounded-lg text-xs font-medium transition-all text-left group cursor-pointer ${
                        active
                          ? 'bg-zinc-800 text-zinc-100 shadow-sm border border-zinc-700/50'
                          : 'text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800/50 border border-transparent'
                      }`}
                    >
                      <div className="flex items-center gap-1.5 min-w-0">
                        <button
                          type="button"
                          onClick={(e) => {
                            e.stopPropagation();
                            toggleTeam(t.id);
                          }}
                          className="p-0.5 rounded text-zinc-500 hover:text-zinc-200 hover:bg-zinc-700/60"
                          title={expanded ? 'Hide members' : 'Show members'}
                        >
                          <ChevronDown
                            size={12}
                            className={`transition-transform duration-150 ${expanded ? '' : '-rotate-90'}`}
                          />
                        </button>
                        <Users size={14} className={active ? 'text-sky-400' : 'text-zinc-500 group-hover:text-zinc-400'} />
                        <span className="truncate">@{t.name}</span>
                      </div>
                      <div className="flex items-center gap-1 flex-shrink-0">
                        {!expanded && (
                          <span className="text-[10px] px-1.5 py-0.5 rounded-full font-mono bg-zinc-800/80 text-zinc-400 border border-zinc-800">
                            {memberIds.length}
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
                            className="opacity-0 group-hover:opacity-100 max-md:opacity-100 p-1 hover:text-zinc-100 hover:bg-zinc-700/60 rounded transition-all text-zinc-400"
                            title={`Team settings for @${t.name}`}
                          >
                            <Settings2 size={12} />
                          </button>
                        )}
                      </div>
                    </div>

                    {expanded && (
                      <div className="ml-[18px] pl-2 border-l border-zinc-800 mt-0.5 mb-1.5 space-y-0.5">
                        {members.map((k) => {
                          const isLead = k.id === t.lead_kwant_id;
                          const memberActive = !isChannel && selectedId === k.id;
                          return (
                            <div
                              key={k.id}
                              role="button"
                              tabIndex={0}
                              onClick={() => onSelectKwant(k)}
                              onKeyDown={(e) => e.key === 'Enter' && onSelectKwant(k)}
                              className={`group/member flex items-center gap-2 px-1.5 ${phone ? 'py-0' : 'py-1'} rounded-md cursor-pointer text-[11px] transition-colors ${
                                memberActive ? 'bg-zinc-800 text-zinc-100' : 'text-zinc-400 hover:bg-zinc-800/50 hover:text-zinc-200'
                              }`}
                              title={busyKwants[k.id] ? `Chat with ${k.name} · ${busyKwants[k.id]}` : `Chat with ${k.name}`}
                            >
                              <ModernAvatar role={k.avatar_sprite} name={k.name} size={18} kwantId={k.id} />
                              <span className="truncate flex-1">
                                {k.name}
                                {busyKwants[k.id] && <span className="sr-only"> ({busyKwants[k.id]})</span>}
                              </span>
                              {needsYou[k.id] > 0 && (
                                <span className="shrink-0 inline-flex text-amber-300" role="img" aria-label="Needs you in the browser" title="Needs you in the browser">
                                  <Hand size={11} />
                                </span>
                              )}
                              {busyKwants[k.id] && (
                                <span className="w-1.5 h-1.5 shrink-0 rounded-full bg-sky-400 animate-pulse motion-reduce:animate-none" aria-hidden="true" />
                              )}
                              {phone ? (
                                <>
                                  {isLead && (
                                    <span className="shrink-0 flex items-center gap-0.5 text-[9px] uppercase tracking-wide text-amber-300/90" title="Team lead">
                                      <Crown size={10} aria-hidden="true" />
                                      lead
                                    </span>
                                  )}
                                  {(onEditKwant || (onUpdateTeamMembers && !isLead)) && (
                                    <button
                                      type="button"
                                      onClick={(e) => {
                                        e.stopPropagation();
                                        playSfx('click');
                                        setMemberMenu({ team: t, kwant: k });
                                      }}
                                      aria-haspopup="dialog"
                                      aria-label={`More for ${k.name}`}
                                      className="-mr-1 shrink-0 rounded-lg text-zinc-500 hover:text-zinc-200 hover:bg-zinc-800 active:bg-zinc-800 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400"
                                    >
                                      <MoreHorizontal size={16} />
                                    </button>
                                  )}
                                </>
                              ) : (
                                <>
                              {onEditKwant && (
                                <button
                                  type="button"
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    onEditKwant(k);
                                  }}
                                  className="opacity-0 group-hover/member:opacity-100 max-sm:opacity-100 focus-visible:opacity-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400 p-0.5 rounded text-zinc-500 hover:text-sky-300 hover:bg-sky-500/10 transition-all"
                                  title={`Edit or delete ${k.name}`}
                          aria-label={`Edit or delete ${k.name}`}
                                >
                                  <Pencil size={11} />
                                </button>
                              )}
                              {isLead ? (
                                <span className="flex items-center gap-0.5 text-[9px] uppercase tracking-wide text-amber-300/90" title="Team lead">
                                  <Crown size={10} />
                                  lead
                                </span>
                              ) : (
                                onUpdateTeamMembers && (
                                  <button
                                    type="button"
                                    onClick={(e) => {
                                      e.stopPropagation();
                                      onUpdateTeamMembers(t, memberIds.filter((id) => id !== k.id));
                                    }}
                                    className="opacity-0 group-hover/member:opacity-100 max-sm:opacity-100 focus-visible:opacity-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400 p-0.5 rounded text-zinc-500 hover:text-rose-300 hover:bg-rose-500/10 transition-all"
                                    title={`Remove ${k.name} from @${t.name}`}
                                  >
                                    <X size={11} />
                                  </button>
                                )
                              )}
                                </>
                              )}
                            </div>
                          );
                        })}

                        {onUpdateTeamMembers && !picking && (
                          <button
                            type="button"
                            onClick={() => {
                              setAddingTo(t.id);
                              playSfx('click');
                            }}
                            className="w-full flex items-center gap-2 px-1.5 py-1 rounded-md text-[11px] text-sky-400/90 hover:text-sky-300 hover:bg-sky-500/10 transition-colors"
                          >
                            <span className="w-[18px] h-[18px] flex items-center justify-center rounded border border-dashed border-sky-500/40">
                              <Plus size={11} />
                            </span>
                            <span>Add member</span>
                          </button>
                        )}

                        {onUpdateTeamMembers && picking && (
                          <div className="rounded-lg border border-zinc-700/60 bg-zinc-900/90 p-1 space-y-0.5">
                            <div className="flex items-center justify-between px-1.5 pt-0.5 pb-1 text-[10px] uppercase tracking-wider text-zinc-500">
                              <span>Add to @{t.name}</span>
                              <button
                                type="button"
                                onClick={() => setAddingTo(null)}
                                className="p-0.5 rounded hover:text-zinc-200 hover:bg-zinc-700/60"
                                title="Cancel"
                              >
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
                            {candidates.length === 0 && (
                              <div className="px-1.5 py-1 text-[11px] text-zinc-500">Every bot is already in this team.</div>
                            )}
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
                        )}
                      </div>
                    )}
                  </div>
                );
              })}
              {teams.length === 0 && (
                <div className="px-2.5 py-1.5 text-[11px] text-zinc-500 italic">
                  No teams in {space.name} yet.
                </div>
              )}
            </div>
          )}
        </div>

        )}

        {/* Every bot in this space */}
        {showBots && (
        <div>
          <div className={sectionHead}>
            <h2>Kwant bots ({kwants.length})</h2>
            {phone ? phoneAdd(`New bot in ${space.name}`, () => onSummonKwant()) : (
            <button
              type="button"
              onClick={() => onSummonKwant()}
              className="flex items-center gap-1 text-[10px] font-medium text-sky-400 hover:text-sky-300 hover:bg-sky-500/10 px-1.5 py-0.5 rounded border border-sky-500/20 transition-all normal-case tracking-normal"
              title={`New bot in ${space.name}`}
            >
              <Plus size={11} />
              <span>New bot</span>
            </button>
            )}
          </div>
          <div className="mt-1 space-y-1">
            {kwants.length === 0 && (
              <div className="px-2.5 py-1.5 text-[11px] text-zinc-500">No bots in {space.name} yet.</div>
            )}
            {kwants.map((k) => {
              const active = !isChannel && selectedId === k.id;
              return (
                <div
                  key={k.id}
                  role="button"
                  tabIndex={0}
                  onClick={() => onSelectKwant(k)}
                  onKeyDown={(e) => e.key === 'Enter' && onSelectKwant(k)}
                  className={`group/fleet w-full flex items-center gap-3 p-2 rounded-xl text-left cursor-pointer transition-all ${
                    active
                      ? 'bg-zinc-800/90 text-zinc-100 shadow-sm border border-zinc-700/60'
                      : 'text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800/40'
                  }`}
                >
                  <span className="shrink-0" title={canRun === false ? "Can't run yet: no account works here" : undefined}>
                    <ModernAvatar
                      role={k.avatar_sprite}
                      name={k.name}
                      size={32}
                      isOnline={canRun !== false}
                      kwantId={k.id}
                    />
                  </span>
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center justify-between gap-1">
                      <span className="text-xs font-semibold text-zinc-200 truncate">{k.name}</span>
                      {needsYou[k.id] > 0 && (
                        <span
                          className="shrink-0 inline-flex items-center gap-1 px-1.5 py-px rounded-full text-[10px] font-medium bg-amber-400/15 text-amber-200 border border-amber-400/30"
                          title={`${k.name} needs you in the browser`}
                        >
                          <Hand size={10} aria-hidden="true" />
                          Needs you{needsYou[k.id] > 1 ? ` · ${needsYou[k.id]}` : ''}
                        </span>
                      )}
                      {approvalsOff(k) && <ApprovalsOffBadge compact title={`${k.name} runs commands, file writes and tools without asking.`} />}
                      {onEditKwant && !phone && (
                        <button
                          type="button"
                          onClick={(e) => {
                            e.stopPropagation();
                            onEditKwant(k);
                          }}
                          className="ml-auto opacity-0 group-hover/fleet:opacity-100 max-sm:opacity-100 focus-visible:opacity-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400 p-0.5 rounded text-zinc-500 hover:text-sky-300 hover:bg-sky-500/10 transition-all"
                          title={`Edit or delete ${k.name}`}
                          aria-label={`Edit or delete ${k.name}`}
                        >
                          <Pencil size={11} />
                        </button>
                      )}
                      {/* Capped, so a long model id (Custom API) leaves room for the bot's name. */}
                      <span
                        className="max-w-[60%] truncate whitespace-nowrap text-[10px] text-zinc-500 font-mono uppercase bg-zinc-900 px-1 py-0.2 rounded border border-zinc-800"
                        title={k.preferred_model === 'auto' ? 'Laya' : pickLabel(k.preferred_model)}
                      >
                        {k.preferred_model === 'auto' ? 'Laya' : pickLabel(k.preferred_model)}
                      </span>
                    </div>
                    {busyKwants[k.id] ? (
                      <p className="text-[11px] text-sky-300 truncate mt-0.5" title={busyKwants[k.id]}>
                        {busyKwants[k.id]}
                      </p>
                    ) : (
                      <p className="text-[11px] text-zinc-400 truncate mt-0.5">{k.title}</p>
                    )}
                  </div>
                  {/* Phones: the edit button sits at the row's end so it doesn't stretch the name line */}
                  {onEditKwant && phone && (
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
            })}
          </div>
        </div>
        )}
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
              <AlertTriangle size={13} className="text-amber-400" aria-hidden="true" />
            )}
            {/* The warning wraps, so its fix is never cut off */}
            <span className={readyCount === 0 ? 'text-left' : 'truncate'}>
              {readyCount === null
                ? 'Checking accounts…'
                : readyCount === 0
                  ? offerClaude
                    ? 'Nothing can run here · Sign in with Claude'
                    : 'Nothing can run here · Add an account'
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
