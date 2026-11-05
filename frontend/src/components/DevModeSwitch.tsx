// The Dev mode switch in the sidebar space header (style v2 §10, flow F1/F2/F9): the Branch switch.
// Off has no cover: one press turns it on (F1). On is covered by a 1 px violet ring and a lock; turning
// it off takes two presses (F2): press 1 arms (2 px amber ring, open lock, `off?`), press 2 within 5 s
// turns it off. Armed drops back after 5 s, on Esc or when focus leaves. Presses are ignored while a
// flip settles (450 ms), so a double-click on Off can't turn it on and arm it.

import React, { useCallback, useEffect, useId, useRef, useState, useSyncExternalStore } from 'react';
import { Lock, LockOpen, X } from 'lucide-react';
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
  /** Dev mode went on: the sidebar shows the HEAD → dev note. */
  onTurnedOn?: () => void;
  /** Test hooks: start in a given state, and a clock. */
  initial?: Partial<SwitchState>;
  now?: () => number;
}

export const DevModeSwitch: React.FC<DevModeSwitchProps> = ({ space, phone = false, onTurnedOff, onTurnedOn, initial, now = Date.now }) => {
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
          if (s.on && !was?.on) onTurnedOn?.();
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
  const hint = look.hint && (
    <span
      role="presentation"
      className={
        phone
          ? 'kw-dv-hint fixed inset-x-4 mx-auto w-fit max-w-[calc(100vw-32px)] text-center leading-snug bottom-[calc(56px+env(safe-area-inset-bottom)+12px)] z-40 rounded-lg border border-white/14 bg-surface-overlay px-2.5 py-1.5 text-meta text-zinc-100 shadow-raised'
          : 'kw-dv-hint absolute -right-9 top-full mt-1.5 z-30 whitespace-nowrap rounded-lg border border-white/14 bg-surface-overlay px-2.5 py-1.5 text-meta text-zinc-100 shadow-raised'
      }
      aria-hidden="true"
      data-dv-hint
    >
      {look.hint}
      {!phone && <kbd className="dv-kbd ml-1.5">Esc</kbd>}
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
    role: 'switch' as const,
    'aria-checked': state.on,
    'aria-label': look.label,
    'aria-describedby': descId,
    title: look.tooltip,
    'data-dev-switch': state.phase,
    'data-look': look.look,
    'data-on': state.on || undefined,
    'data-ring': look.ring || undefined,
    'data-knob': look.knobRight ? 'right' : 'left',
    'data-cover': look.coverOpen ? 'open' : 'closed',
    style: { ['--kw-dv-fill' as string]: String(look.fill) } as React.CSSProperties,
  };

  // The Branch switch (style v2 §10): a mono label and a tiny git graph. The node sits on the line
  // when off and hops onto the fork when on; then the cover (a 1 px violet ring, ::after) closes and a
  // lock shows. Armed (only ever while on) lifts it: 2 px amber ring, open lock, the fork dashed amber
  // with a halo on the node, `off?`. Reduced motion keeps the same shapes, instantly (kw-dv-rm).
  const armed = look.look === 'armed';
  const word = armed ? 'off?' : 'dev';
  const button = (
    <button
      {...common}
      data-armed={armed || undefined}
      data-covered={(look.ring && !armed) || undefined}
      className={`kw-dv${state.reduced ? ' kw-dv-rm' : ''}${phone ? ' h-11' : ''} focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400`}
    >
      {look.glyph === 'locked' && <Lock size={10} strokeWidth={2.5} className="kw-dv-lock" aria-hidden="true" data-dv-glyph="locked" />}
      {look.glyph === 'unlocked' && <LockOpen size={10} strokeWidth={2.5} className="kw-dv-lock" aria-hidden="true" data-dv-glyph="unlocked" />}
      <span className="kw-dv-lbl" aria-hidden="true" data-dv-word>
        {word}
      </span>
      <svg className="kw-dv-graph" viewBox="0 0 44 24" aria-hidden="true" data-dv-graph>
        <path className="kw-dv-base" d="M2 17H42" />
        <path className="kw-dv-fork" d="M12 17C19 17 18 7 26 7H42" />
        <g className="kw-dv-node">
          <circle className="kw-dv-halo" cx="12" cy="17" r="5" />
          <circle className="kw-dv-dot" cx="12" cy="17" r="5" />
          <circle className="kw-dv-pip" cx="12" cy="17" r="1.6" />
        </g>
      </svg>
    </button>
  );

  return (
    <span className={`relative inline-flex shrink-0 items-center justify-center ${phone ? 'min-w-[44px] min-h-[44px]' : ''}`} data-dev-switch-wrap>
      {button}
      {hint}
      <span id={descId} className="sr-only" data-dv-describe>
        {look.describe}
      </span>
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
            if (s.on) onTurnedOn?.();
            btn.current?.focus();
          }}
        />
      )}
    </span>
  );
};

/**
 * The flip note under the space header (style v2 §10): on reads `HEAD → dev` + "Dev mode is on for
 * {space}." with one caret blink; off reads `merged into main` + "Dev mode is off. Approvals stay on
 * {approvals}." role=status, 10 s or until dismissed.
 */
export const DevFlipNote: React.FC<{ on: boolean; space?: string; approvals?: string; onDismiss: () => void; ms?: number }> = ({ on, space, approvals, onDismiss, ms = 10_000 }) => {
  useEffect(() => {
    const t = setTimeout(onDismiss, ms);
    return () => clearTimeout(t);
  }, [onDismiss, ms]);
  return (
    <div role="status" data-dev-flip-note={on ? 'on' : 'off'} {...(on ? {} : { 'data-dev-off-note': '' })} className="kw-dv-note mt-1.5 flex items-start gap-2 rounded-xl border border-white/14 bg-surface-overlay px-2.5 py-2 text-meta text-zinc-200 shadow-raised">
      <p className="min-w-0 flex-1 leading-snug">
        <span className="font-mono font-semibold text-sky-300 whitespace-nowrap">{on ? 'HEAD → dev' : 'merged into main'}</span>
        {on && <span className="dv-caret" aria-hidden="true" />}{' '}
        {on ? `Dev mode is on for ${space ?? 'this space'}.` : `Dev mode is off. Approvals stay on ${approvalsWord(approvals)}.`}
      </p>
      <button type="button" onClick={onDismiss} aria-label="Dismiss" className="-m-1 p-1 shrink-0 rounded text-zinc-400 hover:text-zinc-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400">
        <X size={13} aria-hidden="true" />
      </button>
    </div>
  );
};

/** The turned-off note (kept for callers): the flip note's off side. */
export const DevOffNote: React.FC<{ approvals?: string; onDismiss: () => void; ms?: number }> = (p) => <DevFlipNote on={false} {...p} />;

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
