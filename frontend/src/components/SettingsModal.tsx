import React, { useState, useEffect } from 'react';
import { Cpu, Key, Globe, X, Sparkles, Check, Plug, ChevronRight } from 'lucide-react';
import { playSfx } from '../audio/chiptune';

interface SettingsData {
  anthropic_api_key: string;
  xai_api_key: string;
  openai_api_key: string;
  ollama_url: string;
  claude_cli_found: boolean;
  claude_cli_path: string | null;
  grok_cli_found: boolean;
  grok_cli_path: string | null;
  codex_cli_found: boolean;
  codex_cli_path: string | null;
}

interface SettingsModalProps {
  isOpen: boolean;
  onClose: () => void;
  onOpenTools?: () => void;
}

export const SettingsModal: React.FC<SettingsModalProps> = ({ isOpen, onClose, onOpenTools }) => {
  const [settings, setSettings] = useState<SettingsData | null>(null);
  const [anthropicKey, setAnthropicKey] = useState('');
  const [xaiKey, setXaiKey] = useState('');
  const [openaiKey, setOpenaiKey] = useState('');
  const [ollamaUrl, setOllamaUrl] = useState('http://localhost:11434/v1');
  const [saving, setSaving] = useState(false);
  const [saveSuccess, setSaveSuccess] = useState(false);

  useEffect(() => {
    if (isOpen) {
      fetch('/api/settings')
        .then((r) => r.json())
        .then((data: SettingsData) => {
          setSettings(data);
          setAnthropicKey(data.anthropic_api_key);
          setXaiKey(data.xai_api_key);
          setOpenaiKey(data.openai_api_key);
          setOllamaUrl(data.ollama_url || 'http://localhost:11434/v1');
        })
        .catch((err) => console.error('Failed to load settings:', err));
    }
  }, [isOpen]);

  if (!isOpen) return null;

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    setSaving(true);
    playSfx('click');

    try {
      const res = await fetch('/api/settings', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          anthropic_api_key: anthropicKey,
          xai_api_key: xaiKey,
          openai_api_key: openaiKey,
          ollama_url: ollamaUrl,
        }),
      });

      if (res.ok) {
        setSaveSuccess(true);
        playSfx('coin');
        setTimeout(() => setSaveSuccess(false), 2500);
      }
    } catch (err) {
      console.error('Failed to save settings:', err);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 bg-black/70 backdrop-blur-sm flex items-center justify-center p-4 select-none">
      <div className="bg-zinc-900 border border-zinc-800 rounded-2xl max-w-lg w-full max-h-[92vh] overflow-y-auto p-6 text-zinc-100 shadow-2xl space-y-6">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-zinc-800 pb-4">
          <div className="flex items-center gap-2.5">
            <div className="w-8 h-8 rounded-lg bg-sky-500/10 border border-sky-500/20 flex items-center justify-center text-sky-400">
              <Sparkles size={17} />
            </div>
            <div>
              <h2 className="text-sm font-semibold text-zinc-100">Fleet & Engine Settings</h2>
              <p className="text-xs text-zinc-400">Configure local CLI routing and cloud provider credentials</p>
            </div>
          </div>

          <button
            onClick={() => {
              playSfx('click');
              onClose();
            }}
            className="p-1.5 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 transition-colors"
          >
            <X size={18} />
          </button>
        </div>

        {/* Local CLI Auto-Detection Fleet */}
        <div className="bg-zinc-950/60 p-4 rounded-xl border border-zinc-800 space-y-3">
          <div className="flex items-center gap-2 text-xs font-medium text-zinc-300">
            <Cpu size={15} className="text-sky-400" />
            <span>Local Host CLI Engines (Auto-Detected)</span>
          </div>
          <p className="text-xs text-zinc-400 leading-relaxed">
            Tasks auto-route through your authenticated local terminal CLIs without requiring cloud API keys.
          </p>

          <div className="grid grid-cols-1 gap-2 pt-1">
            <div className="flex items-center justify-between bg-zinc-900/90 px-3 py-2 rounded-lg border border-zinc-800 text-xs">
              <div className="flex items-center gap-2">
                <span className={`w-2 h-2 rounded-full ${settings?.claude_cli_found ? 'bg-emerald-400 shadow-sm shadow-emerald-500/50' : 'bg-zinc-600'}`} />
                <span className="font-medium text-zinc-200">Claude Code CLI</span>
              </div>
              <span className="text-[11px] font-mono text-zinc-400">
                {settings?.claude_cli_found ? 'Connected' : 'Not installed'}
              </span>
            </div>

            <div className="flex items-center justify-between bg-zinc-900/90 px-3 py-2 rounded-lg border border-zinc-800 text-xs">
              <div className="flex items-center gap-2">
                <span className={`w-2 h-2 rounded-full ${settings?.grok_cli_found ? 'bg-emerald-400 shadow-sm shadow-emerald-500/50' : 'bg-zinc-600'}`} />
                <span className="font-medium text-zinc-200">xAI Grok CLI</span>
              </div>
              <span className="text-[11px] font-mono text-zinc-400">
                {settings?.grok_cli_found ? 'Connected' : 'Not installed'}
              </span>
            </div>

            <div className="flex items-center justify-between bg-zinc-900/90 px-3 py-2 rounded-lg border border-zinc-800 text-xs">
              <div className="flex items-center gap-2">
                <span className={`w-2 h-2 rounded-full ${settings?.codex_cli_found ? 'bg-emerald-400 shadow-sm shadow-emerald-500/50' : 'bg-zinc-600'}`} />
                <span className="font-medium text-zinc-200">OpenAI Codex CLI</span>
              </div>
              <span className="text-[11px] font-mono text-zinc-400">
                {settings?.codex_cli_found ? 'Connected' : 'Not installed'}
              </span>
            </div>
          </div>
        </div>

        {onOpenTools && (
          <button
            type="button"
            onClick={() => {
              playSfx('click');
              onOpenTools();
            }}
            className="w-full flex items-center gap-3 p-3 rounded-xl border border-zinc-800 bg-zinc-950/60 hover:border-sky-500/40 text-left transition-colors group"
          >
            <span className="w-8 h-8 rounded-lg bg-sky-500/10 border border-sky-500/20 flex items-center justify-center text-sky-400">
              <Plug size={15} />
            </span>
            <span className="flex-1">
              <span className="block text-xs font-medium text-zinc-200">Tools (MCP servers)</span>
              <span className="block text-[11px] text-zinc-500">Give kwants abilities like files, web pages or GitHub</span>
            </span>
            <ChevronRight size={15} className="text-zinc-500 group-hover:text-sky-300" />
          </button>
        )}

        {/* Cloud API Keys Form */}
        <form onSubmit={handleSave} className="space-y-4">
          <div className="flex items-center gap-2 text-xs font-medium text-zinc-300">
            <Key size={14} className="text-violet-400" />
            <span>Direct Cloud API Keys (Optional)</span>
          </div>

          <div className="space-y-3">
            <div>
              <label className="block text-xs font-medium text-zinc-400 mb-1">
                Anthropic API Key
              </label>
              <input
                type="password"
                value={anthropicKey}
                onChange={(e) => setAnthropicKey(e.target.value)}
                placeholder="sk-ant-api03-..."
                className="w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors"
              />
            </div>

            <div>
              <label className="block text-xs font-medium text-zinc-400 mb-1">
                xAI Grok API Key
              </label>
              <input
                type="password"
                value={xaiKey}
                onChange={(e) => setXaiKey(e.target.value)}
                placeholder="xai-..."
                className="w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors"
              />
            </div>

            <div>
              <label className="block text-xs font-medium text-zinc-400 mb-1">
                OpenAI API Key
              </label>
              <input
                type="password"
                value={openaiKey}
                onChange={(e) => setOpenaiKey(e.target.value)}
                placeholder="sk-..."
                className="w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors"
              />
            </div>

            <div>
              <label className="block text-xs font-medium text-zinc-400 mb-1 flex items-center gap-1.5">
                <Globe size={13} className="text-zinc-500" />
                <span>Ollama Base URL</span>
              </label>
              <input
                type="text"
                value={ollamaUrl}
                onChange={(e) => setOllamaUrl(e.target.value)}
                placeholder="http://localhost:11434/v1"
                className="w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors"
              />
            </div>
          </div>

          {saveSuccess && (
            <div className="bg-emerald-500/10 border border-emerald-500/20 text-emerald-400 px-3 py-2 rounded-lg text-xs flex items-center gap-2">
              <Check size={14} />
              <span>Settings saved successfully to database</span>
            </div>
          )}

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
              disabled={saving}
              className="px-4 py-2 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 transition-colors shadow-sm"
            >
              {saving ? 'Saving...' : 'Save Settings'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
