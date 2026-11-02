// API keys and the Ollama address: one per provider, shared by every space. They live in
// Settings › This space › Accounts, next to the accounts that use them, and are saved on their
// own: only these four fields are sent, so nothing else in App settings changes.
import React, { useEffect, useState } from 'react';
import { AlertTriangle, Check, ChevronDown, ChevronRight, Globe, Key, Loader2 } from 'lucide-react';
import { playSfx } from '../audio/chiptune';
import { editorProps } from '../escape';

const DEFAULT_OLLAMA = 'http://localhost:11434/v1';

interface Keys {
  anthropic_api_key: string;
  xai_api_key: string;
  openai_api_key: string;
  ollama_url: string;
}

const FIELDS: { key: Exclude<keyof Keys, 'ollama_url'>; label: string; placeholder: string }[] = [
  { key: 'anthropic_api_key', label: 'Anthropic API Key', placeholder: 'sk-ant-api03-...' },
  { key: 'xai_api_key', label: 'xAI Grok API Key', placeholder: 'xai-...' },
  { key: 'openai_api_key', label: 'OpenAI API Key', placeholder: 'sk-...' },
];

const input =
  'w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors';

export const ApiKeysPanel: React.FC<{ onSaved?: () => void; defaultOpen?: boolean }> = ({ onSaved, defaultOpen = false }) => {
  const [open, setOpen] = useState(defaultOpen);
  const [keys, setKeys] = useState<Keys | null>(null);
  // What's saved, so Escape keeps keys typed but not saved yet.
  const [loaded, setLoaded] = useState<Keys | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  const load = () => {
    setLoadError(null);
    fetch('/api/settings')
      .then((r) => {
        if (!r.ok) throw new Error(`HTTP ${r.status}`);
        return r.json();
      })
      .then((d: Partial<Keys>) => {
        const k = {
          anthropic_api_key: d.anthropic_api_key ?? '',
          xai_api_key: d.xai_api_key ?? '',
          openai_api_key: d.openai_api_key ?? '',
          ollama_url: d.ollama_url || DEFAULT_OLLAMA,
        };
        setKeys(k);
        setLoaded(k);
      })
      .catch(() => setLoadError('Couldn’t load your keys.'));
  };
  useEffect(load, []);

  const count = keys ? FIELDS.filter((f) => keys[f.key].trim()).length : 0;

  const save = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!keys || saving) return;
    playSfx('click');
    setSaving(true);
    setSaveError(null);
    try {
      const res = await fetch('/api/settings', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(keys),
      });
      if (!res.ok) {
        const data = await res.json().catch(() => null);
        throw new Error(data && typeof data.error === 'string' ? data.error : `Saving failed (HTTP ${res.status}).`);
      }
      playSfx('coin');
      setSaved(true);
      setTimeout(() => setSaved(false), 2500);
      onSaved?.();
      load();
    } catch (err) {
      setSaveError(err instanceof Error ? err.message : 'Could not reach the server.');
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="rounded-xl border border-zinc-800 bg-zinc-950/60" data-api-keys>
      <button
        type="button"
        aria-expanded={open}
        aria-controls="space-api-keys"
        onClick={() => setOpen((v) => !v)}
        className="w-full flex items-center gap-2 p-3 text-left rounded-xl focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400"
      >
        <Key size={14} className="text-violet-400" aria-hidden="true" />
        <span className="flex-1">
          <span className="block text-xs font-medium text-zinc-200">API keys and Ollama</span>
          <span className="block text-[11px] text-zinc-500">
            Optional, shared by every space. {count ? `${count} saved. ` : ''}Not needed when bots use a signed-in CLI.
          </span>
        </span>
        {open ? <ChevronDown size={15} className="text-zinc-500" aria-hidden="true" /> : <ChevronRight size={15} className="text-zinc-500" aria-hidden="true" />}
      </button>

      <form id="space-api-keys" hidden={!open} onSubmit={save} {...editorProps(JSON.stringify(keys) !== JSON.stringify(loaded))} className="space-y-3 px-3 pb-3">
        <p className="text-[11px] text-zinc-500">
          One key per provider for the API-key engines. To keep several keys for one provider, add them as accounts above.
        </p>
        {loadError ? (
          <p role="alert" className="flex items-center gap-2 text-xs text-rose-300">
            <AlertTriangle size={13} /> {loadError}
            <button type="button" onClick={load} className="underline hover:text-rose-100">
              Try again
            </button>
          </p>
        ) : !keys ? (
          <p className="flex items-center gap-2 text-xs text-zinc-400">
            <Loader2 size={13} className="animate-spin motion-reduce:animate-none" /> Loading…
          </p>
        ) : (
          <>
            {FIELDS.map((f) => (
              <div key={f.key}>
                <label htmlFor={`key-${f.key}`} className="block text-xs font-medium text-zinc-400 mb-1">
                  {f.label}
                </label>
                <input
                  id={`key-${f.key}`}
                  type="password"
                  autoComplete="off"
                  value={keys[f.key]}
                  onChange={(e) => setKeys({ ...keys, [f.key]: e.target.value })}
                  placeholder={f.placeholder}
                  className={input}
                />
              </div>
            ))}
            <div>
              <label htmlFor="key-ollama" className="mb-1 flex items-center gap-1.5 text-xs font-medium text-zinc-400">
                <Globe size={13} className="text-zinc-500" />
                <span>Ollama Base URL</span>
              </label>
              <input id="key-ollama" type="text" value={keys.ollama_url} onChange={(e) => setKeys({ ...keys, ollama_url: e.target.value })} placeholder={DEFAULT_OLLAMA} className={input} />
            </div>
            {saveError && (
              <p role="alert" className="flex items-center gap-2 rounded-lg border border-rose-500/20 bg-rose-500/10 px-3 py-2 text-xs text-rose-300">
                <AlertTriangle size={13} /> {saveError}
              </p>
            )}
            <div className="flex items-center justify-end gap-2">
              {saved && (
                <span role="status" className="flex items-center gap-1 text-xs text-emerald-400">
                  <Check size={13} /> Saved for every space
                </span>
              )}
              <button type="submit" disabled={saving} className="px-4 py-2 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 transition-colors disabled:opacity-50">
                {saving ? 'Saving…' : 'Save keys'}
              </button>
            </div>
          </>
        )}
      </form>
    </div>
  );
};
