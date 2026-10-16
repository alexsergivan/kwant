import React, { useEffect, useState } from 'react';
import { X, Sparkles, Pencil } from 'lucide-react';
import { ModernAvatar, AVATAR_STYLES } from './ModernAvatar';
import { AGENT_PRESETS, AgentPreset, uniqueAgentName } from '../agentPresets';
import { playSfx } from '../audio/chiptune';

export interface AgentFormData {
  name: string;
  title: string;
  persona_prompt: string;
  avatar_sprite: string;
  preferred_model: string;
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
  /** Resolves to an error message when saving failed. */
  onSubmit: (data: AgentFormData) => Promise<string | void> | void;
}

export const NewAgentModal: React.FC<NewAgentModalProps> = ({
  isOpen,
  onClose,
  initial,
  takenNames = [],
  onSubmit,
}) => {
  const [name, setName] = useState('');
  const [title, setTitle] = useState('');
  const [roleStyle, setRoleStyle] = useState('architect');
  const [model, setModel] = useState('auto');
  const [prompt, setPrompt] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [presetId, setPresetId] = useState<string | null>(null);
  const isEdit = !!initial;

  useEffect(() => {
    if (!isOpen) return;
    setName(initial?.name ?? '');
    setTitle(initial?.title ?? '');
    setRoleStyle(initial?.avatar_sprite || 'architect');
    setModel(initial?.preferred_model || 'auto');
    setPrompt(initial?.persona_prompt ?? '');
    setSubmitting(false);
    setError(null);
    setPresetId(null);
  }, [isOpen, initial]);

  const applyPreset = (preset: AgentPreset | null) => {
    playSfx('click');
    setPresetId(preset?.id ?? null);
    setError(null);
    setName(preset ? uniqueAgentName(preset.name, takenNames) : '');
    setTitle(preset?.title ?? '');
    setRoleStyle(preset?.avatar_sprite ?? 'architect');
    setModel(preset?.preferred_model ?? 'auto');
    setPrompt(preset?.persona_prompt ?? '');
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
      avatar_sprite: roleStyle,
      preferred_model: model,
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
      <div className="bg-zinc-900 border border-zinc-800 rounded-2xl max-w-lg w-full max-h-[92vh] overflow-y-auto p-6 text-zinc-100 shadow-2xl space-y-5">
        <div className="flex items-center justify-between border-b border-zinc-800 pb-3">
          <div className="flex items-center gap-2">
            <div className="w-7 h-7 rounded-lg bg-sky-500/10 border border-sky-500/20 flex items-center justify-center text-sky-400">
              {isEdit ? <Pencil size={14} /> : <Sparkles size={15} />}
            </div>
            <h3 className="text-sm font-semibold text-zinc-100">
              {isEdit ? `Edit ${initial?.name}` : 'Create New AI Agent'}
            </h3>
          </div>
          <button
            onClick={() => {
              playSfx('click');
              onClose();
            }}
            className="p-1 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 transition-colors"
          >
            <X size={17} />
          </button>
        </div>

        <form onSubmit={handleSubmit} className="space-y-4">
          {!isEdit && (
            <div>
              <label className="block text-xs font-medium text-zinc-400 mb-1">Start from a persona</label>
              <div className="grid grid-cols-3 gap-1.5">
                {[null, ...AGENT_PRESETS].map((preset) => {
                  const active = (preset?.id ?? null) === presetId;
                  return (
                    <button
                      key={preset?.id ?? 'blank'}
                      type="button"
                      onClick={() => applyPreset(preset)}
                      title={preset ? `${preset.title}: ${preset.persona_prompt}` : 'Start with empty fields'}
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
            <div className="grid grid-cols-4 gap-1.5">
              {AVATAR_STYLES.map((item) => {
                const active = roleStyle === item.id;
                return (
                  <button
                    key={item.id}
                    type="button"
                    onClick={() => setRoleStyle(item.id)}
                    className={`flex flex-col items-center gap-1 py-2 rounded-xl border text-[11px] transition-all ${
                      active
                        ? 'border-sky-500/60 bg-sky-500/10 text-sky-200 shadow-sm'
                        : 'border-zinc-800 bg-zinc-950/60 text-zinc-400 hover:border-zinc-700 hover:text-zinc-200'
                    }`}
                  >
                    <ModernAvatar role={item.id} size={32} />
                    <span className="truncate font-medium">{item.label}</span>
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

          <div>
            <label className="block text-xs font-medium text-zinc-400 mb-1">Persona & System Prompt</label>
            <textarea
              rows={isEdit || presetId ? 6 : 3}
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              placeholder="Define instructions, personality, tone, and domain expertise..."
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
              {isEdit ? 'Save Changes' : 'Create Agent'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
