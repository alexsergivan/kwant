import React, { useCallback, useEffect, useReducer, useRef, useState } from 'react';
import { AlertTriangle, AppWindow, Box, Laptop, Loader2, PanelRight, X } from 'lucide-react';
import { playSfx } from '../audio/chiptune';
import {
  COMPUTER_MODES,
  COMPUTER_MODE_EVENT,
  LOCAL_BROWSER_NOTE,
  ComputerMode,
  ComputerSettings,
  SwitchState,
  fetchComputerSettings,
  initialSwitchState,
  modeLabel,
  otherMode,
  saveComputerMode,
  saveLocalWindow,
  showsLocalWindow,
  shownMode,
  switchReducer,
  unavailableReason,
} from '../computerMode';

export interface ComputerModeApi {
  state: SwitchState;
  /** Saves at once; on failure the control goes back and `state.error` says why. */
  select: (mode: ComputerMode) => void;
  dismissError: () => void;
  /** Local mode: opens the bots' Chrome as a window, or brings it back into the app. */
  setWindow: (visible: boolean) => void;
  /** True while a window switch restarts Chrome. */
  windowPending: boolean;
}

/** Loads the setting while `enabled`, and follows switches made elsewhere in this tab. */
export function useComputerMode(enabled: boolean): ComputerModeApi {
  const [state, dispatch] = useReducer(switchReducer, initialSwitchState);
  const stateRef = useRef(state);
  stateRef.current = state;

  useEffect(() => {
    if (!enabled) return;
    let alive = true;
    fetchComputerSettings()
      .then((s) => alive && s && dispatch({ type: 'loaded', settings: s }))
      .catch(() => {});
    const onSwitched = (e: Event) => dispatch({ type: 'loaded', settings: (e as CustomEvent<ComputerSettings>).detail });
    window.addEventListener(COMPUTER_MODE_EVENT, onSwitched);
    return () => {
      alive = false;
      window.removeEventListener(COMPUTER_MODE_EVENT, onSwitched);
    };
  }, [enabled]);

  const select = useCallback((mode: ComputerMode) => {
    const { settings, pending } = stateRef.current;
    if (!settings || pending || mode === settings.mode) return;
    playSfx('click');
    dispatch({ type: 'start', mode });
    saveComputerMode(mode).then((r) => {
      if (r.ok) {
        dispatch({ type: 'saved', settings: r.settings });
        window.dispatchEvent(new CustomEvent(COMPUTER_MODE_EVENT, { detail: r.settings }));
      } else {
        dispatch({ type: 'failed', error: r.error });
      }
    });
  }, []);

  const [windowPending, setWindowPending] = useState(false);
  const setWindow = useCallback((visible: boolean) => {
    if (stateRef.current.pending) return;
    playSfx('click');
    setWindowPending(true);
    saveLocalWindow(visible).then((r) => {
      setWindowPending(false);
      if (r.ok) {
        dispatch({ type: 'saved', settings: r.settings });
        window.dispatchEvent(new CustomEvent(COMPUTER_MODE_EVENT, { detail: r.settings }));
      } else {
        dispatch({ type: 'failed', error: r.error });
      }
    });
  }, []);

  // Closing the window puts the browser back in the app on the server; notice that.
  const windowOpen = enabled && showsLocalWindow(state.settings);
  useEffect(() => {
    if (!windowOpen) return;
    const timer = window.setInterval(() => {
      fetchComputerSettings()
        .then((s) => s && dispatch({ type: 'loaded', settings: s }))
        .catch(() => {});
    }, 5000);
    return () => window.clearInterval(timer);
  }, [windowOpen]);

  const dismissError = useCallback(() => dispatch({ type: 'dismiss' }), []);
  return { state, select, dismissError, setWindow, windowPending };
}

const ICONS: Record<ComputerMode, typeof Laptop> = { local: Laptop, vm: Box };

export const ComputerModeError: React.FC<{ error: string; onDismiss?: () => void }> = ({ error, onDismiss }) => (
  <div role="alert" className="flex items-start gap-2 px-3 py-2 rounded-lg bg-rose-500/10 border border-rose-500/20 text-rose-300 text-[11px]">
    <AlertTriangle size={13} className="shrink-0 mt-px" aria-hidden="true" />
    <span className="flex-1">{error}</span>
    {onDismiss && (
      <button type="button" onClick={onDismiss} aria-label="Dismiss" className="shrink-0 text-rose-300/70 hover:text-rose-200">
        <X size={12} />
      </button>
    )}
  </div>
);

/** Two-option control for the Settings screen. Renders nothing until the setting is loaded. */
export const ComputerModeControl: React.FC<{ api: ComputerModeApi }> = ({ api }) => {
  const { state, select, dismissError } = api;
  const refs = useRef<Partial<Record<ComputerMode, HTMLButtonElement | null>>>({});
  const settings = state.settings;
  if (!settings) return null;
  const selected = shownMode(state);

  const onKeyDown = (e: React.KeyboardEvent, mode: ComputerMode) => {
    if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(e.key)) return;
    e.preventDefault();
    const next = otherMode(mode);
    if (unavailableReason(settings, next)) return;
    refs.current[next]?.focus();
    select(next);
  };

  return (
    <div className="space-y-2">
      <div role="radiogroup" aria-labelledby="computer-mode-label" className="grid grid-cols-1 sm:grid-cols-2 gap-2">
        {COMPUTER_MODES.map(({ id, label, hint }) => {
          const Icon = ICONS[id];
          const reason = unavailableReason(settings, id);
          const checked = selected === id;
          const saving = state.pending === id;
          return (
            <button
              key={id}
              ref={(el) => (refs.current[id] = el)}
              type="button"
              role="radio"
              aria-checked={checked}
              aria-disabled={!!reason || undefined}
              aria-describedby={`computer-mode-${id}-hint`}
              tabIndex={checked || (!selected && id === 'local') ? 0 : -1}
              // Not disabled while saving, so keyboard focus stays put; select() ignores it.
              disabled={!!reason}
              aria-busy={saving || undefined}
              onClick={() => select(id)}
              onKeyDown={(e) => onKeyDown(e, id)}
              className={`text-left p-3 rounded-xl border transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500 ${
                checked
                  ? 'border-sky-500/60 bg-sky-500/10'
                  : reason
                    ? 'border-zinc-800 bg-zinc-950/40 cursor-not-allowed'
                    : 'border-zinc-800 bg-zinc-950/60 hover:border-sky-500/40'
              } ${state.pending && !saving ? 'opacity-60' : ''}`}
            >
              <span className={`flex items-center gap-2 text-xs font-medium ${reason ? 'text-zinc-500' : 'text-zinc-100'}`}>
                {saving ? <Loader2 size={14} className="animate-spin text-sky-400" aria-hidden="true" /> : <Icon size={14} className={checked ? 'text-sky-400' : 'text-zinc-500'} aria-hidden="true" />}
                {label}
                {saving && <span className="sr-only">(switching)</span>}
              </span>
              <span id={`computer-mode-${id}-hint`} className="block mt-1 text-[11px] leading-snug">
                <span className={reason ? 'text-zinc-600' : 'text-zinc-500'}>{hint}</span>
                {reason && <span className="block mt-1 text-amber-300/90">Unavailable: {reason}</span>}
              </span>
            </button>
          );
        })}
      </div>
      {state.error && <ComputerModeError error={state.error} onDismiss={dismissError} />}
    </div>
  );
};

/** One-click switch for the browser pane header. Renders nothing until the setting is loaded. */
export const ComputerModeChip: React.FC<{ api: ComputerModeApi }> = ({ api }) => {
  const { state, select } = api;
  const settings = state.settings;
  const current = shownMode(state);
  if (!settings || !current) return null;
  const next = otherMode(current);
  const reason = unavailableReason(settings, next);
  const Icon = ICONS[current];
  const short = COMPUTER_MODES.find((m) => m.id === current)!.short;
  const title = state.pending
    ? `Switching to ${modeLabel(current)}…`
    : reason
      ? `Using ${modeLabel(current)}. ${modeLabel(next)} is unavailable: ${reason}`
      : `Using ${modeLabel(current)}. Click to switch to ${modeLabel(next)}.`;
  return (
    <button
      type="button"
      onClick={() => select(next)}
      disabled={!!reason}
      aria-busy={!!state.pending || undefined}
      title={title}
      aria-label={title}
      className={`flex items-center gap-1 px-1.5 py-1 rounded-md border text-[10px] font-semibold transition-colors disabled:cursor-not-allowed focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500 ${
        current === 'local'
          ? 'border-amber-500/30 bg-amber-500/10 text-amber-200 enabled:hover:bg-amber-500/20'
          : 'border-sky-500/30 bg-sky-500/10 text-sky-200 enabled:hover:bg-sky-500/20'
      }`}
    >
      {state.pending ? <Loader2 size={12} className="animate-spin" aria-hidden="true" /> : <Icon size={12} aria-hidden="true" />}
      <span>{short}</span>
    </button>
  );
};

/** Local mode, in-app view: opens the same browser as a real window for a sign-in or CAPTCHA. */
export const OpenWindowButton: React.FC<{ api: ComputerModeApi; label?: boolean }> = ({ api, label }) => {
  const settings = api.state.settings;
  if (settings?.mode !== 'local' || settings.window.visible) return null;
  const reason = settings.window.available ? null : settings.window.reason;
  const title = reason
    ? `Can't open a window: ${reason}`
    : 'Open this browser as a window on your computer, to sign in or solve a CAPTCHA yourself';
  return (
    <button
      type="button"
      onClick={() => api.setWindow(true)}
      disabled={!!reason || api.windowPending}
      aria-busy={api.windowPending || undefined}
      title={title}
      aria-label={title}
      className="flex items-center gap-1 p-1.5 rounded-md text-zinc-400 enabled:hover:text-zinc-100 enabled:hover:bg-zinc-800 disabled:opacity-50 disabled:cursor-not-allowed"
    >
      {api.windowPending ? <Loader2 size={13} className="animate-spin" aria-hidden="true" /> : <AppWindow size={13} aria-hidden="true" />}
      {label && <span className="text-[10px] font-semibold whitespace-nowrap">Open as window</span>}
    </button>
  );
};

/** Shown in place of the live view while the bots' Chrome is open as a window. */
export const LocalBrowserNote: React.FC<{ api?: ComputerModeApi }> = ({ api }) => (
  <div className="flex-1 flex items-center justify-center p-6 rounded-xl border border-zinc-800 bg-zinc-950 text-center">
    <div className="max-w-xs space-y-2">
      <Laptop size={28} className="mx-auto text-amber-300/80" aria-hidden="true" />
      <p className="text-xs font-medium text-zinc-200">{LOCAL_BROWSER_NOTE}</p>
      <p className="text-[11px] text-zinc-500">Sign in or finish what you need there. Closing the window, or the button below, brings it back here.</p>
      {api && (
        <button
          type="button"
          onClick={() => api.setWindow(false)}
          disabled={api.windowPending}
          aria-busy={api.windowPending || undefined}
          className="inline-flex items-center gap-1.5 mt-1 px-3 py-1.5 rounded-lg text-[11px] font-semibold border border-sky-500/30 bg-sky-500/10 text-sky-200 enabled:hover:bg-sky-500/20 disabled:opacity-60"
        >
          {api.windowPending ? <Loader2 size={12} className="animate-spin" aria-hidden="true" /> : <PanelRight size={12} aria-hidden="true" />}
          Show it here instead
        </button>
      )}
    </div>
  </div>
);
