// The Dev mode switch in the space controls (explainer spec §3, flow F1/F2/F9): switch v3, a state word
// (Off / On / Off?) next to a track and knob. One press turns it on (F1). Turning it off takes two presses
// (F2): press 1 arms (amber track ring, open lock, "Off?"), press 2 within 5 s turns it off. Armed drops
// back after 5 s, on Esc or when focus leaves. Presses are ignored while a flip settles (450 ms), so a
// double-click on Off can't turn it on and arm it. The armed line itself is the Dev mode row's sub-line;
// the polite live region here still announces it.

import React, { useCallback, useEffect, useId, useRef, useState, useSyncExternalStore } from 'react';
import { Lock, LockOpen, X } from 'lucide-react';
import {
  RECOVER_MS,
  SwitchAction,
  SwitchState,
  approvalsWord,
  devApi,
  devStore,
  DevModeError,
  DevModeState,
  initialSwitch,
  switchLook,
  switchReducer,
} from '../devMode';

export function useReducedMotion(): boolean {
  const query = typeof window !== 'undefined' && typeof window.matchMedia === 'function' ? window.matchMedia('(prefers-reduced-motion: reduce)') : null;
  const [reduced, setReduced] = useState(() => !!query?.matches);
  useEffect(() => {
    if (!query) return;
    const on = () => setReduced(query.matches);
    query.addEventListener?.('change', on);
    return () => query.removeEventListener?.('change', on);
  }, [query]);
  return reduced;
}

/** Whether Dev mode is on in a space (the open one when no id is given), live. */
export function useDevModeOn(space?: string | null): boolean {
  return useSyncExternalStore(
    devStore.subscribe,
    () => devStore.isOn(space === undefined ? devStore.current() : space),
    () => false,
  );
}

export function useDevMode(space: string | null | undefined): DevModeState | null {
  return useSyncExternalStore(devStore.subscribe, () => devStore.get(space), () => null);
}

export interface DevModeSwitchProps {
  space: { id: string; name: string; approvals?: string };
  phone?: boolean;
  /** Dev mode went off: the sidebar shows the turned-off note. */
  onTurnedOff?: () => void;
  /** The person turned it on here (flow F3): the sidebar shows the toast. Never for a space already on. */
  onTurnedOn?: (s: DevModeState) => void;
  /** In the space controls: the row's label and sub-line name and describe the switch (spec §5). */
  labelledBy?: string;
  describedBy?: string;
  /** Armed (press 1 of 2) and not-the-owner, for the row's sub-line and the explainer. */
  onLook?: (look: { armed: boolean; disabled: boolean; on: boolean; failed: boolean }) => void;
  /** The button, so the explainer's "Turn on Dev mode" can press it and give it focus. */
  buttonRef?: React.MutableRefObject<HTMLButtonElement | null>;
  /** Test hooks: start in a given state, and a clock. */
  initial?: Partial<SwitchState>;
  now?: () => number;
}

/** The last on/off seen per space, kept in this browser only (no repo, no token): the switch's first frame. */
export const lastOn = {
  key: (space: string) => `kw-dev-on:${space}`,
  /** null: never seen here (a fresh browser), so the switch waits for the server instead of guessing. */
  get(space: string): boolean | null {
    try {
      const v = localStorage.getItem(lastOn.key(space));
      return v === '1' ? true : v === '0' ? false : null;
    } catch {
      return null;
    }
  },
  set(space: string, on: boolean) {
    try {
      localStorage.setItem(lastOn.key(space), on ? '1' : '0');
    } catch {
      /* private mode: the switch waits for the server on each load */
    }
  },
};

export const DevModeSwitch: React.FC<DevModeSwitchProps> = ({ space, phone = false, onTurnedOff, onTurnedOn, labelledBy, describedBy, onLook, buttonRef, initial, now = Date.now }) => {
  const reduced = useReducedMotion();
  const known = useDevMode(space.id);
  // Until this space's state loads, start from the last value seen here, so a space that is on doesn't
  // show "off" for a moment on every load.
  const [state, setState] = useState<SwitchState>(() =>
    initialSwitch({ reduced, ...(known ? { on: known.on } : lastOn.get(space.id) ? { on: true } : {}), ...initial }),
  );
  // Never seen here and not loaded yet: neither on nor off (no node on the graph, busy) until dev_mode
  // answers, so an on space never reads "off" for a moment (QA F8-R3-1).
  const [loading, setLoading] = useState(() => !known && !initial && lastOn.get(space.id) === null);
  // The first load failed: the row's sub-line stops waiting for a repo list (addendum 3).
  const [failed, setFailed] = useState(false);
  // The reducer's truth lives in the ref, written only by dispatch. Copying `state` into it on render
  // would let a render that started before a dispatch (a store update mid-save) put an older state back.
  const ref = useRef(state);
  const btn = useRef<HTMLButtonElement | null>(null);

  const dispatch = useCallback(
    (a: SwitchAction): SwitchState => {
      const next = switchReducer(ref.current, a, space.name);
      ref.current = next;
      setState(next);
      return next;
    },
    [space.name],
  );

  useEffect(() => void dispatch({ type: 'reduced', reduced }), [reduced, dispatch]);

  // Load this space's state; a 403 is a bot or not the owner: the switch shows disabled.
  useEffect(() => {
    let live = true;
    devApi.get(space.id).then(
      (s) => {
        if (!live) return;
        devStore.set(space.id, s);
      },
      (e: unknown) => {
        if (!live) return;
        setLoading(false);
        setFailed(true);
        dispatch({ type: 'server', state: null, disabled: e instanceof DevModeError && (e.status === 403 || e.status === 404) });
      },
    );
    return () => {
      live = false;
    };
  }, [space.id, dispatch]);

  // The store changes (the load above, SSE, Settings): follow it.
  useEffect(() => {
    if (!known) return;
    setLoading(false);
    dispatch({ type: 'server', state: known });
    lastOn.set(space.id, known.on);
  }, [known, dispatch, space.id]);

  // A lifted cover drops back after 5 s; a flip settles at 450 ms.
  useEffect(() => {
    if (state.phase === 'lifted' && state.liftedAt !== null) {
      const t = setTimeout(() => dispatch({ type: 'recover', now: now() }), Math.max(0, state.liftedAt + RECOVER_MS - now()));
      return () => clearTimeout(t);
    }
    if (state.phase === 'settling') {
      const t = setTimeout(() => dispatch({ type: 'settled', now: now() }), Math.max(0, state.settleUntil - now()));
      return () => clearTimeout(t);
    }
    return undefined;
  }, [state.phase, state.liftedAt, state.settleUntil, dispatch, now]);

  const save = useCallback(
    (on: boolean) => {
      const was = devStore.get(space.id);
      devApi.put(space.id, { on }).then(
        (s) => {
          devStore.set(space.id, s);
          dispatch({ type: 'saved', on: s.on });
          if (!s.on && was?.on) onTurnedOff?.();
          if (s.on && on && !was?.on) onTurnedOn?.(s);
        },
        (e: unknown) => dispatch({ type: 'save_failed', error: e instanceof Error ? e.message : 'Dev mode couldn’t be changed.' }),
      );
    },
    [space.id, dispatch, onTurnedOff, onTurnedOn],
  );

  const press = () => {
    const before = ref.current;
    const next = dispatch({ type: 'press', now: now() });
    if (next.saving && !before.saving) save(next.on);
  };

  const look = switchLook(state);
  const descId = useId();
  const armed = look.look === 'armed';
  const disabled = state.disabled;
  useEffect(() => onLook?.({ armed, disabled, on: state.on, failed }), [onLook, armed, disabled, state.on, failed]);
  const setBtn = useCallback(
    (el: HTMLButtonElement | null) => {
      btn.current = el;
      if (buttonRef) buttonRef.current = el;
    },
    [buttonRef],
  );

  // Switch v3 (spec §3): the state word, then a 40×22 track with a 16 px knob. Off: a hollow node at the
  // left. On: the knob at the right with a pip, a small lock in the track. Armed: amber ring, open lock,
  // "Off?". Not colour alone: the word, the knob's side and the lock all change.
  const word = loading ? '' : armed ? 'Off?' : state.on ? 'On' : 'Off';
  const button = (
    <button
      ref={setBtn}
      type="button"
      onClick={press}
      onKeyDown={(e: React.KeyboardEvent) => {
        if (e.key === 'Escape' && ref.current.phase === 'lifted') {
          e.stopPropagation();
          dispatch({ type: 'escape' });
        }
      }}
      onBlur={() => dispatch({ type: 'blur' })}
      // Not the owner: still focusable so the reason (title) can be read; presses do nothing (reducer).
      disabled={loading}
      aria-disabled={disabled || undefined}
      aria-busy={loading || state.saving || undefined}
      role="switch"
      aria-checked={state.on}
      {...(labelledBy ? { 'aria-labelledby': labelledBy } : { 'aria-label': look.label })}
      aria-describedby={describedBy ?? descId}
      title={look.tooltip}
      data-dev-switch={loading ? 'loading' : state.phase}
      data-look={look.look}
      data-on={state.on || undefined}
      data-ring={look.ring || undefined}
      data-knob={look.knobRight ? 'right' : 'left'}
      data-cover={look.coverOpen ? 'open' : 'closed'}
      data-armed={armed || undefined}
      // Answer 1 (0.7.2): the only "busy" sign is this spinner, and only while the PUT is in flight.
      data-saving={state.saving || undefined}
      data-covered={(look.ring && !armed) || undefined}
      className={`kw-dv${state.reduced ? ' kw-dv-rm' : ''}${phone ? ' h-11' : ''}`}
    >
      <span className="kw-dv-val" aria-hidden="true" data-dv-word>
        {word}
      </span>
      <span className="kw-dv-track" aria-hidden="true" data-dv-track>
        {look.glyph === 'locked' && state.on && <Lock size={10} strokeWidth={2.5} className="kw-dv-lock" data-dv-glyph="locked" />}
        {look.glyph === 'locked' && !state.on && <span className="sr-only" data-dv-glyph="locked" />}
        {look.glyph === 'unlocked' && <LockOpen size={10} strokeWidth={2.5} className="kw-dv-lock" data-dv-glyph="unlocked" />}
        <span className="kw-dv-knob" data-dv-knob style={loading ? { visibility: 'hidden' } : undefined} />
      </span>
    </button>
  );

  return (
    <span className={`relative inline-flex shrink-0 items-center justify-center ${phone ? 'min-w-[44px] min-h-[44px]' : ''}`} data-dev-switch-wrap>
      {button}
      <span id={descId} className="sr-only" data-dv-describe>
        {look.describe}
      </span>
      <span className="sr-only" aria-live="polite" data-dv-live>
        {state.announce ? <span key={state.announce.seq}>{state.announce.text}</span> : null}
      </span>
      {state.error && !state.saving && state.phase === 'covered' && (
        <span role="alert" className="absolute right-0 top-full mt-1.5 z-30 w-56 max-w-[calc(100vw-32px)] rounded-lg border border-rose-300/30 bg-surface-overlay px-2.5 py-1.5 text-meta text-bad">
          {state.error}
        </span>
      )}
    </span>
  );
};

/**
 * The flip note under the space header (style v2 §10). On reads `HEAD → dev` + "Dev mode is on for
 * {space}." (the autodetect build shows the on toast instead). Off (flow copy §7) reads `HEAD → main`
 * + the 0.7.0 text, with Turn back on (F8), one press. role=status, 10 s or until dismissed; the
 * timer pauses while the pointer is over it or focus is inside it.
 */
export const DevFlipNote: React.FC<{ on: boolean; space?: string; approvals?: string; onDismiss: () => void; ms?: number; onTurnBackOn?: () => Promise<unknown> }> = ({
  on,
  space,
  approvals,
  onDismiss,
  ms = 10_000,
  onTurnBackOn,
}) => {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [held, setHeld] = useState(false);
  useEffect(() => {
    if (busy || error || held) return undefined;
    const t = setTimeout(onDismiss, ms);
    return () => clearTimeout(t);
  }, [onDismiss, ms, busy, error, held]);
  const back = () => {
    if (!onTurnBackOn || busy) return;
    setBusy(true);
    setError(null);
    onTurnBackOn().then(onDismiss, (e: unknown) => {
      setBusy(false);
      setError(e instanceof Error ? e.message : 'Dev mode couldn’t be turned back on.');
    });
  };
  return (
    <div
      role="status"
      data-dev-flip-note={on ? 'on' : 'off'}
      {...(on ? {} : { 'data-dev-off-note': '' })}
      onPointerEnter={() => setHeld(true)}
      onPointerLeave={() => setHeld(false)}
      onFocus={() => setHeld(true)}
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setHeld(false);
      }}
      className="kw-dv-note mt-1.5 flex items-start gap-2 rounded-xl border border-white/14 bg-surface-overlay px-2.5 py-2 text-meta text-zinc-200 shadow-raised"
    >
      <div className="min-w-0 flex-1 leading-snug">
        <p>
          <span className="font-mono font-semibold text-sky-300 whitespace-nowrap">{on ? 'HEAD → dev' : 'HEAD → main'}</span>
          {on && <span className="dv-caret" aria-hidden="true" />}{' '}
          {on ? (
            `Dev mode is on for ${space ?? 'this space'}.`
          ) : (
            <>
              <strong className="font-semibold text-zinc-100">Dev mode is off.</strong> Reviews, the ship gate and the production floor stopped. Approvals stay on {approvalsWord(approvals)}. What
              landed keeps its history.
            </>
          )}
        </p>
        {!on && onTurnBackOn && (
          <button
            type="button"
            onClick={back}
            aria-disabled={busy || undefined}
            data-dev-turn-back-on
            className="mt-1.5 inline-flex items-center min-h-[44px] sm:min-h-[28px] -ml-1 px-2 rounded-lg text-meta font-semibold text-sky-300 hover:bg-sky-500/10 aria-disabled:opacity-60 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400"
          >
            {busy ? 'Turning on…' : 'Turn back on'}
          </button>
        )}
        {error && <p className="mt-1 text-bad">{error}</p>}
      </div>
      <button type="button" onClick={onDismiss} aria-label="Dismiss" className="-m-1 p-1 shrink-0 rounded text-zinc-400 hover:text-zinc-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400">
        <X size={13} aria-hidden="true" />
      </button>
    </div>
  );
};

/** The turned-off note (kept for callers): the flip note's off side. */
export const DevOffNote: React.FC<{ approvals?: string; onDismiss: () => void; ms?: number; onTurnBackOn?: () => Promise<unknown> }> = (p) => <DevFlipNote on={false} {...p} />;

/** The </> badge with its caret on a space tile; `behind` is the surface it sits on. */
export const DevBadge: React.FC<{ behind?: string }> = ({ behind }) => (
  <span className="kw-dev-badge" aria-hidden="true" data-dev-badge style={behind ? ({ ['--kw-dev-behind' as string]: behind } as React.CSSProperties) : undefined}>
    &lt;/&gt;
    <span className="kw-dev-caret" />
  </span>
);

/** The hard hat: 10×8 pixels, zinc-100 with a 2 px sky-400 ridge. */
const HAT: Array<[number, number, number, string]> = [
  [3, 0, 4, '#ecebfa'],
  [2, 1, 6, '#ecebfa'],
  [1, 2, 8, '#ecebfa'],
  [1, 3, 8, '#ecebfa'],
  [1, 4, 8, '#ecebfa'],
  [0, 5, 10, '#ecebfa'],
  [0, 6, 10, '#ecebfa'],
  [4, 0, 2, '#9d8fff'],
  [4, 1, 2, '#9d8fff'],
  [4, 2, 2, '#9d8fff'],
  [4, 3, 2, '#9d8fff'],
  [4, 4, 2, '#9d8fff'],
];

export const HardHat: React.FC = () => (
  <span className="kw-dev-hat" title="Working under Dev mode rules" data-dev-hat>
    <svg width="10" height="8" viewBox="0 0 10 8" aria-hidden="true" style={{ shapeRendering: 'crispEdges' }}>
      {HAT.map(([x, y, w, fill], i) => (
        <rect key={i} x={x} y={y} width={w} height={1} fill={fill} />
      ))}
    </svg>
    <span className="sr-only">Working under Dev mode rules</span>
  </span>
);
