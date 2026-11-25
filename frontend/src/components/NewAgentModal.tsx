import React, { useEffect, useRef, useState } from 'react';
import { TagListInput } from './TagListInput';
import { X, Sparkles, Pencil, ChevronRight, Brain, CalendarClock, UserRound, ListChecks, Trash2, Gauge, Globe } from 'lucide-react';
import { ModernAvatar, AVATAR_STYLES } from './ModernAvatar';
import { AGENT_PRESETS, AgentPreset, PRESET_GROUPS, PresetGroup, uniqueAgentName } from '../agentPresets';
import { playSfx } from '../audio/chiptune';
import { KwantToolsPicker } from './KwantToolsPicker';
import { ModelPicker } from './ModelPicker';
import { useEngines } from '../useEngines';
import { McpServer, defaultKwantServerIds } from '../mcp';
import { ENGINES, Profile, engineOfModel, profileStatus, profilesForEngine } from '../profiles';
import { MemoryPanel } from './MemoryPanel';
import { RoutinesPanel } from './RoutinesPanel';
import { SkillsPanel } from './SkillsPanel';
import { UsagePanel } from './UsagePanel';
import { BotBudgetSection } from './BotBudget';
import type { Channel, Kwant } from './Sidebar';
import { BotApprovalsSetting } from './ApprovalsSetting';
import { FlightSchoolPanel } from './FlightSchoolPanel';
import { useEscapeLayer } from '../escape';
import { BrowserSites, FieldErrorKey, SITE_LIMITS, changedSites, fieldError, sitesOf, sitesSummary } from '../browserSites';
import { useFocusAfterRender } from '../focusAfterRender';
import { avatarForJob, hasCustomMoreOptions } from '../newBotForm';

export type EditTab = 'profile' | 'memory' | 'skills' | 'routines' | 'usage';
const EDIT_TABS: Array<{ id: EditTab; label: string; Icon: typeof Brain }> = [
  { id: 'profile', label: 'Profile', Icon: UserRound },
  { id: 'memory', label: 'Memory', Icon: Brain },
  { id: 'skills', label: 'Skills', Icon: ListChecks },
  { id: 'routines', label: 'Routines', Icon: CalendarClock },
  { id: 'usage', label: 'Usage', Icon: Gauge },
];

export interface AgentFormData {
  name: string;
  title: string;
  persona_prompt: string;
  soul?: string;
  avatar_sprite: string;
  preferred_model: string;
  /** MCP servers this kwant may use; undefined when they weren't loaded. */
  mcp_server_ids?: string[];
  /** Pinned LLM profile; null uses the engine's default. */
  profile_id?: string | null;
  profile_failover?: boolean;
  /** What routing sends it ("Good for") and keeps away from it ("Not for"). */
  covers?: string[];
  avoids?: string[];
  /** Which sites its own browser may open; absent on older servers. Sent only when changed. */
  browser_allowed_sites?: string[];
  browser_blocked_sites?: string[];
  browser_block_private_network?: boolean;
}

interface NewAgentModalProps {
  isOpen: boolean;
  onClose: () => void;
  /** When set, the modal edits this agent instead of creating a new one. */
  initial?: AgentFormData | null;
  /** Names already in use, so presets pick a free one. */
  takenNames?: string[];
  /** When creating from a team, the team the new agent will join. */
  joinTeamName?: string;
  /** The agent being edited, to load its tool servers. */
  kwantId?: string;
  /** Opens the MCP servers screen. */
  onManageServers?: () => void;
  /** Every LLM profile, for the profile picker. */
  profiles?: Profile[];
  /** Opens Settings → Engines & accounts. */
  onManageProfiles?: () => void;
  /** Opens Settings → Engines. */
  onOpenSettings?: () => void;
  /** Opens Settings at "Sign in with Claude". */
  onSignInClaude?: () => void;
  /** Changes when Settings closes, so engine status is read again. */
  enginesRefreshKey?: number;
  /** Resolves to an error message when saving failed. */
  onSubmit: (data: AgentFormData) => Promise<string | void> | void;
  /** For the Routines tab when editing. */
  kwants?: Kwant[];
  channels?: Channel[];
  /** Tab to open on (editing only), e.g. 'skills' from a "skill proposed" notification. */
  initialTab?: EditTab;
  /** Skills waiting for review, shown as a count on the Skills tab. */
  pendingSkills?: number;
  /** Editing only: opens the Delete bot confirm. */
  onRequestDelete?: () => void;
  /** Editing only: the approvals override saved; load bots again for the markers. */
  onApprovalsChanged?: () => void;
  /** Whether this bot runs tools without asking: the bot's effective value, or the space's for a new bot. */
  approvalsOff?: boolean;
  /** Opens a task on the Mission board (from a lesson on the Memory tab). Absent when the board is off. */
  onOpenTask?: (taskId: string) => void;
  /** Show it once was started from the Skills tab: close and open the bot's browser. Absent = no entry. */
  onShowOnce?: () => void;
  /** The page the bot's browser is on, for Show it once's Start at */
  browserUrl?: string;
}

type ProfileFields = { name: string; title: string; roleStyle: string; model: string; prompt: string; soul: string; profileId: string | null; failover: boolean; covers: string; avoids: string; sites: string };

export const NewAgentModal: React.FC<NewAgentModalProps> = ({
  isOpen,
  onClose,
  initial,
  takenNames = [],
  joinTeamName,
  kwantId,
  onManageServers,
  profiles = [],
  onManageProfiles,
  onOpenSettings,
  onSignInClaude,
  enginesRefreshKey,
  onSubmit,
  kwants = [],
  onShowOnce,
  browserUrl,
  channels = [],
  initialTab = 'profile',
  pendingSkills = 0,
  onRequestDelete,
  onApprovalsChanged,
  onOpenTask,
  approvalsOff = false,
}) => {
  const [tab, setTab] = useState<EditTab>('profile');
  const [name, setName] = useState('');
  const [title, setTitle] = useState('');
  const [roleStyle, setRoleStyle] = useState('default');
  // A new bot's avatar follows its job until the person (or a persona) picks one.
  const [avatarPinned, setAvatarPinned] = useState(false);
  // Everything past the name and the job waits under one closed "More options".
  const [moreOpen, setMoreOpen] = useState(false);
  const [model, setModel] = useState('auto');
  const [prompt, setPrompt] = useState('');
  const [soul, setSoul] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [presetId, setPresetId] = useState<string | null>(null);
  const [presetGroup, setPresetGroup] = useState<PresetGroup>('tech');
  const [mcpIds, setMcpIds] = useState<string[] | undefined>(undefined);
  const [profileId, setProfileId] = useState<string | null>(null);
  const [failover, setFailover] = useState(true);
  const [covers, setCovers] = useState<string[]>([]);
  const [avoids, setAvoids] = useState<string[]>([]);
  // The site rules as the bot has them (null: this server has none), and as edited here.
  const [sitesBase, setSitesBase] = useState<BrowserSites | null>(null);
  const [sites, setSites] = useState<BrowserSites | null>(null);
  const [sitesOpen, setSitesOpen] = useState(false);
  const focusAfterRender = useFocusAfterRender();
  // A 400 that names one list ("browser_allowed_sites: …"), shown next to that list.
  const [fieldErrors, setFieldErrors] = useState<Partial<Record<FieldErrorKey, string>>>({});
  // Follows the prop until the person changes this bot's approvals here.
  const [toolsRunFree, setToolsRunFree] = useState(approvalsOff);
  useEffect(() => setToolsRunFree(approvalsOff), [approvalsOff, isOpen]);
  const isEdit = !!initial;
  const engines = useEngines(isOpen);
  const { reload: reloadEngines } = engines;
  useEffect(() => {
    if (isOpen && enginesRefreshKey) reloadEngines();
  }, [isOpen, enginesRefreshKey, reloadEngines]);
  const engine = engineOfModel(model);
  const engineProfiles = profilesForEngine(profiles, engine);
  const avatar = avatarPinned ? roleStyle : avatarForJob(prompt, name);

  // The profile as it opened (or as a preset filled it in): Escape keeps the editor open while it differs.
  const [base, setBase] = useState<ProfileFields | null>(null);
  const current: ProfileFields = { name, title, roleStyle: avatar, model, prompt, soul, profileId, failover, covers: covers.join('\n'), avoids: avoids.join('\n'), sites: JSON.stringify(sites) };
  const unsaved = !!base && (Object.keys(base) as (keyof ProfileFields)[]).some((k) => base[k] !== current[k]);
  const rootRef = useRef<HTMLDivElement>(null);
  useEscapeLayer(rootRef, isOpen, onClose, {
    modal: true,
    unsaved,
    // Opened from a menu that has gone: back to the bot in the sidebar, or the composer.
    fallback: () =>
      [kwantId ? `[data-bot-row="${kwantId}"] button` : '', '[data-composer]']
        .map((sel) => (sel ? document.querySelector<HTMLElement>(sel) : null))
        .find((el) => el && !el.closest('[inert]')),
  });

  useEffect(() => {
    if (!isOpen) return;
    setBase({
      name: initial?.name ?? '',
      title: initial?.title ?? '',
      roleStyle: initial ? initial.avatar_sprite || 'architect' : 'default',
      model: initial?.preferred_model || 'auto',
      prompt: initial?.persona_prompt ?? '',
      soul: initial?.soul ?? '',
      profileId: initial?.profile_id ?? null,
      failover: initial?.profile_failover ?? true,
      covers: (initial?.covers ?? []).join('\n'),
      avoids: (initial?.avoids ?? []).join('\n'),
      sites: JSON.stringify(sitesOf(initial)),
    });
    setName(initial?.name ?? '');
    setTitle(initial?.title ?? '');
    setRoleStyle(initial ? initial.avatar_sprite || 'architect' : 'default');
    setAvatarPinned(!!initial);
    setMoreOpen(hasCustomMoreOptions(initial));
    setModel(initial?.preferred_model || 'auto');
    setPrompt(initial?.persona_prompt ?? '');
    setSoul(initial?.soul ?? '');
    setSubmitting(false);
    setError(null);
    setTab(initialTab);
    setPresetId(null);
    setMcpIds(undefined);
    setProfileId(initial?.profile_id ?? null);
    setFailover(initial?.profile_failover ?? true);
    setCovers(initial?.covers ?? []);
    setAvoids(initial?.avoids ?? []);
    setSitesBase(sitesOf(initial));
    setSites(sitesOf(initial));
    setSitesOpen(false);
    setFieldErrors({});
    let cancelled = false;
    // If a list can't load, the picker stays hidden and nothing is saved: saving an empty
    // list would switch every server off for this kwant.
    if (kwantId) {
      fetch(`/api/kwants/${kwantId}/mcp-servers`)
        .then((r) => (r.ok ? r.json() : Promise.reject()))
        .then((d) => !cancelled && Array.isArray(d.server_ids) && setMcpIds(d.server_ids))
        .catch(() => undefined);
    } else {
      // Every server starts on, like the backend default.
      fetch('/api/mcp/servers')
        .then((r) => (r.ok ? r.json() : Promise.reject()))
        .then((list: McpServer[]) => !cancelled && setMcpIds(defaultKwantServerIds(list)))
        .catch(() => undefined);
    }
    return () => {
      cancelled = true;
    };
  }, [isOpen, initial, kwantId]);

  const applyPreset = (preset: AgentPreset | null) => {
    playSfx('click');
    setPresetId(preset?.id ?? null);
    setError(null);
    const filled = {
      name: preset ? uniqueAgentName(preset.name, takenNames) : '',
      title: preset?.title ?? '',
      roleStyle: preset?.avatar_sprite ?? 'default',
      model: preset?.preferred_model ?? 'auto',
      prompt: preset?.persona_prompt ?? '',
      soul: preset?.soul ?? '',
    };
    setName(filled.name);
    setTitle(filled.title);
    setRoleStyle(filled.roleStyle);
    // A persona brings its own avatar; no persona hands the avatar back to the job.
    setAvatarPinned(!!preset);
    setModel(filled.model);
    setPrompt(filled.prompt);
    setSoul(filled.soul);
    setBase((b) => (b ? { ...b, ...filled } : b));
  };

  if (!isOpen) return null;

  const fieldInputId = (field: FieldErrorKey) => `agent-${field.replace(/_/g, '-')}`;
  const listProps = (field: FieldErrorKey, set: (next: string[]) => void) => ({
    inputId: fieldInputId(field),
    serverError: fieldErrors[field] ?? null,
    onChange: (next: string[]) => {
      set(next);
      if (fieldErrors[field]) setFieldErrors(({ [field]: _gone, ...rest }) => rest);
    },
  });

  // The profile form (and its sticky footer) shows unless an edit tab other than Profile is open.
  const showForm = !(isEdit && !!kwantId && tab !== 'profile');

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim()) return;
    if (/[\s@]/.test(name.trim())) {
      setError('Name cannot contain spaces or @ — teammates @mention it.');
      return;
    }

    setSubmitting(true);
    setError(null);
    setFieldErrors({});
    playSfx('click');

    const err = await onSubmit({
      name: name.trim(),
      title: title.trim() || 'AI Specialist',
      persona_prompt: prompt.trim() || 'You are an intelligent AI specialist ready to collaborate.',
      soul: soul.trim(),
      avatar_sprite: avatar,
      preferred_model: model,
      mcp_server_ids: mcpIds,
      // A pin only makes sense for one engine; auto and Ollama run on the defaults. Until the
      // profiles have loaded, the current pin is left alone.
      profile_id:
        profiles.length === 0 ? undefined : engine && engineProfiles.some((p) => p.id === profileId) ? profileId : null,
      profile_failover: failover,
      // Editing: only once loaded, since older servers leave them out. Creating: always.
      ...(!isEdit || initial?.covers ? { covers, avoids } : {}),
      ...(isEdit && sitesBase && sites ? changedSites(sitesBase, sites) : {}),
    });

    setSubmitting(false);
    if (err) {
      const named = fieldError(err);
      if (!named) {
        setError(err);
        return;
      }
      // Nothing was saved: show why next to the list, and take focus there.
      setFieldErrors({ [named.field]: named.text });
      // Every list a 400 can name lives under More options.
      setMoreOpen(true);
      setError('Nothing was saved. Fix the entry marked above, then save again.');
      if (named.field.startsWith('browser_')) setSitesOpen(true);
      focusAfterRender(() => document.getElementById(fieldInputId(named.field)));
      return;
    }
    onClose();
  };

  return (
    <div className="kw-dialog fixed inset-0 z-50 bg-black/70 backdrop-blur-sm flex items-center justify-center p-4 select-none">
      <div
        ref={rootRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby="agent-modal-title"
        className={`kw-dialog-panel outline-none bg-zinc-900 border border-zinc-800 rounded-2xl max-w-lg w-full max-h-[92vh] overflow-y-auto p-4 sm:p-6 text-zinc-100 shadow-2xl space-y-5 ${
          showForm ? '!pb-0' : ''
        }`}
      >
        <div className="flex items-center justify-between border-b border-zinc-800 pb-3">
          <div className="flex items-center gap-2">
            <div className="w-7 h-7 rounded-lg bg-sky-500/10 border border-sky-500/20 flex items-center justify-center text-sky-400">
              {isEdit ? <Pencil size={14} /> : <Sparkles size={15} />}
            </div>
            <h3 id="agent-modal-title" className="text-sm font-semibold text-zinc-100">
              {isEdit ? `Edit ${initial?.name}` : joinTeamName ? `New bot for @${joinTeamName}` : 'New bot'}
            </h3>
          </div>
          <button
            onClick={() => {
              playSfx('click');
              onClose();
            }}
            aria-label="Close"
            className="p-1 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 transition-colors"
          >
            <X size={17} />
          </button>
        </div>

        {isEdit && kwantId && (
          <div className="flex p-0.5 rounded-lg bg-zinc-950 border border-zinc-800" role="tablist" aria-label="Bot settings">
            {EDIT_TABS.map(({ id, label, Icon }) => (
              <button
                key={id}
                type="button"
                role="tab"
                id={`agent-tab-${id}`}
                aria-selected={tab === id}
                aria-controls={`agent-panel-${id}`}
                onClick={() => {
                  playSfx('click');
                  setTab(id);
                }}
                className={`flex-1 flex items-center justify-center gap-1.5 py-1.5 rounded-md text-[11px] font-medium transition-colors ${
                  tab === id ? 'bg-sky-500/15 text-sky-200' : 'text-zinc-500 hover:text-zinc-300'
                }`}
              >
                <Icon size={12} aria-hidden="true" /> {label}
                {id === 'skills' && pendingSkills > 0 && (
                  <span className="ml-0.5 px-1 min-w-[1rem] rounded-full bg-amber-500/25 text-amber-200 text-[10px]" aria-label={`${pendingSkills} to review`}>
                    {pendingSkills}
                  </span>
                )}
              </button>
            ))}
          </div>
        )}

        {isEdit && kwantId && tab === 'memory' && (
          <div role="tabpanel" id="agent-panel-memory" aria-labelledby="agent-tab-memory">
            <MemoryPanel kwantId={kwantId} kwantName={initial?.name ?? 'This bot'} onOpenTask={onOpenTask} />
          </div>
        )}
        {isEdit && kwantId && tab === 'skills' && (
          <div role="tabpanel" id="agent-panel-skills" aria-labelledby="agent-tab-skills">
            <SkillsPanel kwantId={kwantId} kwantName={initial?.name ?? 'This bot'} onShowOnceStarted={onShowOnce} browserUrl={browserUrl} />
          </div>
        )}
        {isEdit && kwantId && tab === 'routines' && (
          <div role="tabpanel" id="agent-panel-routines" aria-labelledby="agent-tab-routines">
            <RoutinesPanel kwantId={kwantId} kwants={kwants} channels={channels} />
          </div>
        )}
        {isEdit && kwantId && tab === 'usage' && (
          <div role="tabpanel" id="agent-panel-usage" aria-labelledby="agent-tab-usage" className="space-y-4">
            <BotBudgetSection kwantId={kwantId} kwantName={initial?.name ?? 'This bot'} />
            <UsagePanel kwantId={kwantId} kwantName={initial?.name ?? 'This bot'} />
          </div>
        )}

        <form onSubmit={handleSubmit} className="space-y-4" hidden={!showForm} role={isEdit && kwantId ? 'tabpanel' : undefined} id="agent-panel-profile">
          <div>
            <label htmlFor="agent-name" className="block text-xs font-medium text-zinc-400 mb-1">Name</label>
            <div className="flex items-center gap-2">
              <span className="flex-shrink-0" data-name-avatar>
                <ModernAvatar role={avatar} name={name} size={32} modelBadge={false} />
              </span>
              <input
                id="agent-name"
                type="text"
                required
                autoFocus={!isEdit}
                value={name}
                onChange={(e) => setName(e.target.value)}
                onKeyDown={(e) => {
                  // Enter before the job is written moves on to the job instead of creating a bot without one.
                  if (e.key === 'Enter' && !prompt.trim()) {
                    e.preventDefault();
                    document.getElementById('agent-job')?.focus();
                  }
                }}
                placeholder="e.g. SupportBot"
                className="min-w-0 flex-1 bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors"
              />
              <button
                type="button"
                aria-label="Change avatar"
                aria-controls="agent-more-options"
                onClick={() => {
                  playSfx('click');
                  setMoreOpen(true);
                  focusAfterRender(() => document.querySelector<HTMLElement>('[data-avatar-grid] [aria-pressed="true"]'));
                }}
                className="flex-shrink-0 px-2 min-h-[44px] sm:min-h-[36px] rounded-lg text-[11px] font-medium text-sky-300 hover:text-sky-200 hover:bg-zinc-800 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
              >
                Change
              </button>
            </div>
          </div>

          <div>
            <label htmlFor="agent-job" className="block text-xs font-medium text-zinc-400 mb-1">What should it do?</label>
            <textarea
              id="agent-job"
              rows={isEdit || presetId ? 6 : 3}
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              placeholder="e.g. Answer our support emails kindly, and pass anything about refunds to me."
              className="w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors resize-none leading-normal"
            />
          </div>

          <div className="rounded-lg border border-zinc-800 bg-zinc-950/40">
            <button
              type="button"
              aria-expanded={moreOpen}
              aria-controls="agent-more-options"
              onClick={() => {
                playSfx('click');
                setMoreOpen((o) => !o);
              }}
              className="w-full flex items-center gap-2 px-3 py-2 min-h-[44px] sm:min-h-[40px] rounded-lg text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-500"
              data-more-options
            >
              <span className="text-xs font-medium text-zinc-300">More options</span>
              <span className="min-w-0 flex-1 truncate text-[11px] text-zinc-500">
                · {isEdit ? 'title, avatar, AI and more' : 'persona, avatar, AI and more'}
              </span>
              <ChevronRight
                size={13}
                className={`flex-shrink-0 text-zinc-500 transition-transform motion-reduce:transition-none ${moreOpen ? 'rotate-90' : ''}`}
                aria-hidden="true"
              />
            </button>
            <div id="agent-more-options" hidden={!moreOpen} className="space-y-4 px-3 pb-3 pt-1">
              {!isEdit && (
                <div>
                  <div className="flex items-center justify-between mb-1.5">
                    <span id="agent-persona-label" className="text-xs font-medium text-zinc-400">
                      Start from a persona <span className="hidden sm:inline text-zinc-600 font-normal">· optional</span>
                    </span>
                    <div className="flex p-0.5 rounded-lg bg-zinc-950 border border-zinc-800" role="tablist">
                      {PRESET_GROUPS.map((g) => (
                        <button
                          key={g.id}
                          type="button"
                          role="tab"
                          aria-selected={presetGroup === g.id}
                          onClick={() => {
                            playSfx('click');
                            setPresetGroup(g.id);
                          }}
                          className={`px-2.5 py-0.5 rounded-md text-[11px] font-medium whitespace-nowrap transition-colors ${
                            presetGroup === g.id ? 'bg-sky-500/15 text-sky-200' : 'text-zinc-500 hover:text-zinc-300'
                          }`}
                        >
                          {g.label}
                        </button>
                      ))}
                    </div>
                  </div>
                  <div className="grid grid-cols-3 gap-1.5" role="group" aria-labelledby="agent-persona-label">
                    {AGENT_PRESETS.filter((p) => p.group === presetGroup).map((preset) => {
                      // No persona picked means a blank start; pressing the picked one again goes back to it.
                      const active = preset.id === presetId;
                      return (
                        <button
                          key={preset.id}
                          type="button"
                          aria-pressed={active}
                          onClick={() => applyPreset(active ? null : preset)}
                          title={`${preset.title}: ${preset.persona_prompt}\n\nPersonality: ${preset.soul}`}
                          className={`flex items-center gap-1.5 px-2 py-1.5 rounded-lg border text-left text-[11px] transition-all ${
                            active
                              ? 'border-sky-500/60 bg-sky-500/10 text-sky-200'
                              : 'border-zinc-800 bg-zinc-950/60 text-zinc-400 hover:border-zinc-700 hover:text-zinc-200'
                          }`}
                        >
                          <ModernAvatar role={preset.avatar_sprite} size={18} />
                          <span className="truncate font-medium">{preset.label}</span>
                        </button>
                      );
                    })}
                  </div>
                </div>
              )}

              <div>
                <label htmlFor="agent-title" className="block text-xs font-medium text-zinc-400 mb-1">Role & Title</label>
                <input
                  id="agent-title"
                  type="text"
                  value={title}
                  onChange={(e) => setTitle(e.target.value)}
                  placeholder="e.g. Senior Security Auditor"
                  className="w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors"
                />
              </div>

              <div>
                <span id="agent-avatar-label" className="block text-xs font-medium text-zinc-400 mb-1">Avatar</span>
                <div className="grid grid-cols-6 gap-1.5" role="group" aria-labelledby="agent-avatar-label" data-avatar-grid>
                  {AVATAR_STYLES.map((item) => {
                    const active = avatar === item.id;
                    return (
                      <button
                        key={item.id}
                        type="button"
                        aria-pressed={active}
                        onClick={() => {
                          setRoleStyle(item.id);
                          setAvatarPinned(true);
                        }}
                        title={item.label}
                        className={`flex flex-col items-center gap-1 py-1.5 px-0.5 min-w-0 rounded-xl border text-[10px] transition-all ${
                          active
                            ? 'border-sky-500/60 bg-sky-500/10 text-sky-200 shadow-sm'
                            : 'border-zinc-800 bg-zinc-950/60 text-zinc-400 hover:border-zinc-700 hover:text-zinc-200'
                        }`}
                      >
                        <ModernAvatar role={item.id} size={32} />
                        <span className="w-full truncate text-center font-medium">{item.label}</span>
                      </button>
                    );
                  })}
                </div>
              </div>

              <ModelPicker
                value={model}
                onChange={setModel}
                engines={engines.engines}
                onOpenSettings={onOpenSettings}
                onSignInClaude={onSignInClaude}
                profiles={profiles}
                profileId={profileId}
                onPickAccount={setProfileId}
              />

              {engine && (
                <div>
                  <div className="flex items-center justify-between mb-1">
                    <label htmlFor="agent-profile" className="text-xs font-medium text-zinc-400">
                      Account
                    </label>
                    {onManageProfiles && (
                      <button
                        type="button"
                        onClick={onManageProfiles}
                        className="flex items-center gap-0.5 text-[11px] text-zinc-500 hover:text-sky-300 transition-colors"
                      >
                        Manage accounts <ChevronRight size={11} />
                      </button>
                    )}
                  </div>
                  <select
                    id="agent-profile"
                    value={profileId ?? ''}
                    onChange={(e) => setProfileId(e.target.value || null)}
                    className="w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 focus:border-sky-500 focus:outline-none transition-colors"
                  >
                    <option value="">
                      {`${ENGINES.find((e) => e.id === engine)?.label} default${
                        engineProfiles[0]?.is_default ? ` (${engineProfiles[0].name})` : ''
                      }`}
                    </option>
                    {engineProfiles.map((p) => (
                      <option key={p.id} value={p.id}>
                        {`${p.name} · ${profileStatus(p).text}`}
                      </option>
                    ))}
                    {profileId && !engineProfiles.some((p) => p.id === profileId) && (
                      <option value={profileId}>From another AI (cleared on save)</option>
                    )}
                  </select>
                  <label className="mt-1.5 flex items-start gap-2 text-[11px] text-zinc-400">
                    <input type="checkbox" checked={failover} onChange={(e) => setFailover(e.target.checked)} className="mt-0.5 accent-sky-500" />
                    <span>Switch to this AI’s next account when this one runs out of usage or is signed out</span>
                  </label>
                </div>
              )}

              {isEdit && kwantId && (
                <BotApprovalsSetting
                  kwantId={kwantId}
                  kwantName={initial?.name ?? name}
                  onChanged={(s) => {
                    setToolsRunFree(s.approvals_effective === 'off');
                    onApprovalsChanged?.();
                  }}
                />
              )}

              {isEdit && kwantId && <FlightSchoolPanel kwantId={kwantId} kwantName={initial?.name ?? name} />}

              {mcpIds !== undefined && (
                <KwantToolsPicker model={model} selected={mcpIds} onChange={setMcpIds} onManageServers={onManageServers} approvalsOff={toolsRunFree} />
              )}

              {isEdit && sites && (
                <details
                  className="group rounded-lg border border-zinc-800 bg-zinc-950/40"
                  open={sitesOpen}
                  onToggle={(e) => setSitesOpen((e.currentTarget as HTMLDetailsElement).open)}
                  data-browser-sites
                >
                  <summary className="flex items-center gap-2 px-3 py-2 min-h-[40px] cursor-pointer select-none list-none [&::-webkit-details-marker]:hidden rounded-lg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-500">
                    <Globe size={13} className="flex-shrink-0 text-zinc-500" aria-hidden="true" />
                    <span className="text-xs font-medium text-zinc-400">Browser sites</span>
                    <span className="min-w-0 flex-1 truncate text-[11px] text-zinc-500" data-sites-summary>
                      · {sitesSummary(sites)}
                    </span>
                    <ChevronRight size={13} className="flex-shrink-0 text-zinc-500 transition-transform motion-reduce:transition-none group-open:rotate-90" aria-hidden="true" />
                  </summary>
                  <div className="space-y-3 px-3 pb-3 pt-1">
                    <p className="text-[11px] text-zinc-500 leading-snug">
                      Which sites this bot's own browser may open. <span className="font-mono text-zinc-400">example.com</span> includes
                      its subdomains; <span className="font-mono text-zinc-400">*.example.com</span> only the subdomains. Your own
                      browsing isn't limited.
                    </p>
                    <TagListInput
                      label="Allowed sites"
                      hint={sites.browser_allowed_sites.length ? 'only these' : 'empty: any site'}
                      placeholder="e.g. wikipedia.org — Enter to add"
                      value={sites.browser_allowed_sites}
                      limits={SITE_LIMITS}
                      {...listProps('browser_allowed_sites', (next) => setSites({ ...sites, browser_allowed_sites: next }))}
                    />
                    <TagListInput
                      label="Blocked sites"
                      hint="never these, even if allowed"
                      placeholder="e.g. facebook.com — Enter to add"
                      value={sites.browser_blocked_sites}
                      limits={SITE_LIMITS}
                      tone="bad"
                      {...listProps('browser_blocked_sites', (next) => setSites({ ...sites, browser_blocked_sites: next }))}
                    />
                    <label className="flex items-start gap-2 text-[11px] text-zinc-400">
                      <input
                        type="checkbox"
                        checked={sites.browser_block_private_network}
                        onChange={(e) => setSites({ ...sites, browser_block_private_network: e.target.checked })}
                        className="mt-0.5 accent-sky-500"
                      />
                      <span>
                        Block local and private network addresses
                        <span className="block text-zinc-500">localhost, 192.168.x.x, 10.x.x.x and names that point to them, such as your router</span>
                      </span>
                    </label>
                  </div>
                </details>
              )}

              <div>
                <label htmlFor="agent-soul" className="block text-xs font-medium text-zinc-400 mb-1">
                  Personality <span className="text-zinc-600 font-normal">· voice, values, quirks</span>
                </label>
                <textarea
                  id="agent-soul"
                  rows={isEdit || presetId ? 4 : 2}
                  value={soul}
                  onChange={(e) => setSoul(e.target.value)}
                  placeholder="e.g. Warm and witty, a little impatient with jargon, celebrates small wins..."
                  className="w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors resize-none leading-normal"
                />
              </div>

              {(!isEdit || initial?.covers) && (
                <div className="space-y-3">
                  <TagListInput
                    label="Good for"
                    hint="what to send this bot"
                    placeholder="e.g. taxes, invoices — Enter to add"
                    value={covers}
                    {...listProps('covers', setCovers)}
                  />
                  <TagListInput
                    label="Not for"
                    hint="what never to send it"
                    placeholder="e.g. legal advice — Enter to add"
                    value={avoids}
                    {...listProps('avoids', setAvoids)}
                    tone="bad"
                  />
                  {!isEdit && (
                    <p className="text-[11px] text-zinc-500 leading-snug">
                      Channels use these to pick who answers. Optional; you can change them later in the bot's settings.
                    </p>
                  )}
                </div>
              )}
            </div>
          </div>

          {isEdit && initial && name.trim() !== initial.name && (
            <p className="text-[11px] text-zinc-500">
              Renaming updates past messages; teammates will need to @mention the new name.
            </p>
          )}

          {error && <p className="text-[11px] text-rose-400">{error}</p>}

          <div
            // Sticks to the panel's bottom edge. The panel drops its own bottom padding while this form shows, so the
            // footer carries it (on a phone with the safe area) and nothing scrolls out beneath Create.
            className="sticky bottom-0 z-10 -mx-4 sm:-mx-6 px-4 sm:px-6 pt-3 pb-[calc(1rem+env(safe-area-inset-bottom))] sm:pb-6 bg-zinc-900 border-t border-zinc-800 flex flex-wrap items-center justify-end gap-2.5"
            data-form-footer
          >
            {isEdit && onRequestDelete && (
              <button
                type="button"
                onClick={() => {
                  playSfx('click');
                  onRequestDelete();
                }}
                className="mr-auto inline-flex items-center gap-1.5 px-3 py-2 rounded-lg text-xs font-medium text-rose-300 hover:text-rose-200 hover:bg-rose-500/10 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-rose-400"
              >
                <Trash2 size={13} aria-hidden="true" />
                Delete bot
              </button>
            )}
            <button
              type="button"
              onClick={() => {
                playSfx('click');
                onClose();
              }}
              className="px-3.5 py-2 rounded-lg text-xs font-medium text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800 transition-colors"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={submitting || !name.trim()}
              className="px-4 py-2 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-700 transition-colors shadow-sm disabled:opacity-50"
            >
              {isEdit ? 'Save changes' : joinTeamName ? 'Create and add to team' : 'Create bot'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
