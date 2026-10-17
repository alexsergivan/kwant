import React, { useState, useEffect, useRef } from 'react';
import { Cpu, Key, Globe, X, Sparkles, Check, Plug, ChevronRight, UserRound, LogOut, Bell, AlertTriangle } from 'lucide-react';
import {
  BROWSER_NOTIFY_KEY,
  MASK,
  NOTIFY_KINDS,
  NotifyKind,
  browserNotifyEnabled,
  notificationPermission,
  webhookUrlError,
} from '../continuity';
import { playSfx } from '../audio/chiptune';
import { useAuth } from './AuthGate';

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
  /** "••••••••" when one is saved, "" otherwise. */
  notify_webhook_url?: string;
  notify_on?: NotifyKind[];
}

interface SettingsModalProps {
  isOpen: boolean;
  onClose: () => void;
  onOpenTools?: () => void;
  onOpenProfiles?: () => void;
}

export const SettingsModal: React.FC<SettingsModalProps> = ({ isOpen, onClose, onOpenTools, onOpenProfiles }) => {
  const { authRequired, signOut } = useAuth();
  const [settings, setSettings] = useState<SettingsData | null>(null);
  const [anthropicKey, setAnthropicKey] = useState('');
  const [xaiKey, setXaiKey] = useState('');
  const [openaiKey, setOpenaiKey] = useState('');
  const [ollamaUrl, setOllamaUrl] = useState('http://localhost:11434/v1');
  const [saving, setSaving] = useState(false);
  const [saveSuccess, setSaveSuccess] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [webhookUrl, setWebhookUrl] = useState('');
  const [notifyOn, setNotifyOn] = useState<NotifyKind[]>(NOTIFY_KINDS.map((k) => k.id));
  const [browserNotify, setBrowserNotify] = useState(browserNotifyEnabled);
  const [permission, setPermission] = useState(notificationPermission);
  const webhookError = webhookUrlError(webhookUrl);

  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  useEffect(() => {
    if (!isOpen) return;
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && closeRef.current();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [isOpen]);

  useEffect(() => {
    if (isOpen) {
      setSaveError(null);
      setPermission(notificationPermission());
      setBrowserNotify(browserNotifyEnabled());
      fetch('/api/settings')
        .then((r) => r.json())
        .then((data: SettingsData) => {
          setSettings(data);
          setAnthropicKey(data.anthropic_api_key);
          setXaiKey(data.xai_api_key);
          setOpenaiKey(data.openai_api_key);
          setOllamaUrl(data.ollama_url || 'http://localhost:11434/v1');
          setWebhookUrl(data.notify_webhook_url ?? '');
          if (Array.isArray(data.notify_on)) setNotifyOn(data.notify_on);
        })
        .catch((err) => console.error('Failed to load settings:', err));
    }
  }, [isOpen]);

  if (!isOpen) return null;

  // Permission is only ever asked from this click, never on load.
  const toggleBrowserNotify = async () => {
    playSfx('click');
    if (browserNotify) {
      localStorage.setItem(BROWSER_NOTIFY_KEY, 'off');
      setBrowserNotify(false);
      return;
    }
    let p = notificationPermission();
    if (p === 'default') p = await Notification.requestPermission();
    setPermission(p);
    if (p === 'granted') {
      localStorage.setItem(BROWSER_NOTIFY_KEY, 'on');
      setBrowserNotify(true);
    }
  };

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    if (webhookError) return;
    setSaveError(null);
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
          notify_webhook_url: webhookUrl.trim(),
          notify_on: notifyOn,
        }),
      });

      if (res.ok) {
        setSaveSuccess(true);
        if (webhookUrl.trim() && webhookUrl.trim() !== MASK) setWebhookUrl(MASK);
        playSfx('coin');
        setTimeout(() => setSaveSuccess(false), 2500);
      } else {
        const data = await res.json().catch(() => null);
        setSaveError(data && typeof data.error === 'string' ? data.error : `Saving failed (HTTP ${res.status}).`);
      }
    } catch (err) {
      console.error('Failed to save settings:', err);
      setSaveError('Could not reach the server.');
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 bg-black/70 backdrop-blur-sm flex items-center justify-center p-4 select-none">
      <div role="dialog" aria-modal="true" aria-labelledby="settings-title" className="bg-zinc-900 border border-zinc-800 rounded-2xl max-w-lg w-full max-h-[92vh] overflow-y-auto p-4 sm:p-6 text-zinc-100 shadow-2xl space-y-6">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-zinc-800 pb-4">
          <div className="flex items-center gap-2.5">
            <div className="w-8 h-8 rounded-lg bg-sky-500/10 border border-sky-500/20 flex items-center justify-center text-sky-400">
              <Sparkles size={17} />
            </div>
            <div>
              <h2 id="settings-title" className="text-sm font-semibold text-zinc-100">Fleet & Engine Settings</h2>
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

        {onOpenProfiles && (
          <button
            type="button"
            onClick={() => {
              playSfx('click');
              onOpenProfiles();
            }}
            className="w-full flex items-center gap-3 p-3 rounded-xl border border-zinc-800 bg-zinc-950/60 hover:border-sky-500/40 text-left transition-colors group"
          >
            <span className="w-8 h-8 rounded-lg bg-sky-500/10 border border-sky-500/20 flex items-center justify-center text-sky-400">
              <UserRound size={15} />
            </span>
            <span className="flex-1">
              <span className="block text-xs font-medium text-zinc-200">LLM profiles</span>
              <span className="block text-[11px] text-zinc-500">Work and personal accounts per engine, with automatic failover</span>
            </span>
            <ChevronRight size={15} className="text-zinc-500 group-hover:text-sky-300" />
          </button>
        )}

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

          <fieldset className="space-y-3 pt-2 border-t border-zinc-800">
            <legend className="flex items-center gap-2 text-xs font-medium text-zinc-300 pt-3">
              <Bell size={14} className="text-amber-300" />
              <span>Notifications</span>
            </legend>

            <div className="flex items-start gap-3 bg-zinc-950/60 p-3 rounded-xl border border-zinc-800">
              <div className="flex-1">
                <p id="browser-notify-label" className="text-xs font-medium text-zinc-200">Notify me on this computer</p>
                <p className="text-[11px] text-zinc-500">
                  Only while kwnt is in a background tab.
                  {permission === 'denied' && <span className="text-amber-300"> Blocked by your browser; allow notifications for this site in its settings.</span>}
                  {permission === 'unsupported' && <span className="text-amber-300"> This browser can't show notifications.</span>}
                </p>
              </div>
              <button
                type="button"
                role="switch"
                aria-checked={browserNotify}
                aria-labelledby="browser-notify-label"
                disabled={permission === 'unsupported' || (permission === 'denied' && !browserNotify)}
                onClick={toggleBrowserNotify}
                className={`relative shrink-0 w-9 h-5 rounded-full transition-colors disabled:opacity-40 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500 ${
                  browserNotify ? 'bg-sky-600' : 'bg-zinc-700'
                }`}
              >
                <span
                  className={`absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white transition-transform motion-reduce:transition-none ${browserNotify ? 'translate-x-4' : ''}`}
                />
              </button>
            </div>

            <div>
              <label htmlFor="notify-webhook" className="block text-xs font-medium text-zinc-400 mb-1">
                Phone webhook (optional)
              </label>
              <div className="flex gap-1.5">
                <input
                  id="notify-webhook"
                  type={webhookUrl === MASK ? 'password' : 'url'}
                  value={webhookUrl}
                  onChange={(e) => setWebhookUrl(e.target.value)}
                  onFocus={() => webhookUrl === MASK && setWebhookUrl('')}
                  onBlur={() => !webhookUrl.trim() && settings?.notify_webhook_url === MASK && setWebhookUrl(MASK)}
                  placeholder="https://ntfy.sh/your-secret-topic"
                  aria-invalid={!!webhookError}
                  aria-describedby="notify-webhook-hint"
                  autoComplete="off"
                  spellCheck={false}
                  className={`flex-1 min-w-0 bg-zinc-950 border rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:outline-none transition-colors ${
                    webhookError ? 'border-rose-500/70' : 'border-zinc-800 focus:border-sky-500'
                  }`}
                />
                {settings?.notify_webhook_url === MASK && (
                  <button
                    type="button"
                    onClick={() => setWebhookUrl('')}
                    className="px-2.5 rounded-lg text-[11px] font-medium text-zinc-300 border border-zinc-800 hover:text-rose-200 hover:bg-zinc-800 transition-colors"
                  >
                    Remove
                  </button>
                )}
              </div>
              <p id="notify-webhook-hint" className={`mt-1 text-[11px] ${webhookError ? 'text-rose-400' : 'text-zinc-500'}`}>
                {webhookError ??
                  (webhookUrl === MASK
                    ? 'A webhook is saved. Type a new URL to replace it.'
                    : settings?.notify_webhook_url === MASK && !webhookUrl.trim()
                      ? 'Saving will remove the webhook.'
                      : 'https only. kwnt posts a short title and line, never message text.')}
              </p>
            </div>

            <div className="space-y-1.5" role="group" aria-labelledby="notify-kinds-label">
              <p id="notify-kinds-label" className="text-xs font-medium text-zinc-400">Tell me when</p>
              {NOTIFY_KINDS.map((k) => (
                <label key={k.id} className="flex items-center gap-2 text-[11px] text-zinc-300">
                  <input
                    type="checkbox"
                    className="accent-sky-500"
                    checked={notifyOn.includes(k.id)}
                    onChange={(e) => setNotifyOn((prev) => (e.target.checked ? [...prev, k.id] : prev.filter((x) => x !== k.id)))}
                  />
                  {k.label}
                </label>
              ))}
            </div>
          </fieldset>

          {saveError && (
            <div className="bg-rose-500/10 border border-rose-500/20 text-rose-300 px-3 py-2 rounded-lg text-xs flex items-center gap-2" role="alert">
              <AlertTriangle size={14} />
              <span>{saveError}</span>
            </div>
          )}

          {saveSuccess && (
            <div className="bg-emerald-500/10 border border-emerald-500/20 text-emerald-400 px-3 py-2 rounded-lg text-xs flex items-center gap-2">
              <Check size={14} />
              <span>Settings saved successfully to database</span>
            </div>
          )}

          <div className="flex items-center justify-end gap-2.5 pt-2 border-t border-zinc-800">
            {authRequired && (
              <button
                type="button"
                onClick={() => {
                  playSfx('click');
                  signOut();
                }}
                title="Forget the access token in this browser"
                className="mr-auto flex items-center gap-1.5 px-3 py-2 rounded-lg text-xs font-medium text-zinc-400 hover:text-rose-200 hover:bg-zinc-800 transition-colors"
              >
                <LogOut size={13} /> Sign out
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
              disabled={saving || !!webhookError}
              className="px-4 py-2 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 transition-colors shadow-sm disabled:opacity-50"
            >
              {saving ? 'Saving...' : 'Save Settings'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
