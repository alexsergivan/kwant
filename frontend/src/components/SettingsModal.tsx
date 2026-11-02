import React, { useState, useEffect, useRef } from 'react';
import { AuditorSettings, DAILY_CAPS, SETTING_HINT as AUDITOR_HINT, isAuditorModelError, MODEL_ERROR_TEXT as AUDITOR_MODEL_ERROR, parseAuditorSettings } from '../auditor';
import { AuditorModelSetting } from './AuditorModelSetting';
import { Key, X, Settings, Check, ChevronRight, LogOut, Bell, AlertTriangle, Layers, Monitor, Volume2, VolumeX, Gauge, MapPin, Vibrate, VibrateOff, Sparkles, KanbanSquare } from 'lucide-react';
import { TURN_LIMITS, TurnLimitKey, limitError } from '../threads';
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
import { LayaStatusRow } from './LayaStatus';
import { AppUpdateAbout, useAppUpdate } from './AppUpdateBanner';
import { StarAskSettingsLink } from './StarAsk';
import { ProactiveSettings, WORK_AHEAD_CAP_MAX, WORK_AHEAD_CAP_MIN, parseProactiveSettings } from '../decideFirst';
import {
  AUTOFIX_MODES,
  AutofixSettings,
  DefaultLimits,
  LIMIT_PICKS,
  LimitScope,
  MISSION_BOARD_SETTING_EVENT,
  NO_LIMIT,
  defaultLimitsBody,
  formatTokens,
  limitError as tokenLimitError,
  parseAutofixSettings,
  parseDefaultLimits,
  parseLimit,
  parseMissionBoardSetting,
  requestLimitFrom,
} from '../missions';
import { FlightSchoolSettings, TIERS, TIER_HINTS, TIER_LABELS, Tier, parseFlightSchoolSettings } from '../flightSchool';
import { DISCOVERY_CAPS, DISCOVERY_SETTING_EVENT, parseDiscoverySettings } from '../automationDiscovery';
import { editorProps, isTextField } from '../escape';

const RECOVERY_PCTS = [0, 10, 25, 50, 100];

/** One labelled switch row, like the others in this dialog. */
const SwitchRow: React.FC<{ id: string; label: string; hint: string; checked: boolean; onToggle: () => void; children?: React.ReactNode }> = ({
  id,
  label,
  hint,
  checked,
  onToggle,
  children,
}) => (
  <div className="flex items-start gap-3 bg-zinc-950/60 p-3 rounded-xl border border-zinc-800">
    <div className="flex-1 min-w-0">
      <p id={`${id}-label`} className="text-xs font-medium text-zinc-200">{label}</p>
      <p id={`${id}-hint`} className="text-[11px] text-zinc-500">{hint}</p>
      {children}
    </div>
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-labelledby={`${id}-label`}
      aria-describedby={`${id}-hint`}
      onClick={() => {
        playSfx('click');
        onToggle();
      }}
      className="group shrink-0 -m-3 p-3 rounded-full focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
    >
      <span className={`relative block w-9 h-5 rounded-full transition-colors ${checked ? 'bg-sky-600' : 'bg-zinc-700'}`}>
        <span className={`absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white transition-transform motion-reduce:transition-none ${checked ? 'translate-x-4' : ''}`} />
      </span>
    </button>
  </div>
);

/** The Flight School switch and the tier new bots start at (Phase C3). */
export const FlightSchoolSetting: React.FC<{ value: FlightSchoolSettings; onChange: (v: FlightSchoolSettings) => void }> = ({ value, onChange }) => (
  <SwitchRow
    id="flight-school"
    label="Flight School: limit what each bot may do on its own"
    hint="Give each bot a tier per kind of work: Observe, Sandbox or Act. A tier only adds approval cards, it never removes one. Payments always ask you, whatever the tier."
    checked={value.flight_school}
    onToggle={() => onChange({ ...value, flight_school: !value.flight_school })}
  >
    {value.flight_school && (
      <div className="mt-2 space-y-2" data-testid="flight-school-options">
        <p className="text-[11px] text-zinc-300">
          Bots you already have stay at Act, so nothing changes until you lower one in its settings.
        </p>
        <div className="flex flex-wrap items-center gap-2">
          <label htmlFor="flight-school-new-bot" className="text-[11px] text-zinc-300">
            Bots you add from now on start at
          </label>
          <select
            id="flight-school-new-bot"
            value={value.flight_school_new_bot_tier}
            onChange={(e) => onChange({ ...value, flight_school_new_bot_tier: e.target.value as Tier })}
            className="bg-zinc-950 border border-zinc-800 rounded-lg px-2 py-1.5 text-xs text-zinc-100 focus:border-sky-500 focus:outline-none"
          >
            {TIERS.map((t) => (
              <option key={t} value={t}>
                {TIER_LABELS[t]}
              </option>
            ))}
          </select>
        </div>
        <p className="text-[11px] text-zinc-500">{TIER_HINTS[value.flight_school_new_bot_tier]}</p>
      </div>
    )}
  </SwitchRow>
);

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
  /** Default token limits for new tasks and missions; 0 = no limit. Missing on older servers. */
  mission_default_task_budget_tokens?: number;
  mission_default_budget_tokens?: number;
  /** Proactive bots; missing on older servers. */
  decide_first?: boolean;
  work_ahead?: boolean;
  work_ahead_daily_cap?: number;
}

/**
 * A default token limit: "No limit" or a quick pick, or Custom with a typed amount. `value` is the
 * typed text while it's custom, so a half-typed amount shows its error; `onChange` gets the text.
 */
const DefaultLimitField: React.FC<{ id: string; label: string; scope: LimitScope; value: string; onChange: (v: string) => void }> = ({ id, label, scope, value, onChange }) => {
  const n = parseLimit(value);
  const [custom, setCustom] = useState(() => n === null || (n !== NO_LIMIT && !(LIMIT_PICKS as readonly number[]).includes(n)));
  const err = custom ? tokenLimitError(value, scope) : null;
  const choice = custom ? 'custom' : String(n ?? NO_LIMIT);
  return (
    <div className="space-y-1">
      <label htmlFor={id} className="block text-[11px] font-medium text-zinc-300">
        {label}
      </label>
      <div className="flex flex-wrap gap-2">
        <select
          id={id}
          value={choice}
          onChange={(e) => {
            if (e.target.value === 'custom') {
              setCustom(true);
              onChange('');
            } else {
              setCustom(false);
              onChange(e.target.value);
            }
          }}
          className="bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 min-h-[40px] sm:min-h-[36px] text-xs text-zinc-100 focus:outline-none focus:border-sky-500"
        >
          <option value={String(NO_LIMIT)}>No limit</option>
          {LIMIT_PICKS.map((p) => (
            <option key={p} value={String(p)}>
              {formatTokens(p)} tokens
            </option>
          ))}
          <option value="custom">Custom…</option>
        </select>
        {custom && (
          <input
            aria-label={`${label}, custom amount`}
            type="text"
            inputMode="decimal"
            autoComplete="off"
            placeholder="e.g. 300k"
            value={value}
            onChange={(e) => onChange(e.target.value)}
            aria-invalid={!!err}
            aria-describedby={`${id}-err`}
            className={`w-32 bg-zinc-950 border rounded-lg px-3 py-2 text-xs text-zinc-100 focus:outline-none ${err ? 'border-rose-500/60 focus:border-rose-400' : 'border-zinc-800 focus:border-sky-500'}`}
          />
        )}
      </div>
      {custom && (
        <p id={`${id}-err`} className={`text-[11px] ${err ? 'text-rose-300' : 'text-zinc-500'}`}>
          {err ?? 'Tokens, like 300k or 2M.'}
        </p>
      )}
    </div>
  );
};

/** Typed default-limit text → numbers, or null while one can't be saved. */
const defaultLimitsFrom = (t: { task: string; mission: string }): DefaultLimits | null => {
  const task = parseLimit(t.task);
  const mission = parseLimit(t.mission);
  if (task === null || mission === null) return null;
  if (task !== NO_LIMIT && tokenLimitError(t.task, 'task')) return null;
  if (mission !== NO_LIMIT && tokenLimitError(t.mission, 'mission')) return null;
  return { task, mission };
};

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
  /** Inside Settings › All spaces: no dialog frame, no close button, Escape is the parent's. */
  embedded?: boolean;
  /** Opens Settings › This space › Accounts, where the API keys and Ollama address live. */
  onOpenApiKeys?: () => void;
}

/** App settings: shared by every space (keys, Laya, limits, notifications, sounds). Accounts,
 * bots and tools are per space, in Space settings. */
type DiscoverySettings = NonNullable<ReturnType<typeof parseDiscoverySettings>>;

/** Automation Discovery: off until the person turns it on; suggestions only, never a routine on its own. */
const DiscoverySetting: React.FC<{ value: DiscoverySettings; onChange: (v: DiscoverySettings) => void }> = ({ value, onChange }) => {
  const on = value.automation_discovery === 'on';
  const cap = value.automation_discovery_daily_cap;
  return (
    <SwitchRow
      id="automation-discovery"
      label="Suggest routines from repeated tasks"
      hint="When you ask for the same task on the board on a regular rhythm, you get a suggested routine for it, with the bot, the time and the checks it would keep. Nothing is set up until you accept it in Routines. It learns only from board tasks you created, never from chats."
      checked={on}
      onToggle={() => onChange({ ...value, automation_discovery: on ? 'off' : 'on' })}
    >
      {on && (
        <div className="mt-2 flex flex-wrap items-center gap-2" data-testid="automation-discovery-options">
          <label htmlFor="automation-discovery-cap" className="text-[11px] text-zinc-300">
            At most
          </label>
          <select
            id="automation-discovery-cap"
            value={cap}
            onChange={(e) => onChange({ ...value, automation_discovery_daily_cap: Number(e.target.value) })}
            aria-describedby="automation-discovery-cap-hint"
            className="bg-zinc-950 border border-zinc-800 rounded-lg px-2 py-1.5 text-xs text-zinc-100 focus:border-sky-500 focus:outline-none"
          >
            {DISCOVERY_CAPS.map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </select>
          <span className="text-[11px] text-zinc-300">new {cap === 1 ? 'suggestion' : 'suggestions'} a day</span>
          <p id="automation-discovery-cap-hint" className="basis-full text-[11px] text-zinc-500">
            {cap === 0 ? 'With 0, it still looks but suggests nothing new; Scan now in Routines shows what it found.' : 'More waiting suggestions come on later days.'}
          </p>
        </div>
      )}
    </SwitchRow>
  );
};

export const SettingsModal: React.FC<SettingsModalProps> = ({ isOpen, onClose, tokensUsed = 0, embedded = false, onOpenApiKeys }) => {
  const { authRequired, deviceLogin, signOut } = useAuth();
  const computer = useComputerMode(isOpen);
  const [sfxMuted, setSfxMuted] = useState(isSfxMuted);
  const [vibration, setVibration] = useState(hapticsEnabled);
  const [settings, setSettings] = useState<SettingsData | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveSuccess, setSaveSuccess] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  // Text typed into the form and not saved yet: Escape keeps it (and Settings) open.
  const [typed, setTyped] = useState(false);
  // A 400 about the audit model shows next to that picker, not only at the bottom.
  const modelError = isAuditorModelError(saveError);
  // null until loaded (or from a server without the setting), and then left out of the save
  const [agentTools, setAgentTools] = useState<boolean | null>(null);
  // Deciding and working ahead on their own; null on a server without these settings
  const [proactive, setProactive] = useState<ProactiveSettings | null>(null);
  // null: this server has no Mission board, so the switch stays hidden.
  const [missionBoard, setMissionBoard] = useState<boolean | null>(null);
  const [autofix, setAutofix] = useState<AutofixSettings | null>(null);
  // null on a server without the Swarm Auditor: the switch stays hidden and out of the save
  const [auditor, setAuditor] = useState<AuditorSettings | null>(null);
  // null on a server without Flight School: the switch stays hidden and out of the save
  const [flightSchool, setFlightSchool] = useState<FlightSchoolSettings | null>(null);
  const [discovery, setDiscovery] = useState<DiscoverySettings | null>(null);
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
  // Fresh tokens one request's hand-offs may use before they pause (0 = no limit), as typed; null on a server without it.
  const [chainBudget, setChainBudget] = useState<string | null>(null);
  const chainBudgetValue = chainBudget === null ? null : requestLimitFrom(chainBudget);
  const chainBudgetError = chainBudget !== null && chainBudgetValue === null;
  // Default token limits for new tasks and missions, as typed; null on a server without them.
  const [tokenLimits, setTokenLimits] = useState<{ task: string; mission: string } | null>(null);
  const tokenLimitsValue = tokenLimits ? defaultLimitsFrom(tokenLimits) : null;
  const tokenLimitsInvalid = !!tokenLimits && !tokenLimitsValue;
  const limitErrors = Object.fromEntries(
    LIMIT_FIELDS.map(({ key }) => [key, limits ? limitError(key, limits[key]) : null]),
  ) as Record<TurnLimitKey, string | null>;
  const limitsInvalid = Object.values(limitErrors).some(Boolean);
  const perAgentOverGlobal =
    !!limits && !limitsInvalid && Number(limits.max_turns_per_agent) > Number(limits.max_turns_global);

  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  useEffect(() => {
    if (!isOpen || embedded) return;
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && !e.defaultPrevented && closeRef.current();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [isOpen, embedded]);

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
          const dl = parseDefaultLimits(data);
          setTokenLimits(dl ? { task: String(dl.task), mission: String(dl.mission) } : null);
          setAgentTools(typeof data.agent_tools === 'boolean' ? data.agent_tools : null);
          setProactive(parseProactiveSettings(data));
          setMissionBoard(parseMissionBoardSetting(data));
          setAutofix(parseAutofixSettings(data));
          setAuditor(parseAuditorSettings(data));
          setFlightSchool(parseFlightSchoolSettings(data));
          setDiscovery(parseDiscoverySettings(data));
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
    if (webhookError || limitsInvalid || chainBudgetError || tokenLimitsInvalid) return;
    setSaveError(null);
    setSaving(true);
    playSfx('click');

    try {
      const res = await fetch('/api/settings', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          notify_webhook_url: webhookUrl.trim(),
          notify_on: notifyOnToSave(notifyOn, appUpdates, !!proactive, !!discovery, flightSchool !== null),
          ...(limits
            ? {
                max_turns_per_agent: Number(limits.max_turns_per_agent),
                max_turns_global: Number(limits.max_turns_global),
                delegation_follow_up_minutes: Number(limits.delegation_follow_up_minutes),
              }
            : {}),
          ...(chainBudgetValue !== null ? { chain_token_budget: chainBudgetValue } : {}),
          ...(tokenLimitsValue ? defaultLimitsBody(tokenLimitsValue) : {}),
          ...(agentTools !== null ? { agent_tools: agentTools } : {}),
          ...(proactive ?? {}),
          ...(missionBoard !== null ? { mission_board: missionBoard } : {}),
          ...(autofix !== null ? { autofix: autofix.autofix, recovery_budget_pct: autofix.recovery_budget_pct } : {}),
          ...(flightSchool ?? {}),
          ...(auditor ?? {}),
          ...(discovery ?? {}),
          ...(place ? { timezone: place.timezone.trim(), location: place.location.trim() } : {}),
        }),
      });

      if (res.ok) {
        setSaveSuccess(true);
        setTyped(false);
        if (missionBoard !== null) window.dispatchEvent(new CustomEvent(MISSION_BOARD_SETTING_EVENT, { detail: missionBoard }));
        if (discovery) window.dispatchEvent(new CustomEvent(DISCOVERY_SETTING_EVENT, { detail: discovery }));
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

  // A plain call, not a component: a component made here would remount the form on every render.
  const frame = (children: React.ReactNode) =>
    embedded ? (
      <div data-app-settings className="text-zinc-100 space-y-6 select-none">
        {children}
      </div>
    ) : (
      <div className="kw-dialog fixed inset-0 z-50 bg-black/70 backdrop-blur-sm flex items-center justify-center p-4 select-none">
        <div role="dialog" aria-modal="true" aria-labelledby="settings-title" className="kw-dialog-panel bg-zinc-900 border border-zinc-800 rounded-2xl max-w-lg sm:max-w-2xl w-full max-h-[92vh] overflow-y-auto p-4 sm:p-6 text-zinc-100 shadow-2xl space-y-6">
          {children}
        </div>
      </div>
    );

  return frame(
    <>
        {/* Header: the dialog's own; embedded, the Settings screen has the heading and the close button. */}
        {!embedded && (
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
        )}

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
        <StarAskSettingsLink />

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

        {/* App settings form; API keys moved to Settings › This space › Accounts */}
        <form onSubmit={handleSave} onChange={(e) => isTextField(e.target) && setTyped(true)} {...editorProps(typed)} className="space-y-4">
          {onOpenApiKeys && (
            <button
              type="button"
              onClick={() => {
                playSfx('click');
                onOpenApiKeys();
              }}
              data-api-keys-link
              className="w-full flex items-center gap-2 p-3 text-left rounded-xl border border-zinc-800 bg-zinc-950/60 hover:border-zinc-700 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400"
            >
              <Key size={14} className="text-violet-400" aria-hidden="true" />
              <span className="flex-1">
                <span className="block text-xs font-medium text-zinc-200">API keys and Ollama</span>
                <span className="block text-[11px] text-zinc-500">Now in Settings › This space › Accounts, next to the accounts that use them.</span>
              </span>
              <ChevronRight size={15} className="text-zinc-500" aria-hidden="true" />
            </button>
          )}

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
            {tokenLimits !== null && (
              <div className="space-y-2" role="group" aria-labelledby="token-limits-h" data-token-limits>
                <div>
                  <p id="token-limits-h" className="text-[11px] font-semibold text-zinc-200">
                    Token limits
                  </p>
                  <p className="text-[11px] text-zinc-500">Bots can use as many tokens as they need. Set a limit to cap spending per task.</p>
                </div>
                <div className="grid gap-3 sm:grid-cols-2">
                  <DefaultLimitField
                    id="default-task-limit"
                    label="New tasks"
                    scope="task"
                    value={tokenLimits.task}
                    onChange={(v) => setTokenLimits((p) => (p ? { ...p, task: v } : p))}
                  />
                  <DefaultLimitField
                    id="default-mission-limit"
                    label="New missions"
                    scope="mission"
                    value={tokenLimits.mission}
                    onChange={(v) => setTokenLimits((p) => (p ? { ...p, mission: v } : p))}
                  />
                </div>
                <p className="text-[11px] text-zinc-500">You can still set or remove a limit on any task or mission from the board.</p>
              </div>
            )}
            {chainBudget !== null && (
              <div className="space-y-1" data-chain-token-budget>
                <DefaultLimitField id="chain-token-budget" label="Hand-offs per request" scope="request" value={chainBudget} onChange={setChainBudget} />
                <p className="text-[11px] text-zinc-500">
                  With no limit, bots hand work to each other until they're done. Set one and they pause and ask you before going over it for one of
                  your messages.
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

          {proactive && (
            <fieldset className="space-y-3 pt-2 border-t border-zinc-800" data-testid="proactive-settings">
              <legend className="flex items-center gap-2 text-xs font-medium text-zinc-300 pt-3">
                <Sparkles size={14} className="text-violet-300" aria-hidden="true" />
                <span>Bots on their own</span>
              </legend>
              <SwitchRow
                id="decide-first"
                label="Bots decide reversible choices themselves"
                hint="Instead of asking you to pick, they run their own checks, ask a teammate, choose, and show it with a Change button. They still ask you before sending, paying, deleting or signing in."
                checked={proactive.decide_first}
                onToggle={() => setProactive((p) => (p ? { ...p, decide_first: !p.decide_first } : p))}
              />
              <SwitchRow
                id="work-ahead"
                label="Bots work ahead while I’m away"
                hint="When a chat has been quiet for 2 hours, its lead finishes open checks and gets the next step ready. You get one “While you were away” message per chat a day at most. They never send, pay or contact anyone."
                checked={proactive.work_ahead}
                onToggle={() => setProactive((p) => (p ? { ...p, work_ahead: !p.work_ahead } : p))}
              >
                <div className="mt-2 flex items-center gap-2">
                  <label htmlFor="work-ahead-cap" className={`text-[11px] ${proactive.work_ahead ? 'text-zinc-300' : 'text-zinc-500'}`}>
                    At most
                  </label>
                  <select
                    id="work-ahead-cap"
                    value={proactive.work_ahead_daily_cap}
                    disabled={!proactive.work_ahead}
                    onChange={(e) => setProactive((p) => (p ? { ...p, work_ahead_daily_cap: Number(e.target.value) } : p))}
                    className="bg-zinc-950 border border-zinc-800 rounded-lg px-2 py-1.5 text-xs text-zinc-100 focus:border-sky-500 focus:outline-none disabled:opacity-50"
                  >
                    {Array.from({ length: WORK_AHEAD_CAP_MAX - WORK_AHEAD_CAP_MIN + 1 }, (_, i) => i + WORK_AHEAD_CAP_MIN).map((n) => (
                      <option key={n} value={n}>
                        {n}
                      </option>
                    ))}
                  </select>
                  <span className={`text-[11px] ${proactive.work_ahead ? 'text-zinc-300' : 'text-zinc-500'}`}>
                    {proactive.work_ahead_daily_cap === 1 ? 'time' : 'times'} a day in each space
                  </span>
                </div>
              </SwitchRow>
            </fieldset>
          )}

          {missionBoard !== null && (
            <fieldset className="space-y-3 pt-2 border-t border-zinc-800" data-testid="mission-board-settings">
              <legend className="flex items-center gap-2 text-xs font-medium text-zinc-300 pt-3">
                <KanbanSquare size={14} className="text-sky-400" aria-hidden="true" />
                <span>Mission board</span>
              </legend>
              <SwitchRow
                id="mission-board"
                label="Show the Mission board"
                hint="One board for all your team’s work: Ready, Working, Needs you, Review and Verified. A task reaches Verified only when a check passes, not on a bot’s word."
                checked={missionBoard}
                onToggle={() => setMissionBoard((v) => !v)}
              />
              {autofix !== null && (
                <SwitchRow
                  id="autofix"
                  label="Retry failed tasks with AutoFix"
                  hint="When a task fails, AutoFix works out why and plans a retry that fits: wait a bit, start from a checkpoint, or pass along what the check said. It never retries a missing login, a denied action or a spent budget."
                  checked={autofix.autofix !== 'off'}
                  onToggle={() => setAutofix((a) => (a ? { ...a, autofix: a.autofix === 'off' ? 'suggest' : 'off' } : a))}
                >
                  {autofix.autofix !== 'off' && (
                    <div className="mt-2 space-y-2" data-testid="autofix-options">
                      <div role="radiogroup" aria-label="How AutoFix retries" className="space-y-1">
                        {AUTOFIX_MODES.filter((m) => m.value !== 'off').map((m) => (
                          <label key={m.value} className="flex items-start gap-2 min-h-[32px] cursor-pointer text-[11px]">
                            <input
                              type="radio"
                              name="autofix-mode"
                              value={m.value}
                              checked={autofix.autofix === m.value}
                              onChange={() => setAutofix((a) => (a ? { ...a, autofix: m.value } : a))}
                              className="mt-0.5 accent-sky-500"
                            />
                            <span>
                              <span className="block font-medium text-zinc-200">{m.value === 'suggest' ? 'Ask me first' : 'Retry on its own'}</span>
                              <span className="block text-zinc-500">{m.hint}</span>
                            </span>
                          </label>
                        ))}
                      </div>
                      <div className="flex flex-wrap items-center gap-2">
                        <label htmlFor="recovery-pct" className="text-[11px] text-zinc-300">
                          Retries may spend up to
                        </label>
                        <select
                          id="recovery-pct"
                          value={autofix.recovery_budget_pct}
                          onChange={(e) => setAutofix((a) => (a ? { ...a, recovery_budget_pct: Number(e.target.value) } : a))}
                          className="bg-zinc-950 border border-zinc-800 rounded-lg px-2 py-1.5 text-xs text-zinc-100 focus:border-sky-500 focus:outline-none"
                        >
                          {[...new Set([...RECOVERY_PCTS, autofix.recovery_budget_pct])]
                            .sort((x, y) => x - y)
                            .map((n) => (
                              <option key={n} value={n}>
                                {n}%
                              </option>
                            ))}
                        </select>
                        <span className="text-[11px] text-zinc-300">of each task’s budget</span>
                      </div>
                    </div>
                  )}
                </SwitchRow>
              )}
              {flightSchool !== null && <FlightSchoolSetting value={flightSchool} onChange={setFlightSchool} />}
              {auditor !== null && (
                <SwitchRow
                  id="auditor"
                  label="Challenge important work with an audit"
                  hint={AUDITOR_HINT}
                  checked={auditor.auditor === 'on'}
                  onToggle={() => setAuditor((a) => (a ? { ...a, auditor: a.auditor === 'on' ? 'off' : 'on' } : a))}
                >
                  {auditor.auditor === 'on' && (
                    <div className="mt-2 flex flex-wrap items-center gap-2" data-testid="auditor-options">
                      <label htmlFor="auditor-cap" className="text-[11px] text-zinc-300">
                        At most
                      </label>
                      <select
                        id="auditor-cap"
                        value={auditor.auditor_daily_cap}
                        onChange={(e) => setAuditor((a) => (a ? { ...a, auditor_daily_cap: Number(e.target.value) } : a))}
                        className="bg-zinc-950 border border-zinc-800 rounded-lg px-2 py-1.5 text-xs text-zinc-100 focus:border-sky-500 focus:outline-none"
                      >
                        {[...new Set([...DAILY_CAPS, auditor.auditor_daily_cap])]
                          .sort((x, y) => x - y)
                          .map((n) => (
                            <option key={n} value={n}>
                              {n}
                            </option>
                          ))}
                      </select>
                      <span className="text-[11px] text-zinc-300">{auditor.auditor_daily_cap === 1 ? 'audit' : 'audits'} a day</span>
                    </div>
                  )}
                  {(auditor.auditor === 'on' || modelError) && auditor.auditor_model !== undefined && (
                    <AuditorModelSetting
                      value={auditor.auditor_model}
                      error={modelError ? AUDITOR_MODEL_ERROR : null}
                      onChange={(m) => {
                        setSaveError(null);
                        setAuditor((x) => (x ? { ...x, auditor_model: m } : x));
                      }}
                    />
                  )}
                </SwitchRow>
              )}
              {discovery !== null && <DiscoverySetting value={discovery} onChange={setDiscovery} />}
            </fieldset>
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
              {notifyKindsFor(appUpdates, !!proactive, !!discovery, !!flightSchool?.flight_school).map((k) => (
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
              disabled={saving || !!webhookError || limitsInvalid || tokenLimitsInvalid}
              className="px-4 py-2 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 transition-colors shadow-sm disabled:opacity-50"
            >
              {saving ? 'Saving…' : 'Save'}
            </button>
          </div>
        </form>
    </>,
  );
};
