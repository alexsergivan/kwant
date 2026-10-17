import React, { useEffect, useState } from 'react';
import { X, Sparkles, Pencil, ChevronRight, Brain, CalendarClock, UserRound } from 'lucide-react';
import { ModernAvatar, AVATAR_STYLES } from './ModernAvatar';
import { AGENT_PRESETS, AgentPreset, PRESET_GROUPS, PresetGroup, uniqueAgentName } from '../agentPresets';
import { playSfx } from '../audio/chiptune';
import { KwantToolsPicker } from './KwantToolsPicker';
import { McpServer, defaultKwantServerIds } from '../mcp';
import { ENGINES, Profile, engineOfModel, profileStatus, profilesForEngine } from '../profiles';
import { MemoryPanel } from './MemoryPanel';
import { RoutinesPanel } from './RoutinesPanel';
import type { Channel, Kwant } from './Sidebar';

type EditTab = 'profile' | 'memory' | 'routines';
const EDIT_TABS: Array<{ id: EditTab; label: string; Icon: typeof Brain }> = [
  { id: 'profile', label: 'Profile', Icon: UserRound },
  { id: 'memory', label: 'Memory', Icon: Brain },
  { id: 'routines', label: 'Routines', Icon: CalendarClock },
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
}

const MODEL_OPTIONS = [
  { id: 'auto', label: 'Auto (Laya 33ms Decision Router - Recommended)' },
  { id: 'claude', label: 'Anthropic Claude (Deep Architecture)' },
  { id: 'grok', label: 'xAI Grok (Fast Technical Reasoning)' },
  { id: 'codex', label: 'OpenAI Codex (Coding & Scripts)' },
  { id: 'local_ollama', label: 'Local Ollama (Offline / Private)' },
];

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
  /** Opens the LLM profiles screen. */
  onManageProfiles?: () => void;
  /** Resolves to an error message when saving failed. */
  onSubmit: (data: AgentFormData) => Promise<string | void> | void;
  /** For the Routines tab when editing. */
  kwants?: Kwant[];
  channels?: Channel[];
}

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
  onSubmit,
  kwants = [],
  channels = [],
}) => {
  const [tab, setTab] = useState<EditTab>('profile');
  const [name, setName] = useState('');
  const [title, setTitle] = useState('');
  const [roleStyle, setRoleStyle] = useState('architect');
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
  const isEdit = !!initial;
  const engine = engineOfModel(model);
  const engineProfiles = profilesForEngine(profiles, engine);

  useEffect(() => {
    if (!isOpen) return;
    setName(initial?.name ?? '');
    setTitle(initial?.title ?? '');
    setRoleStyle(initial?.avatar_sprite || 'architect');
    setModel(initial?.preferred_model || 'auto');
    setPrompt(initial?.persona_prompt ?? '');
    setSoul(initial?.soul ?? '');
    setSubmitting(false);
    setError(null);
    setTab('profile');
    setPresetId(null);
    setMcpIds(undefined);
    setProfileId(initial?.profile_id ?? null);
    setFailover(initial?.profile_failover ?? true);
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
    setName(preset ? uniqueAgentName(preset.name, takenNames) : '');
    setTitle(preset?.title ?? '');
    setRoleStyle(preset?.avatar_sprite ?? 'architect');
    setModel(preset?.preferred_model ?? 'auto');
    setPrompt(preset?.persona_prompt ?? '');
    setSoul(preset?.soul ?? '');
  };

  if (!isOpen) return null;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim()) return;
    if (/[\s@]/.test(name.trim())) {
      setError('Name cannot contain spaces or @ — teammates @mention it.');
      return;
    }

    setSubmitting(true);
    setError(null);
    playSfx('click');

    const err = await onSubmit({
      name: name.trim(),
      title: title.trim() || 'AI Specialist',
      persona_prompt: prompt.trim() || 'You are an intelligent AI specialist ready to collaborate.',
      soul: soul.trim(),
      avatar_sprite: roleStyle,
      preferred_model: model,
      mcp_server_ids: mcpIds,
      // A pin only makes sense for one engine; auto and Ollama run on the defaults. Until the
      // profiles have loaded, the current pin is left alone.
      profile_id:
        profiles.length === 0 ? undefined : engine && engineProfiles.some((p) => p.id === profileId) ? profileId : null,
      profile_failover: failover,
    });

    setSubmitting(false);
    if (err) {
      setError(err);
      return;
    }
    onClose();
  };

  const modelOptions = MODEL_OPTIONS.some((m) => m.id === model)
    ? MODEL_OPTIONS
    : [...MODEL_OPTIONS, { id: model, label: model }];

  return (
    <div className="fixed inset-0 z-50 bg-black/70 backdrop-blur-sm flex items-center justify-center p-4 select-none">
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="agent-modal-title"
        className="bg-zinc-900 border border-zinc-800 rounded-2xl max-w-lg w-full max-h-[92vh] overflow-y-auto p-4 sm:p-6 text-zinc-100 shadow-2xl space-y-5"
      >
        <div className="flex items-center justify-between border-b border-zinc-800 pb-3">
          <div className="flex items-center gap-2">
            <div className="w-7 h-7 rounded-lg bg-sky-500/10 border border-sky-500/20 flex items-center justify-center text-sky-400">
              {isEdit ? <Pencil size={14} /> : <Sparkles size={15} />}
            </div>
            <h3 id="agent-modal-title" className="text-sm font-semibold text-zinc-100">
              {isEdit ? `Edit ${initial?.name}` : joinTeamName ? `New Agent for @${joinTeamName}` : 'Create New AI Agent'}
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
          <div className="flex p-0.5 rounded-lg bg-zinc-950 border border-zinc-800" role="tablist" aria-label="Agent settings">
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
              </button>
            ))}
          </div>
        )}

        {isEdit && kwantId && tab === 'memory' && (
          <div role="tabpanel" id="agent-panel-memory" aria-labelledby="agent-tab-memory">
            <MemoryPanel kwantId={kwantId} kwantName={initial?.name ?? 'This kwant'} />
          </div>
        )}
        {isEdit && kwantId && tab === 'routines' && (
          <div role="tabpanel" id="agent-panel-routines" aria-labelledby="agent-tab-routines">
            <RoutinesPanel kwantId={kwantId} kwants={kwants} channels={channels} />
          </div>
        )}

        <form onSubmit={handleSubmit} className="space-y-4" hidden={isEdit && !!kwantId && tab !== 'profile'} role={isEdit && kwantId ? 'tabpanel' : undefined} id="agent-panel-profile">
          {!isEdit && (
            <div>
              <div className="flex items-center justify-between mb-1.5">
                <label className="text-xs font-medium text-zinc-400">Start from a persona</label>
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
                      className={`px-2.5 py-0.5 rounded-md text-[11px] font-medium transition-colors ${
                        presetGroup === g.id ? 'bg-sky-500/15 text-sky-200' : 'text-zinc-500 hover:text-zinc-300'
                      }`}
                    >
                      {g.label}
                    </button>
                  ))}
                </div>
              </div>
              <div className="grid grid-cols-3 gap-1.5">
                {[null, ...AGENT_PRESETS.filter((p) => p.group === presetGroup)].map((preset) => {
                  const active = (preset?.id ?? null) === presetId;
                  return (
                    <button
                      key={preset?.id ?? 'blank'}
                      type="button"
                      onClick={() => applyPreset(preset)}
                      title={preset ? `${preset.title}: ${preset.persona_prompt}\n\nSoul: ${preset.soul}` : 'Start with empty fields'}
                      className={`flex items-center gap-1.5 px-2 py-1.5 rounded-lg border text-left text-[11px] transition-all ${
                        active
                          ? 'border-sky-500/60 bg-sky-500/10 text-sky-200'
                          : 'border-zinc-800 bg-zinc-950/60 text-zinc-400 hover:border-zinc-700 hover:text-zinc-200'
                      }`}
                    >
                      {preset ? (
                        <ModernAvatar role={preset.avatar_sprite} size={18} />
                      ) : (
                        <Sparkles size={13} className="mx-0.5 text-zinc-500" />
                      )}
                      <span className="truncate font-medium">{preset ? preset.label : 'Blank'}</span>
                    </button>
                  );
                })}
              </div>
            </div>
          )}

          <div>
            <label className="block text-xs font-medium text-zinc-400 mb-1">Agent Name</label>
            <input
              type="text"
              required
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. DataAnalyst, SecurityAuditor"
              className="w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors"
            />
          </div>

          <div>
            <label className="block text-xs font-medium text-zinc-400 mb-1">Role & Title</label>
            <input
              type="text"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="e.g. Senior Security Auditor"
              className="w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors"
            />
          </div>

          <div>
            <label className="block text-xs font-medium text-zinc-400 mb-1">Avatar & Role Specialty</label>
            <div className="grid grid-cols-6 gap-1.5">
              {AVATAR_STYLES.map((item) => {
                const active = roleStyle === item.id;
                return (
                  <button
                    key={item.id}
                    type="button"
                    onClick={() => setRoleStyle(item.id)}
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

          <div>
            <label className="block text-xs font-medium text-zinc-400 mb-1">Model Engine</label>
            <select
              value={model}
              onChange={(e) => setModel(e.target.value)}
              className="w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 focus:border-sky-500 focus:outline-none transition-colors"
            >
              {modelOptions.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.label}
                </option>
              ))}
            </select>
          </div>

          {engine && (
            <div>
              <div className="flex items-center justify-between mb-1">
                <label htmlFor="agent-profile" className="text-xs font-medium text-zinc-400">
                  Account (LLM profile)
                </label>
                {onManageProfiles && (
                  <button
                    type="button"
                    onClick={onManageProfiles}
                    className="flex items-center gap-0.5 text-[11px] text-zinc-500 hover:text-sky-300 transition-colors"
                  >
                    Manage profiles <ChevronRight size={11} />
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
                  <option value={profileId}>Another engine's profile (cleared on save)</option>
                )}
              </select>
              <label className="mt-1.5 flex items-start gap-2 text-[11px] text-zinc-400">
                <input type="checkbox" checked={failover} onChange={(e) => setFailover(e.target.checked)} className="mt-0.5 accent-sky-500" />
                <span>Switch to the engine's next profile when this one runs out of usage or is signed out</span>
              </label>
            </div>
          )}

          {mcpIds !== undefined && (
            <KwantToolsPicker model={model} selected={mcpIds} onChange={setMcpIds} onManageServers={onManageServers} />
          )}

          <div>
            <label className="block text-xs font-medium text-zinc-400 mb-1">Persona & Instructions</label>
            <textarea
              rows={isEdit || presetId ? 6 : 3}
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              placeholder="What this agent does: expertise, responsibilities, and how it should work..."
              className="w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors resize-none"
            />
          </div>

          <div>
            <label className="block text-xs font-medium text-zinc-400 mb-1">
              Soul <span className="text-zinc-600 font-normal">· personality, voice, values, quirks</span>
            </label>
            <textarea
              rows={isEdit || presetId ? 4 : 2}
              value={soul}
              onChange={(e) => setSoul(e.target.value)}
              placeholder="e.g. Warm and witty, a little impatient with jargon, celebrates small wins..."
              className="w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors resize-none"
            />
          </div>

          {isEdit && initial && name.trim() !== initial.name && (
            <p className="text-[11px] text-zinc-500">
              Renaming updates past messages; teammates will need to @mention the new name.
            </p>
          )}

          {error && <p className="text-[11px] text-rose-400">{error}</p>}

          <div className="flex items-center justify-end gap-2.5 pt-2 border-t border-zinc-800">
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
              className="px-4 py-2 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 transition-colors shadow-sm disabled:opacity-50"
            >
              {isEdit ? 'Save Changes' : joinTeamName ? 'Create & Add to Team' : 'Create Agent'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
