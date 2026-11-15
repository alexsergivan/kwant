// The open chat's streaming reply, outside React state (14b smooth-streaming).
//
// Tokens arrive one event at a time, up to dozens a second. Holding the text in App state re-rendered the
// whole app and every finished message on each token. Here the text lives in a small store: only the live
// reply's body (and the thinking line) subscribe to it, and token updates reach them at most once per
// animation frame. Everything else reads `writing` (any text yet?), which changes twice per reply.

import { useSyncExternalStore } from 'react';

export interface StreamView {
  /** The reply so far, as streamed. */
  text: string;
  /** The latest thinking line. */
  thinking: string;
}

export interface StreamText {
  get(): StreamView;
  subscribe(listener: () => void): () => void;
  /** Shows `next` now, dropping any update still waiting for its frame (a reset, a chat switch). */
  show(next: Partial<StreamView>): void;
  /**
   * Shows what `read` returns at the next animation frame, or after `FALLBACK_MS` when frames don't come
   * (a hidden tab). Calls before the frame only move the reader: one update per frame.
   */
  schedule(read: () => Partial<StreamView>): void;
  /** Applies a waiting update now (the stream ended). */
  flush(): void;
}

/** A hidden tab gets no animation frames; the stream still moves on this timer. */
export const FALLBACK_MS = 50;

const EMPTY: StreamView = Object.freeze({ text: '', thinking: '' });

export interface StreamTextOptions {
  /** Runs after each scheduled (not `show`) update with the old and new view, e.g. the typing blip. */
  onFrame?: (prev: StreamView, next: StreamView) => void;
  /** Frame and timer functions; the window's by default (tests pass their own). */
  frame?: (cb: () => void) => number;
  cancelFrame?: (id: number) => void;
}

export function createStreamText(initial: Partial<StreamView> = {}, opts: StreamTextOptions = {}): StreamText {
  let view: StreamView = initial.text || initial.thinking ? { ...EMPTY, ...initial } : EMPTY;
  const listeners = new Set<() => void>();
  let reader: (() => Partial<StreamView>) | null = null;
  let frameId: number | null = null;
  let timerId: ReturnType<typeof setTimeout> | null = null;
  const raf =
    opts.frame ?? (typeof requestAnimationFrame === 'function' ? (cb: () => void) => requestAnimationFrame(() => cb()) : null);
  const caf = opts.cancelFrame ?? (typeof cancelAnimationFrame === 'function' ? cancelAnimationFrame : null);

  const cancel = () => {
    if (frameId !== null && caf) caf(frameId);
    if (timerId !== null) clearTimeout(timerId);
    frameId = timerId = null;
    reader = null;
  };
  /** Sets the view; true when it changed. */
  const apply = (next: Partial<StreamView>): boolean => {
    const text = next.text ?? view.text;
    const thinking = next.thinking ?? view.thinking;
    if (text === view.text && thinking === view.thinking) return false;
    view = text || thinking ? { text, thinking } : EMPTY;
    listeners.forEach((l) => l());
    return true;
  };
  const flush = () => {
    const read = reader;
    cancel();
    if (!read) return;
    const prev = view;
    if (apply(read())) opts.onFrame?.(prev, view);
  };

  return {
    get: () => view,
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    show(next) {
      cancel();
      apply(next);
    },
    schedule(read) {
      reader = read;
      if (frameId !== null || timerId !== null) return;
      if (raf) frameId = raf(flush);
      timerId = setTimeout(flush, FALLBACK_MS);
    },
    flush,
  };
}

/** A chat with nothing streaming: always empty, never changes. */
export const NO_STREAM: StreamText = {
  get: () => EMPTY,
  subscribe: () => () => {},
  show: () => {},
  schedule: () => {},
  flush: () => {},
};

/** One part of the stream, re-rendering when that part changes. Only the live reply's body and thinking line use this. */
export const useStreamPart = (s: StreamText, part: keyof StreamView): string => {
  const get = () => s.get()[part];
  return useSyncExternalStore(s.subscribe, get, get);
};

/** Whether any reply text has arrived yet; re-renders only when that flips. */
export const useStreamWriting = (s: StreamText): boolean => {
  const get = () => !!s.get().text;
  return useSyncExternalStore(s.subscribe, get, get);
};

/** Whether a thinking line is showing; re-renders only when that flips. */
export const useStreamThinking = (s: StreamText): boolean => {
  const get = () => !!s.get().thinking;
  return useSyncExternalStore(s.subscribe, get, get);
};
