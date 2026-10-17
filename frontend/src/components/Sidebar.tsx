import React, { useState } from 'react';
import { Hash, Plus, CheckCircle2, Users, ChevronDown, Settings2, X, Crown, UserPlus, Pencil, Inbox } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import { playSfx } from '../audio/chiptune';
import { badgeText } from '../away';

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

export interface SidebarProps {
  kwants: Kwant[];
  channels: Channel[];
  teams: Team[];
  selectedId: string;
  isChannel: boolean;
  selectedTeamId?: string | null;
  /** Agents busy outside the open chat -> "Working on …", shown under their name. */
  busyKwants?: Record<string, string>;
  onSelectKwant: (kwant: Kwant) => void;
  onSelectChannel: (channel: Channel) => void;
  onSelectTeam: (team: Team) => void;
  onManageTeam?: (team: Team) => void;
  onUpdateTeamMembers?: (team: Team, memberIds: string[]) => void;
  onEditKwant?: (kwant: Kwant) => void;
  /** Opens the new-agent form; with a team, the new agent joins it. */
  onSummonKwant: (team?: Team) => void;
  onCreateTeam: () => void;
  /** "While you were away": unseen count for the badge, whether it's open, and how to open it. */
  awayCount?: number;
  awayActive?: boolean;
  onOpenAway?: () => void;
}

export const memberIdsOf = (t: Team): string[] =>
  Array.isArray(t.member_kwants)
    ? t.member_kwants
    : typeof t.member_kwants === 'string'
    ? (JSON.parse(t.member_kwants || '[]') as string[])
    : [];

export const Sidebar: React.FC<SidebarProps> = ({
  kwants,
  channels,
  teams = [],
  selectedId,
  isChannel,
  selectedTeamId,
  busyKwants = {},
  onSelectKwant,
  onSelectChannel,
  onSelectTeam,
  onManageTeam,
  onUpdateTeamMembers,
  onEditKwant,
  onSummonKwant,
  onCreateTeam,
  awayCount = 0,
  awayActive = false,
  onOpenAway,
}) => {
  const [teamsOpen, setTeamsOpen] = useState(true);
  const [fleetOpen, setFleetOpen] = useState(() => localStorage.getItem('kwant.fleetOpen') === '1');
  const [collapsedTeams, setCollapsedTeams] = useState<Set<string>>(new Set());
  const [addingTo, setAddingTo] = useState<string | null>(null);

  const toggleFleet = () => {
    const next = !fleetOpen;
    setFleetOpen(next);
    localStorage.setItem('kwant.fleetOpen', next ? '1' : '0');
  };

  const toggleTeam = (id: string) =>
    setCollapsedTeams((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  // Channels to display: hide auto-created team channels to avoid duplication with Teams section
  const displayChannels = channels.filter(
    (c) => !teams.some((t) => c.name === `team-${t.name}`)
  );

  const isTeamActive = (team: Team) => {
    if (selectedTeamId !== undefined && selectedTeamId !== null) {
      return selectedTeamId === team.id;
    }
    return (
      (isChannel && selectedId === team.id) ||
      channels.some((c) => c.id === selectedId && c.name === `team-${team.name}`)
    );
  };

  const isChannelActive = (c: Channel) => {
    if (!isChannel) return false;
    if (selectedTeamId && teams.some((t) => t.id === selectedTeamId)) {
      return false;
    }
    return selectedId === c.id;
  };

  return (
    <aside className="w-64 border-r border-zinc-800/80 bg-zinc-900/40 flex flex-col h-full select-none flex-shrink-0">
      {/* Top Header & New Agent Action */}
      <div className="p-3 border-b border-zinc-800/60 flex items-center justify-between">
        <span className="text-xs font-semibold uppercase tracking-wider text-zinc-400">Workspace</span>
        <button
          onClick={() => onSummonKwant()}
          className="flex items-center gap-1 px-2 py-1 text-xs font-medium text-sky-400 hover:text-sky-300 hover:bg-sky-500/10 rounded-md border border-sky-500/20 transition-all"
        >
          <Plus size={13} />
          <span>New Agent</span>
        </button>
      </div>

      <div className="flex-1 overflow-y-auto p-2 space-y-4">
        {onOpenAway && (
          <button
            type="button"
            onClick={onOpenAway}
            aria-current={awayActive ? 'page' : undefined}
            className={`w-full flex items-center gap-2.5 px-2.5 py-1.5 rounded-lg text-xs font-medium transition-all text-left focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500 ${
              awayActive
                ? 'bg-zinc-800 text-zinc-100 shadow-sm border border-zinc-700/50'
                : 'text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800/50 border border-transparent'
            }`}
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

        {/* Channels Section */}
        <div>
          <div className="px-2 py-1 text-[11px] font-semibold text-zinc-500 uppercase tracking-wider">
            Channels
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
                  <span className="truncate">{c.name}</span>
                </button>
              );
            })}
          </div>
        </div>

        {/* Teams Section */}
        <div>
          <div className="px-2 py-1 flex items-center justify-between text-[11px] font-semibold text-zinc-500 uppercase tracking-wider">
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
            <button
              type="button"
              onClick={onCreateTeam}
              className="flex items-center gap-1 text-[10px] font-medium text-sky-400 hover:text-sky-300 hover:bg-sky-500/10 px-1.5 py-0.5 rounded border border-sky-500/20 transition-all normal-case tracking-normal"
              title="Create New Team"
            >
              <Plus size={11} />
              <span>New Team</span>
            </button>
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
                            className="opacity-0 group-hover:opacity-100 p-1 hover:text-zinc-100 hover:bg-zinc-700/60 rounded transition-all text-zinc-400"
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
                              className={`group/member flex items-center gap-2 px-1.5 py-1 rounded-md cursor-pointer text-[11px] transition-colors ${
                                memberActive ? 'bg-zinc-800 text-zinc-100' : 'text-zinc-400 hover:bg-zinc-800/50 hover:text-zinc-200'
                              }`}
                              title={busyKwants[k.id] ? `Chat with ${k.name} · ${busyKwants[k.id]}` : `Chat with ${k.name}`}
                            >
                              <ModernAvatar role={k.avatar_sprite} name={k.name} size={18} kwantId={k.id} />
                              <span className="truncate flex-1">
                                {k.name}
                                {busyKwants[k.id] && <span className="sr-only"> ({busyKwants[k.id]})</span>}
                              </span>
                              {busyKwants[k.id] && (
                                <span className="w-1.5 h-1.5 shrink-0 rounded-full bg-sky-400 animate-pulse motion-reduce:animate-none" aria-hidden="true" />
                              )}
                              {onEditKwant && (
                                <button
                                  type="button"
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    onEditKwant(k);
                                  }}
                                  className="opacity-0 group-hover/member:opacity-100 p-0.5 rounded text-zinc-500 hover:text-sky-300 hover:bg-sky-500/10 transition-all"
                                  title={`Edit ${k.name}`}
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
                                    className="opacity-0 group-hover/member:opacity-100 p-0.5 rounded text-zinc-500 hover:text-rose-300 hover:bg-rose-500/10 transition-all"
                                    title={`Remove ${k.name} from @${t.name}`}
                                  >
                                    <X size={11} />
                                  </button>
                                )
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
                              <div className="px-1.5 py-1 text-[11px] text-zinc-500">Every agent is already in this team.</div>
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
                              <span>Create a new agent…</span>
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
                  No teams yet. Click + New Team to create.
                </div>
              )}
            </div>
          )}
        </div>

        {/* AI Agents Fleet Section */}
        <div>
          <button
            type="button"
            onClick={toggleFleet}
            className="w-full px-2 py-1 flex items-center justify-between text-[11px] font-semibold text-zinc-500 uppercase tracking-wider hover:text-zinc-300 transition-colors"
            title={fleetOpen ? 'Collapse agent fleet' : 'Show all agents'}
          >
            <span className="flex items-center gap-1">
              <ChevronDown size={12} className={`transition-transform duration-150 ${fleetOpen ? '' : '-rotate-90'}`} />
              <span>Agent Fleet ({kwants.length})</span>
            </span>
            {!fleetOpen && (
              <span className="flex items-center -space-x-1">
                {kwants.slice(0, 6).map((k) => (
                  <ModernAvatar key={k.id} role={k.avatar_sprite} name={k.name} size={16} kwantId={k.id} />
                ))}
                {kwants.length > 6 && (
                  <span className="pl-1.5 text-[10px] font-mono normal-case text-zinc-500">+{kwants.length - 6}</span>
                )}
              </span>
            )}
          </button>
          {fleetOpen && (
          <div className="mt-1 space-y-1">
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
                  <ModernAvatar
                    role={k.avatar_sprite}
                    name={k.name}
                    size={32}
                    isOnline={true}
                    kwantId={k.id}
                  />
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center justify-between gap-1">
                      <span className="text-xs font-semibold text-zinc-200 truncate">{k.name}</span>
                      {onEditKwant && (
                        <button
                          type="button"
                          onClick={(e) => {
                            e.stopPropagation();
                            onEditKwant(k);
                          }}
                          className="ml-auto opacity-0 group-hover/fleet:opacity-100 p-0.5 rounded text-zinc-500 hover:text-sky-300 hover:bg-sky-500/10 transition-all"
                          title={`Edit ${k.name}`}
                        >
                          <Pencil size={11} />
                        </button>
                      )}
                      <span className="text-[10px] text-zinc-500 font-mono uppercase bg-zinc-900 px-1 py-0.2 rounded border border-zinc-800">
                        {k.preferred_model === 'auto' ? 'Laya' : k.preferred_model.split('-')[0]}
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
                </div>
              );
            })}
          </div>
          )}
        </div>
      </div>

      {/* Footer Host Bridge status */}
      <div className="p-3 border-t border-zinc-800/60 bg-zinc-950/40 text-[11px] text-zinc-500 flex items-center justify-between">
        <span className="flex items-center gap-1.5">
          <CheckCircle2 size={13} className="text-emerald-400" />
          <span>Local CLI Fleet Active</span>
        </span>
      </div>
    </aside>
  );
};
