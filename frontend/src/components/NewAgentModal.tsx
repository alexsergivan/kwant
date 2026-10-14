import React, { useState } from 'react';
import { X, Sparkles, Layers, Code2, Terminal, Shield } from 'lucide-react';
import { playSfx } from '../audio/chiptune';

interface NewAgentModalProps {
  isOpen: boolean;
  onClose: () => void;
  onCreateAgent: (data: {
    name: string;
    title: string;
    persona_prompt: string;
    avatar_sprite: string;
    preferred_model: string;
  }) => void;
}

export const NewAgentModal: React.FC<NewAgentModalProps> = ({
  isOpen,
  onClose,
  onCreateAgent,
}) => {
  const [name, setName] = useState('');
  const [title, setTitle] = useState('');
  const [roleStyle, setRoleStyle] = useState('architect');
  const [model, setModel] = useState('auto');
  const [prompt, setPrompt] = useState('');
  const [submitting, setSubmitting] = useState(false);

  if (!isOpen) return null;

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim()) return;

    setSubmitting(true);
    playSfx('click');

    onCreateAgent({
      name: name.trim(),
      title: title.trim() || 'AI Specialist',
      persona_prompt: prompt.trim() || 'You are an intelligent AI specialist ready to collaborate.',
      avatar_sprite: roleStyle,
      preferred_model: model,
    });

    setSubmitting(false);
    onClose();
  };

  return (
    <div className="fixed inset-0 z-50 bg-black/70 backdrop-blur-sm flex items-center justify-center p-4 select-none">
      <div className="bg-zinc-900 border border-zinc-800 rounded-2xl max-w-md w-full p-6 text-zinc-100 shadow-2xl space-y-5">
        <div className="flex items-center justify-between border-b border-zinc-800 pb-3">
          <div className="flex items-center gap-2">
            <div className="w-7 h-7 rounded-lg bg-sky-500/10 border border-sky-500/20 flex items-center justify-center text-sky-400">
              <Sparkles size={15} />
            </div>
            <h3 className="text-sm font-semibold text-zinc-100">Create New AI Agent</h3>
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
            <label className="block text-xs font-medium text-zinc-400 mb-1">Role Specialty</label>
            <div className="grid grid-cols-2 gap-2">
              {[
                { id: 'architect', label: 'Architecture', icon: Layers },
                { id: 'tech_lead', label: 'Coding & Prototyping', icon: Code2 },
                { id: 'devops', label: 'DevOps & Shell', icon: Terminal },
                { id: 'security', label: 'Security & Audit', icon: Shield },
              ].map((item) => {
                const Icon = item.icon;
                const active = roleStyle === item.id;
                return (
                  <button
                    key={item.id}
                    type="button"
                    onClick={() => setRoleStyle(item.id)}
                    className={`flex items-center gap-2 p-2 rounded-lg border text-left text-xs transition-all ${
                      active
                        ? 'border-sky-500/50 bg-sky-500/10 text-sky-300'
                        : 'border-zinc-800 bg-zinc-950/60 text-zinc-400 hover:border-zinc-700'
                    }`}
                  >
                    <Icon size={14} className={active ? 'text-sky-400' : 'text-zinc-500'} />
                    <span className="truncate">{item.label}</span>
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
              <option value="auto">Auto (Laya 33ms Decision Router - Recommended)</option>
              <option value="claude">Anthropic Claude (Deep Architecture)</option>
              <option value="grok">xAI Grok (Fast Technical Reasoning)</option>
              <option value="codex">OpenAI Codex (Coding & Scripts)</option>
              <option value="local_ollama">Local Ollama (Offline / Private)</option>
            </select>
          </div>

          <div>
            <label className="block text-xs font-medium text-zinc-400 mb-1">Persona & System Prompt</label>
            <textarea
              rows={3}
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              placeholder="Define instructions, personality, tone, and domain expertise..."
              className="w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors resize-none"
            />
          </div>

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
              Create Agent
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
