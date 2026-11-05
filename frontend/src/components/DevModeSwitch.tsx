// The Dev mode switch in the sidebar space header: a safety cover over a launch switch (design §1).
// Press 1 lifts the cover, press 2 flips (or opens the setup sheet the first time). A lifted cover
// drops back after 5 s, on Esc or when focus leaves. Presses are ignored while a flip settles.
// It's a <button>, not role="switch": a switch promises one press.

import React, { useCallback, useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { Code, Lock, LockOpen, X } from 'lucide-react';
import {
  DROP_MS,
  RECOVER_MS,
  SwitchAction,
  SwitchState,
  approvalsWord,
  devApi,
  devStore,
  DevModeError,
  DevModeState,
  initialSwitch,
  needsSetup,
  switchLook,
  switchReducer,
} from '../devMode';
import { DevModeSetup } from './DevModeSetup';

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
  /** Test hooks: start in a given state, and a clock. */
  initial?: Partial<SwitchState>;
  now?: () => number;
}

export const DevModeSwitch: React.FC<DevModeSwitchProps> = ({ space, phone = false, onTurnedOff, initial, now = Date.now }) => {
  const reduced = useReducedMotion();
  const known = useDevMode(space.id);
  const [state, setState] = useState<SwitchState>(() =>
    initialSwitch({ reduced, ...(known ? { on: known.on, setup: needsSetup(known) } : {}), ...initial }),
  );
  // The reducer's truth lives in the ref, written only by dispatch. Copying `state` into it on render
  // would let a render that started before a dispatch (a store update mid-save) put an older state back.
  const ref = useRef(state);
  const btn = useRef<HTMLButtonElement>(null);

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
        dispatch({ type: 'server', state: null, disabled: e instanceof DevModeError && (e.status === 403 || e.status === 404) });
      },
    );
    return () => {
      live = false;
    };
  }, [space.id, dispatch]);

  // The store changes (the load above, SSE, Settings): follow it.
  useEffect(() => {
    if (known) dispatch({ type: 'server', state: known });
  }, [known, dispatch]);

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
        },
        (e: unknown) => dispatch({ type: 'save_failed', error: e instanceof Error ? e.message : 'Dev mode couldn’t be changed.' }),
      );
    },
    [space.id, dispatch, onTurnedOff],
  );

  const press = () => {
    const before = ref.current;
    const next = dispatch({ type: 'press', now: now() });
    if (next.saving && !before.saving) save(next.on);
  };

  const look = switchLook(state);
  const hint = look.hint && (
    <span
      role="presentation"
      className={
        phone
          ? 'kw-dv-hint fixed left-1/2 -translate-x-1/2 bottom-[92px] z-40 whitespace-nowrap rounded-lg border border-white/14 bg-surface-overlay px-2.5 py-1.5 text-meta text-zinc-100 shadow-raised'
          : 'kw-dv-hint absolute right-0 top-full mt-1.5 z-30 whitespace-nowrap rounded-lg border border-white/14 bg-surface-overlay px-2.5 py-1.5 text-meta text-zinc-100 shadow-raised'
      }
      aria-hidden="true"
      data-dv-hint
    >
      {look.hint}
    </span>
  );

  const common = {
    ref: btn,
    type: 'button' as const,
    onClick: press,
    onKeyDown: (e: React.KeyboardEvent) => {
      if (e.key === 'Escape' && ref.current.phase === 'lifted') {
        e.stopPropagation();
        dispatch({ type: 'escape' });
      }
    },
    onBlur: () => dispatch({ type: 'blur' }),
    disabled: state.disabled,
    'aria-label': look.label,
    title: look.tooltip,
    'data-dev-switch': state.phase,
    'data-on': state.on || undefined,
    'data-ring': look.ring || undefined,
    'data-knob': look.knobRight ? 'right' : 'left',
    'data-cover': look.coverOpen ? 'open' : 'closed',
    style: { ['--kw-dv-fill' as string]: String(look.fill) } as React.CSSProperties,
  };

  const button = state.reduced ? (
    <button {...common} className="kw-dv-rm focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400">
      <span className="text-meta font-semibold text-zinc-300" aria-hidden="true">
        Dev
      </span>
      <span className="kw-dv-track" aria-hidden="true">
        <span className="kw-dv-fill" />
        <span className="kw-dv-knob">
          {state.phase === 'lifted' || state.phase === 'pending' ? (
            <LockOpen size={8} strokeWidth={2.6} className="text-sky-600" data-dv-glyph="unlocked" />
          ) : (
            <Lock size={8} strokeWidth={2.6} className="text-sky-600" data-dv-glyph="locked" />
          )}
        </span>
      </span>
    </button>
  ) : (
    <button {...common} className="kw-dv focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400">
      <span className="kw-dv-track" aria-hidden="true">
        <span className="kw-dv-fill" />
        <span className="kw-dv-ring" />
        <span className="kw-dv-knob" />
      </span>
      <span className="kw-dv-cover" aria-hidden="true">
        &lt;/&gt;
      </span>
      <span className="kw-dv-hinge" aria-hidden="true" />
    </button>
  );

  return (
    <span className={`relative inline-flex shrink-0 items-center justify-center ${phone ? 'min-w-[44px] min-h-[44px]' : ''}`} data-dev-switch-wrap>
      {button}
      {hint}
      <span className="sr-only" aria-live="polite" data-dv-live>
        {state.announce ? <span key={state.announce.seq}>{state.announce.text}</span> : null}
      </span>
      {state.error && !state.saving && state.phase === 'covered' && (
        <span role="alert" className="absolute right-0 top-full mt-1.5 z-30 w-56 rounded-lg border border-rose-300/30 bg-surface-overlay px-2.5 py-1.5 text-meta text-bad">
          {state.error}
        </span>
      )}
      {state.phase === 'pending' && (
        <DevModeSetup
          space={space}
          initial={known}
          onCancel={() => {
            dispatch({ type: 'setup_cancel' });
            // The cover drops before focus returns, so the ring isn't drawn on a moving switch.
            setTimeout(() => btn.current?.focus(), state.reduced ? 0 : DROP_MS);
          }}
          onDone={(s) => {
            devStore.set(space.id, s);
            dispatch({ type: 'setup_done', now: now() });
            dispatch({ type: 'saved', on: s.on });
            btn.current?.focus();
          }}
        />
      )}
    </span>
  );
};

/** "Dev mode is off." under the space header: role=status, 10 s or until dismissed. */
export const DevOffNote: React.FC<{ approvals?: string; onDismiss: () => void; ms?: number }> = ({ approvals, onDismiss, ms = 10_000 }) => {
  useEffect(() => {
    const t = setTimeout(onDismiss, ms);
    return () => clearTimeout(t);
  }, [onDismiss, ms]);
  return (
    <div role="status" data-dev-off-note className="mt-1.5 flex items-start gap-2 rounded-xl border border-white/14 bg-surface-raised px-2.5 py-2 text-meta text-zinc-300">
      <Code size={13} className="mt-px shrink-0 text-zinc-400" aria-hidden="true" />
      <p className="min-w-0 flex-1">
        <strong className="font-semibold text-zinc-100">Dev mode is off.</strong> Reviews, the ship gate and the production floor stopped. Approvals stay on {approvalsWord(approvals)}. What
        landed keeps its history.
      </p>
      <button type="button" onClick={onDismiss} aria-label="Dismiss" className="-m-1 p-1 shrink-0 rounded text-zinc-400 hover:text-zinc-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400">
        <X size={13} aria-hidden="true" />
      </button>
    </div>
  );
};

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
