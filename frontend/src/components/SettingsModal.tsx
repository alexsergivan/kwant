import React, { useState, useEffect, useRef } from 'react';
import { Key, Globe, X, Settings, Check, ChevronDown, ChevronRight, LogOut, Bell, AlertTriangle, Layers, Monitor, Volume2, VolumeX, Gauge, MapPin, Vibrate, VibrateOff } from 'lucide-react';
import { TURN_LIMITS, TurnLimitKey, limitError } from '../threads';
import { budgetError, budgetValue } from '../chainBudget';
import { DailyUsagePanel } from './DailyUsage';
import {
  BROWSER_NOTIFY_KEY,
  MASK,
  MAX_LOCATION_CHARS,
  NOTIFY_KINDS,
  NotifyKind,
  notifyKindsFor,
  notifyOnToSave,
  browserNotifyEnabled,
  defaultTimeZone,
  notificationPermission,
  timeZoneOptions,
  webhookUrlError,
} from '../continuity';
import { deviceNotifyHint, deviceNotifySupport, isAndroid, notifyEnv as browserNotifyEnv, subscribePush, swRegistration, unsubscribePush } from '../push';

const readNotifyEnv = () => ({ ...browserNotifyEnv(), android: isAndroid(navigator) });
import { isSfxMuted, playSfx, toggleSfxMute } from '../audio/chiptune';
import { haptic, hapticsAvailable, hapticsEnabled, setHapticsEnabled } from '../haptics';
import { useAuth } from './AuthGate';
import { SignedInDevices } from './DeviceSignIn';
import { signedInDevicesVisible } from '../deviceSignIn';
import { ComputerModeControl, useComputerMode } from './ComputerModeSwitch';
import { useEngines } from '../useEngines';
import { LayaStatusRow } from './LayaStatus';
import { AppUpdateAbout, useAppUpdate } from './AppUpdateBanner';

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
  max_turns_per_agent?: number;
  max_turns_global?: number;
  delegation_follow_up_minutes?: number;
  /** Agents' own kwnt tools (delegate, ask_user, share_file, skills, history, memory); false when the `agent_tools` setting is "off". */
  agent_tools?: boolean;
  /** The person's IANA zone, "" when not set. */
  timezone?: string;
  /** The person's city as they typed it, "" when not set. */
  location?: string;
  /** Fresh tokens per request before hand-offs pause; 0 = off. Missing on older servers. */
  chain_token_budget?: number;
}

const LIMIT_FIELDS: { key: TurnLimitKey; label: string; hint: string }[] = [
  { key: 'max_turns_per_agent', label: 'Replies at once per bot', hint: 'More than this and a bot’s next request waits in line.' },
  { key: 'max_turns_global', label: 'Replies at once in total', hint: 'Across all bots in every space; keeps the server responsive.' },
  { key: 'delegation_follow_up_minutes', label: 'Check on delegated work after (minutes)', hint: '0 turns the check-in off.' },
];

interface SettingsModalProps {
  isOpen: boolean;
  onClose: () => void;
  /** Tokens used in the open chat, shown under Usage. */
  tokensUsed?: number;
}

/** App settings: shared by every space (keys, Laya, limits, notifications, sounds). Accounts,
 * bots and tools are per space, in Space settings. */
export const SettingsModal: React.FC<SettingsModalProps> = ({ isOpen, onClose, tokensUsed = 0 }) => {
  const { authRequired, deviceLogin, signOut } = useAuth();
  const computer = useComputerMode(isOpen);
  const engines = useEngines(isOpen);
  const [sfxMuted, setSfxMuted] = useState(isSfxMuted);
  const [vibration, setVibration] = useState(hapticsEnabled);
  const [settings, setSettings] = useState<SettingsData | null>(null);
  const [anthropicKey, setAnthropicKey] = useState('');
  const [xaiKey, setXaiKey] = useState('');
  const [openaiKey, setOpenaiKey] = useState('');
  const [ollamaUrl, setOllamaUrl] = useState('http://localhost:11434/v1');
  const [keysOpen, setKeysOpen] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveSuccess, setSaveSuccess] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  // null until loaded (or from a server without the setting), and then left out of the save
  const [agentTools, setAgentTools] = useState<boolean | null>(null);
  // null until loaded, and on a server without these settings
  const [place, setPlace] = useState<{ timezone: string; location: string } | null>(null);
  const [webhookUrl, setWebhookUrl] = useState('');
  const [notifyOn, setNotifyOn] = useState<NotifyKind[]>(NOTIFY_KINDS.map((k) => k.id));
  // The Mac app's update API answered: only then is there an App updates notification to switch
  const appUpdates = useAppUpdate().model.status !== null;
  const [browserNotify, setBrowserNotify] = useState(browserNotifyEnabled);
  const [permission, setPermission] = useState(notificationPermission);
  const [notifyEnv] = useState(() => readNotifyEnv());
  const support = deviceNotifySupport(notifyEnv);
  const [notifyBusy, setNotifyBusy] = useState(false);
  const [notifyError, setNotifyError] = useState<string | null>(null);
  const webhookError = webhookUrlError(webhookUrl);
  // Kept as typed text so a half-typed value can be shown with its error; null until loaded.
  const [limits, setLimits] = useState<Record<TurnLimitKey, string> | null>(null);
  // Fresh tokens one request's chain may use before hand-offs pause; null on a server without it.
  const [chainBudget, setChainBudget] = useState<string | null>(null);
  const chainBudgetError = chainBudget === null ? null : budgetError(chainBudget);
  const limitErrors = Object.fromEntries(
    LIMIT_FIELDS.map(({ key }) => [key, limits ? limitError(key, limits[key]) : null]),
  ) as Record<TurnLimitKey, string | null>;
  const keysSaved = [anthropicKey, xaiKey, openaiKey].filter((k) => k.trim()).length;
  const limitsInvalid = Object.values(limitErrors).some(Boolean);
  const perAgentOverGlobal =
    !!limits && !limitsInvalid && Number(limits.max_turns_per_agent) > Number(limits.max_turns_global);

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
      setNotifyError(null);
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
          setLimits(
            typeof data.max_turns_per_agent === 'number'
              ? {
                  max_turns_per_agent: String(data.max_turns_per_agent),
                  max_turns_global: String(data.max_turns_global ?? ''),
                  delegation_follow_up_minutes: String(data.delegation_follow_up_minutes ?? ''),
                }
              : null,
          );
          setChainBudget(typeof data.chain_token_budget === 'number' ? String(data.chain_token_budget) : null);
          setAgentTools(typeof data.agent_tools === 'boolean' ? data.agent_tools : null);
          setPlace(
            typeof data.timezone === 'string'
              ? { timezone: data.timezone || defaultTimeZone(), location: typeof data.location === 'string' ? data.location : '' }
              : null,
          );
        })
        .catch((err) => console.error('Failed to load settings:', err));
    }
  }, [isOpen]);

  if (!isOpen) return null;

  // Permission is only ever asked from this click, never on load. On: this page's notifications
  // and, where the browser has push, a subscription so they come with kwant closed. Off: both go.
  const toggleBrowserNotify = async () => {
    if (notifyBusy) return;
    playSfx('click');
    setNotifyError(null);
    if (browserNotify) {
      localStorage.setItem(BROWSER_NOTIFY_KEY, 'off');
      setBrowserNotify(false);
      if (support === 'push') {
        setNotifyBusy(true);
        const reg = await swRegistration();
        if (reg) await unsubscribePush(reg).catch(() => undefined);
        setNotifyBusy(false);
      }
      return;
    }
    let p = notificationPermission();
    if (p === 'default') p = await Notification.requestPermission();
    setPermission(p);
    if (p !== 'granted') return;
    localStorage.setItem(BROWSER_NOTIFY_KEY, 'on');
    setBrowserNotify(true);
    if (support !== 'push') return;
    setNotifyBusy(true);
    try {
      const reg = await swRegistration();
      if (!reg) throw new Error('The app isn’t set up in this browser yet. Reload the page and try again.');
      await subscribePush(reg);
    } catch (err) {
      const why = (err instanceof Error ? err.message : 'unknown error').replace(/[.!?]?$/, (m) => m || '.');
      setNotifyError(`Couldn’t set up notifications for when the app is closed: ${why} You’ll still get them while it’s open.`);
    } finally {
      setNotifyBusy(false);
    }
  };
  const notifyHint = deviceNotifyHint({ support, permission, on: browserNotify, android: notifyEnv.android, standalone: notifyEnv.standalone });
  const notifyBlocked = support === 'insecure' || support === 'ios-install' || support === 'unsupported';

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    if (webhookError || limitsInvalid || chainBudgetError) return;
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
          notify_on: notifyOnToSave(notifyOn, appUpdates),
          ...(limits
            ? {
                max_turns_per_agent: Number(limits.max_turns_per_agent),
                max_turns_global: Number(limits.max_turns_global),
                delegation_follow_up_minutes: Number(limits.delegation_follow_up_minutes),
              }
            : {}),
          ...(chainBudget !== null ? { chain_token_budget: budgetValue(chainBudget) } : {}),
          ...(agentTools !== null ? { agent_tools: agentTools } : {}),
          ...(place ? { timezone: place.timezone.trim(), location: place.location.trim() } : {}),
        }),
      });

      if (res.ok) {
        setSaveSuccess(true);
        // Saving a key or the Ollama URL re-checks engines on the server; show the result.
        window.setTimeout(engines.reload, 1500);
        window.setTimeout(engines.reload, 6000);
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
    <div className="kw-dialog fixed inset-0 z-50 bg-black/70 backdrop-blur-sm flex items-center justify-center p-4 select-none">
      <div role="dialog" aria-modal="true" aria-labelledby="settings-title" className="kw-dialog-panel bg-zinc-900 border border-zinc-800 rounded-2xl max-w-lg sm:max-w-2xl w-full max-h-[92vh] overflow-y-auto p-4 sm:p-6 text-zinc-100 shadow-2xl space-y-6">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-zinc-800 pb-4">
          <div className="flex items-center gap-2.5">
            <div className="w-8 h-8 rounded-lg bg-sky-500/10 border border-sky-500/20 flex items-center justify-center text-sky-400">
              <Settings size={17} />
            </div>
            <div>
              <h2 id="settings-title" className="text-sm font-semibold text-zinc-100">App settings</h2>
              <p className="text-xs text-zinc-400">Shared by every space. Accounts, bots and tools are in each space’s settings.</p>
            </div>
          </div>

          <button
            onClick={() => {
              playSfx('click');
              onClose();
            }}
            aria-label="Close settings"
            className="p-1.5 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400"
          >
            <X size={18} />
          </button>
        </div>

        <LayaStatusRow />

        <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
          <div className="flex items-center gap-3 bg-zinc-950/60 p-3 rounded-xl border border-zinc-800">
            <span className="flex-1 text-xs font-medium text-zinc-200 flex items-center gap-2">
              {sfxMuted ? <VolumeX size={15} className="text-zinc-500" aria-hidden="true" /> : <Volume2 size={15} className="text-sky-400" aria-hidden="true" />}
              <span id="sfx-label">UI sounds</span>
            </span>
            <button
              type="button"
              role="switch"
              aria-checked={!sfxMuted}
              aria-labelledby="sfx-label"
              onClick={() => setSfxMuted(toggleSfxMute())}
              className={`relative shrink-0 w-9 h-5 rounded-full transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500 ${
                !sfxMuted ? 'bg-sky-600' : 'bg-zinc-700'
              }`}
            >
              <span className={`absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white transition-transform motion-reduce:transition-none ${!sfxMuted ? 'translate-x-4' : ''}`} />
            </button>
          </div>
          {hapticsAvailable() && (
            <div className="flex items-center gap-3 bg-zinc-950/60 p-3 rounded-xl border border-zinc-800">
              <span className="flex-1 text-xs font-medium text-zinc-200 flex items-center gap-2">
                {vibration ? <Vibrate size={15} className="text-sky-400" aria-hidden="true" /> : <VibrateOff size={15} className="text-zinc-500" aria-hidden="true" />}
                <span id="vibration-label">Vibration</span>
              </span>
              <button
                type="button"
                role="switch"
                aria-checked={vibration}
                aria-labelledby="vibration-label"
                aria-describedby="vibration-hint"
                onClick={() => {
                  setHapticsEnabled(!vibration);
                  setVibration(!vibration);
                  if (!vibration) haptic('tap');
                }}
                className={`relative shrink-0 w-9 h-5 rounded-full transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500 ${
                  vibration ? 'bg-sky-600' : 'bg-zinc-700'
                }`}
              >
                <span className={`absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white transition-transform motion-reduce:transition-none ${vibration ? 'translate-x-4' : ''}`} />
              </button>
              <span id="vibration-hint" className="sr-only">
                A short buzz when you send, answer or solve a puzzle, and when a reply arrives while the app is open.
              </span>
            </div>
          )}
          <div className="flex items-center gap-3 bg-zinc-950/60 p-3 rounded-xl border border-zinc-800">
            <Gauge size={15} className="text-zinc-500" aria-hidden="true" />
            <span className="flex-1 text-xs font-medium text-zinc-200">Token usage, open chat</span>
            <span className="font-mono text-xs text-zinc-300">{tokensUsed} tk</span>
          </div>
        </div>

        <AppUpdateAbout />

        {computer.state.settings && (
          <div className="bg-zinc-950/60 p-4 rounded-xl border border-zinc-800 space-y-3">
            <div>
              <p id="computer-mode-label" className="flex items-center gap-2 text-xs font-medium text-zinc-300">
                <Monitor size={15} className="text-sky-400" aria-hidden="true" />
                <span>Computer for browsing and commands</span>
              </p>
              <p className="mt-1 text-[11px] text-zinc-500">Switches right away, no need to save.</p>
            </div>
            <ComputerModeControl api={computer} />
          </div>
        )}

        {/* Cloud API Keys Form */}
        <form onSubmit={handleSave} className="space-y-4">
          <div className="rounded-xl border border-zinc-800 bg-zinc-950/60">
          <button
            type="button"
            aria-expanded={keysOpen}
            aria-controls="settings-api-keys"
            onClick={() => setKeysOpen((v) => !v)}
            className="w-full flex items-center gap-2 p-3 text-left rounded-xl focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400"
          >
            <Key size={14} className="text-violet-400" aria-hidden="true" />
            <span className="flex-1">
              <span className="block text-xs font-medium text-zinc-200">API keys and Ollama</span>
              <span className="block text-[11px] text-zinc-500">
                Optional, for every space. {keysSaved ? `${keysSaved} saved. ` : ''}Not needed when a space’s bots use a signed-in CLI.
              </span>
            </span>
            {keysOpen ? <ChevronDown size={15} className="text-zinc-500" aria-hidden="true" /> : <ChevronRight size={15} className="text-zinc-500" aria-hidden="true" />}
          </button>

          <div id="settings-api-keys" hidden={!keysOpen} className="space-y-3 px-3 pb-3">
            <p className="text-[11px] text-zinc-500">
              One key per provider for the API-key engines. To keep several keys for one provider, add them as accounts in a space’s settings → Accounts.
            </p>
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
          </div>

          {place && (
            <fieldset className="space-y-3 pt-2 border-t border-zinc-800">
              <legend className="flex items-center gap-2 text-xs font-medium text-zinc-300 pt-3">
                <MapPin size={14} className="text-sky-300" aria-hidden="true" />
                <span>Where you are</span>
              </legend>
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                <div className="space-y-1">
                  <label htmlFor="settings-timezone" className="block text-[11px] font-medium text-zinc-300">
                    Time zone
                  </label>
                  <input
                    id="settings-timezone"
                    list="settings-timezones"
                    value={place.timezone}
                    onChange={(e) => setPlace((p) => (p ? { ...p, timezone: e.target.value } : p))}
                    spellCheck={false}
                    autoCapitalize="off"
                    autoComplete="off"
                    aria-describedby="settings-timezone-hint"
                    className="w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 focus:border-sky-500 focus:outline-none transition-colors"
                  />
                  <datalist id="settings-timezones">
                    {timeZoneOptions(place.timezone).map((z) => (
                      <option key={z} value={z} />
                    ))}
                  </datalist>
                  <p id="settings-timezone-hint" className="text-[11px] text-zinc-500">
                    Bots tell the time and schedule routines in this zone. This device says {defaultTimeZone()}.
                  </p>
                </div>
                <div className="space-y-1">
                  <label htmlFor="settings-location" className="block text-[11px] font-medium text-zinc-300">
                    City <span className="font-normal text-zinc-500">(optional)</span>
                  </label>
                  <input
                    id="settings-location"
                    value={place.location}
                    maxLength={MAX_LOCATION_CHARS}
                    onChange={(e) => setPlace((p) => (p ? { ...p, location: e.target.value } : p))}
                    placeholder="e.g. Ljubljana"
                    autoComplete="address-level2"
                    aria-describedby="settings-location-hint"
                    className="w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors"
                  />
                  <p id="settings-location-hint" className="text-[11px] text-zinc-500">
                    For weather and local tips. Bots only get what you type here, never your GPS location.
                  </p>
                </div>
              </div>
            </fieldset>
          )}

          {limits && (
            <fieldset className="space-y-3 pt-2 border-t border-zinc-800">
              <legend className="flex items-center gap-2 text-xs font-medium text-zinc-300 pt-3">
                <Layers size={14} className="text-sky-300" aria-hidden="true" />
                <span>Parallel work</span>
              </legend>
              <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
                {LIMIT_FIELDS.map(({ key, label, hint }) => {
                  const err = limitErrors[key];
                  const { min, max } = TURN_LIMITS[key];
                  return (
                    <div key={key} className="space-y-1">
                      <label htmlFor={`limit-${key}`} className="block text-[11px] font-medium text-zinc-300">
                        {label}
                      </label>
                      <input
                        id={`limit-${key}`}
                        type="number"
                        inputMode="numeric"
                        min={min}
                        max={max}
                        step={1}
                        value={limits[key]}
                        onChange={(e) => setLimits((prev) => (prev ? { ...prev, [key]: e.target.value } : prev))}
                        aria-invalid={!!err}
                        aria-describedby={`limit-${key}-hint`}
                        className={`w-full bg-zinc-950 border rounded-lg px-3 py-2 text-xs text-zinc-100 focus:outline-none transition-colors ${
                          err ? 'border-rose-500/60 focus:border-rose-400' : 'border-zinc-800 focus:border-sky-500'
                        }`}
                      />
                      <p id={`limit-${key}-hint`} className={`text-[11px] ${err ? 'text-rose-300' : 'text-zinc-500'}`}>
                        {err ?? `${hint} (${min}–${max})`}
                      </p>
                    </div>
                  );
                })}
              </div>
              {perAgentOverGlobal && (
                <p className="text-[11px] text-amber-300" role="status">
                  The total is lower than the per-bot limit, so a bot never gets more than {limits.max_turns_global} at once.
                </p>
              )}
            </fieldset>
          )}

          <fieldset className="space-y-3 pt-2 border-t border-zinc-800">
            <legend className="flex items-center gap-2 text-xs font-medium text-zinc-300 pt-3">
              <Gauge size={14} className="text-sky-300" aria-hidden="true" />
              <span>Usage</span>
            </legend>
            {chainBudget !== null && (
              <div className="space-y-1">
                <label htmlFor="chain-token-budget" className="block text-[11px] font-medium text-zinc-300">
                  Token budget per request (fresh tokens)
                </label>
                <input
                  id="chain-token-budget"
                  type="text"
                  inputMode="numeric"
                  autoComplete="off"
                  value={chainBudget}
                  onChange={(e) => setChainBudget(e.target.value)}
                  aria-invalid={!!chainBudgetError}
                  aria-describedby="chain-token-budget-hint"
                  className={`w-full sm:w-48 bg-zinc-950 border rounded-lg px-3 py-2 text-xs font-mono text-zinc-100 focus:outline-none transition-colors ${
                    chainBudgetError ? 'border-rose-500/60 focus:border-rose-400' : 'border-zinc-800 focus:border-sky-500'
                  }`}
                />
                <p id="chain-token-budget-hint" className={`text-[11px] ${chainBudgetError ? 'text-rose-300' : 'text-zinc-500'}`}>
                  {chainBudgetError ??
                    'When bots handing work to each other go over this for one of your messages, they pause and ask you before going on. 0 turns it off.'}
                </p>
              </div>
            )}
            <DailyUsagePanel open={isOpen} />
          </fieldset>

          {agentTools !== null && (
            <div className="flex items-start gap-3 bg-zinc-950/60 p-3 rounded-xl border border-zinc-800">
              <div className="flex-1">
                <p id="agent-tools-label" className="text-xs font-medium text-zinc-200">Bots can use their own tools</p>
                <p id="agent-tools-hint" className="text-[11px] text-zinc-500">
                  Delegating to each other, asking you questions, sharing files, skills, searching earlier chats and keeping notes. Off, bots just reply.
                </p>
              </div>
              <button
                type="button"
                role="switch"
                aria-checked={agentTools}
                aria-labelledby="agent-tools-label"
                aria-describedby="agent-tools-hint"
                onClick={() => {
                  playSfx('click');
                  setAgentTools((v) => !v);
                }}
                className={`relative shrink-0 w-9 h-5 rounded-full transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500 ${
                  agentTools ? 'bg-sky-600' : 'bg-zinc-700'
                }`}
              >
                <span
                  className={`absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white transition-transform motion-reduce:transition-none ${agentTools ? 'translate-x-4' : ''}`}
                />
              </button>
            </div>
          )}

          <fieldset className="space-y-3 pt-2 border-t border-zinc-800">
            <legend className="flex items-center gap-2 text-xs font-medium text-zinc-300 pt-3">
              <Bell size={14} className="text-amber-300" />
              <span>Notifications</span>
            </legend>

            <div className="flex items-start gap-3 bg-zinc-950/60 p-3 rounded-xl border border-zinc-800" data-testid="device-notify">
              <div className="flex-1 min-w-0">
                <p id="browser-notify-label" className="text-xs font-medium text-zinc-200">Notify me on this device</p>
                <p id="browser-notify-hint" className={`text-[11px] ${notifyHint.warn ? 'text-amber-300' : 'text-zinc-500'}`}>
                  {notifyHint.text}
                </p>
                {notifyError && (
                  <p className="mt-1 text-[11px] text-rose-400" role="alert">
                    {notifyError}
                  </p>
                )}
              </div>
              {/* The track stays 36×20; the button around it is a 44×44 tap target. */}
              <button
                type="button"
                role="switch"
                aria-checked={browserNotify}
                aria-labelledby="browser-notify-label"
                aria-describedby="browser-notify-hint"
                aria-busy={notifyBusy}
                disabled={notifyBusy || (!browserNotify && (notifyBlocked || permission === 'denied'))}
                onClick={toggleBrowserNotify}
                className="group shrink-0 -m-3 p-3 rounded-full disabled:opacity-40 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
              >
                <span className={`relative block w-9 h-5 rounded-full transition-colors ${browserNotify ? 'bg-sky-600' : 'bg-zinc-700'}`}>
                  <span
                    className={`absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white transition-transform motion-reduce:transition-none ${browserNotify ? 'translate-x-4' : ''}`}
                  />
                </span>
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
                      : 'https only. Each post has a short title and line, never message text.')}
              </p>
            </div>

            <div className="space-y-1.5" role="group" aria-labelledby="notify-kinds-label">
              <p id="notify-kinds-label" className="text-xs font-medium text-zinc-400">Tell me when</p>
              {notifyKindsFor(appUpdates).map((k) => (
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

          {signedInDevicesVisible(authRequired, deviceLogin) && <SignedInDevices />}

          {saveError && (
            <div className="bg-rose-500/10 border border-rose-500/20 text-rose-300 px-3 py-2 rounded-lg text-xs flex items-center gap-2" role="alert">
              <AlertTriangle size={14} />
              <span>{saveError}</span>
            </div>
          )}

          {saveSuccess && (
            <div className="bg-emerald-500/10 border border-emerald-500/20 text-emerald-400 px-3 py-2 rounded-lg text-xs flex items-center gap-2">
              <Check size={14} />
              <span>Saved for every space</span>
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
              disabled={saving || !!webhookError || limitsInvalid}
              className="px-4 py-2 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 transition-colors shadow-sm disabled:opacity-50"
            >
              {saving ? 'Saving…' : 'Save'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
