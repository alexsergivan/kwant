// The dictation mic in the message box (Mira, autonomy/dictation/design.md): the hook that records and
// turns speech into text, the mic button, the one dictation line inside the box, and the "isn't set up"
// popover. The server does the speech work (#610); nothing goes to a bot before Send.
import { useCallback, useEffect, useReducer, useRef, useState, type RefObject } from 'react';
import { AlertCircle, Check, Loader2, Mic, MicOff, RotateCcw, Square, Undo2, X } from 'lucide-react';
import {
  ADDED_MS,
  HIDDEN_MS,
  INSECURE_WHY,
  MAX_SECONDS,
  NO_WORDS,
  SILENCE_LEVEL,
  SILENCE_MS,
  WARN_SECONDS,
  barScales,
  browserSupports,
  cachedAvailability,
  canRetry,
  clock,
  getAvailability,
  insertDictated,
  langHint,
  micErrorText,
  micHidden,
  micVisible,
  onDictationChange,
  rmsLevel,
  setAvailability,
  setMicHidden,
  transcribe,
  undoDictated,
  wordCount,
  wordsLabel,
  type Inserted,
  type MicView,
} from '../dictation';

export type DictPhase = 'idle' | 'asking' | 'recording' | 'transcribing';

export type DictLine =
  | { kind: 'asking' }
  | { kind: 'recording' }
  | { kind: 'transcribing'; seconds: number }
  | { kind: 'added'; words: number; ins: Inserted }
  | { kind: 'bad'; text: string; retry: boolean }
  | { kind: 'warn'; text: string };

export interface DictationOptions {
  spaceId?: string | null;
  /** The open chat; a change stops and discards a recording. */
  chatKey?: string;
  phone: boolean;
  touch: boolean;
  /** The box's text now. */
  value: string;
  textareaRef: RefObject<HTMLTextAreaElement>;
  setValue: (v: string) => void;
  onToast?: (title: string) => void;
}

export interface Dictation {
  view: MicView;
  phase: DictPhase;
  /** Recording or turning into text: Send waits. */
  busy: boolean;
  line: DictLine | null;
  canUndo: boolean;
  silent: boolean;
  startedAt: number;
  levelRef: RefObject<number>;
  secure: boolean;
  why: string | null;
  popOpen: boolean;
  announce: { text: string; n: number };
  micRef: RefObject<HTMLButtonElement>;
  popRef: RefObject<HTMLDivElement>;
  toggle: () => void;
  cancel: () => void;
  retry: () => void;
  dismiss: () => void;
  undo: () => void;
  closePopover: (focusMic?: boolean) => void;
  hideMic: () => void;
  /** A keystroke in the box: the "Added" line goes. */
  noteTyping: () => void;
  /** Sent: the "Added" line goes. */
  noteSent: () => void;
}

interface Rec {
  stream: MediaStream;
  recorder: MediaRecorder;
  chunks: Blob[];
  ctx: AudioContext | null;
  raf: number;
  cap: ReturnType<typeof setTimeout>;
  startedAt: number;
}

const supportedNow = () => (typeof window === 'undefined' ? false : browserSupports(window as unknown as Parameters<typeof browserSupports>[0]));
const secureNow = () => typeof window === 'undefined' || window.isSecureContext !== false;

export function useDictation(o: DictationOptions): Dictation {
  const { spaceId, chatKey, phone, touch, value, textareaRef, setValue, onToast } = o;
  const [, bump] = useReducer((x: number) => x + 1, 0);
  const [phase, setPhaseState] = useState<DictPhase>('idle');
  const [line, setLine] = useState<DictLine | null>(null);
  const [silent, setSilent] = useState(false);
  const [startedAt, setStartedAt] = useState(0);
  const [popOpen, setPopOpen] = useState(false);
  const [announce, setAnnounceState] = useState({ text: '', n: 0 });
  const micRef = useRef<HTMLButtonElement>(null);
  const popRef = useRef<HTMLDivElement>(null);
  const levelRef = useRef(0);
  const recRef = useRef<Rec | null>(null);
  const abortRef = useRef<AbortController | null>(null);
  const blobRef = useRef<{ blob: Blob; seconds: number } | null>(null);
  const insertAtRef = useRef(0);
  const genRef = useRef(0);
  const phaseRef = useRef<DictPhase>('idle');
  const addedTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const valueRef = useRef(value);
  valueRef.current = value;
  const touchedRef = useRef(false);
  const live = useRef({ spaceId, phone, touch, setValue, onToast });
  live.current = { spaceId, phone, touch, setValue, onToast };

  const setPhase = (p: DictPhase) => {
    phaseRef.current = p;
    setPhaseState(p);
  };
  const say = (text: string) => setAnnounceState((a) => ({ text, n: a.n + 1 }));
  const focusMic = () => setTimeout(() => micRef.current?.focus({ preventScroll: true }), 0);
  const clearAdded = () => {
    if (addedTimer.current) clearTimeout(addedTimer.current);
    addedTimer.current = null;
  };

  // Availability: asked when the box mounts or the space changes, and again when the window gets focus.
  useEffect(() => onDictationChange(bump), []);
  useEffect(() => {
    if (!spaceId) return;
    void getAvailability(spaceId);
    const onFocus = () => void getAvailability(spaceId, { fresh: true });
    window.addEventListener('focus', onFocus);
    return () => window.removeEventListener('focus', onFocus);
  }, [spaceId]);

  // Where the words go: the caret, if the box ever had it; else the end.
  useEffect(() => {
    const ta = textareaRef.current;
    if (!ta) return;
    const on = () => {
      touchedRef.current = true;
    };
    ta.addEventListener('focus', on);
    return () => ta.removeEventListener('focus', on);
  }, [textareaRef]);

  const supported = supportedNow();
  const secure = secureNow();
  const avail = spaceId ? cachedAvailability(spaceId) : undefined;
  const view: MicView = spaceId ? micVisible({ supported, secure, availability: avail, hidden: micHidden(spaceId) }) : 'hidden';
  const why = !secure ? INSECURE_WHY : avail && avail !== 'error' ? avail.why ?? 'Add an OpenAI API key in Apps & accounts to dictate.' : null;

  /** Stops the microphone and the meter. `keep`: hand the recording on; else throw it away. */
  const stopRec = (keep: boolean) => {
    const r = recRef.current;
    if (!r) return;
    recRef.current = null;
    clearTimeout(r.cap);
    cancelAnimationFrame(r.raf);
    void r.ctx?.close().catch(() => {});
    const seconds = Math.max(0, (Date.now() - r.startedAt) / 1000);
    const finish = () => {
      const type = r.recorder.mimeType || r.chunks[0]?.type || 'audio/webm';
      void send(new Blob(r.chunks, { type }), seconds);
    };
    if (keep) r.recorder.onstop = finish;
    else r.recorder.onstop = null;
    let stopped = false;
    try {
      if (r.recorder.state !== 'inactive') {
        r.recorder.stop();
        stopped = true;
      }
    } catch {
      /* already stopped */
    }
    r.stream.getTracks().forEach((t) => t.stop());
    setSilent(false);
    levelRef.current = 0;
    if (keep) {
      setPhase('transcribing');
      setLine({ kind: 'transcribing', seconds });
      say('Turning speech into text.');
      if (!stopped) finish();
    }
  };

  const send = async (blob: Blob, seconds: number) => {
    const sid = live.current.spaceId;
    if (!sid) return;
    blobRef.current = { blob, seconds };
    abortRef.current?.abort();
    const abort = new AbortController();
    abortRef.current = abort;
    setPhase('transcribing');
    setLine({ kind: 'transcribing', seconds });
    let r;
    try {
      r = await transcribe(sid, blob, langHint(typeof navigator === 'undefined' ? null : navigator.language), abort.signal);
    } catch {
      return; // cancelled
    }
    if (abortRef.current !== abort) return;
    abortRef.current = null;
    setPhase('idle');
    if (r.ok) {
      const text = r.text.trim();
      blobRef.current = null;
      if (!text) {
        setLine({ kind: 'warn', text: NO_WORDS });
        say(NO_WORDS);
        focusMic();
        return;
      }
      const ins = insertDictated(valueRef.current, insertAtRef.current, text);
      valueRef.current = ins.value;
      live.current.setValue(ins.value);
      const n = wordCount(text);
      setLine({ kind: 'added', words: n, ins });
      say(`Added ${wordsLabel(n)} to the message.`);
      clearAdded();
      addedTimer.current = setTimeout(() => setLine((l) => (l?.kind === 'added' ? null : l)), ADDED_MS);
      const handTouch = live.current.phone && live.current.touch;
      setTimeout(() => {
        const ta = textareaRef.current;
        if (ta) {
          ta.style.height = 'auto';
          ta.style.height = `${Math.min(ta.scrollHeight, 160)}px`;
        }
        if (handTouch || !ta) micRef.current?.focus({ preventScroll: true });
        else {
          ta.focus();
          ta.setSelectionRange(ins.caret, ins.caret);
        }
      }, 0);
      return;
    }
    if (r.status === 409 && r.code === 'no_transcriber') {
      blobRef.current = null;
      setAvailability(sid, { available: false, source: null, why: r.why ?? r.error });
      setLine(null);
      setPopOpen(true);
      return;
    }
    const retry = canRetry(r.status, r.code);
    if (!retry) blobRef.current = null;
    setLine({ kind: 'bad', text: r.error, retry });
    focusMic();
  };

  const start = async () => {
    const sid = live.current.spaceId;
    if (!sid) return;
    clearAdded();
    blobRef.current = null;
    const ta = textareaRef.current;
    const v = valueRef.current;
    insertAtRef.current = ta && touchedRef.current ? Math.min(ta.selectionEnd ?? v.length, v.length) : v.length;
    const gen = ++genRef.current;
    setPhase('asking');
    setLine({ kind: 'asking' });
    let stream: MediaStream;
    try {
      stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    } catch (err) {
      if (gen !== genRef.current) return;
      setPhase('idle');
      const text = micErrorText((err as { name?: string })?.name, live.current.phone);
      setLine({ kind: 'warn', text });
      focusMic();
      return;
    }
    if (gen !== genRef.current) {
      stream.getTracks().forEach((t) => t.stop());
      return;
    }
    let recorder: MediaRecorder;
    try {
      recorder = new MediaRecorder(stream);
    } catch {
      stream.getTracks().forEach((t) => t.stop());
      setPhase('idle');
      setLine({ kind: 'warn', text: micErrorText('', live.current.phone) });
      focusMic();
      return;
    }
    const chunks: Blob[] = [];
    recorder.ondataavailable = (e) => {
      if (e.data && e.data.size > 0) chunks.push(e.data);
    };
    const now = Date.now();
    const rec: Rec = { stream, recorder, chunks, ctx: null, raf: 0, cap: setTimeout(() => stopRec(true), MAX_SECONDS * 1000), startedAt: now };
    recRef.current = rec;
    // The device going away (unplugged) stops and keeps what's there.
    stream.getAudioTracks().forEach((t) => t.addEventListener('ended', () => recRef.current === rec && stopRec(true)));
    // The level meter and the silence hint, at about 15 fps.
    try {
      const AC: typeof AudioContext | undefined = window.AudioContext ?? (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
      if (AC) {
        const ctx = new AC();
        const an = ctx.createAnalyser();
        an.fftSize = 1024;
        ctx.createMediaStreamSource(stream).connect(an);
        rec.ctx = ctx;
        const buf = new Float32Array(an.fftSize);
        let last = 0;
        let lastSound = now;
        let quiet = false;
        const tick = (t: number) => {
          if (recRef.current !== rec) return;
          if (t - last >= 66) {
            last = t;
            an.getFloatTimeDomainData(buf);
            const lvl = rmsLevel(buf);
            levelRef.current = lvl;
            const at = Date.now();
            if (lvl >= SILENCE_LEVEL) lastSound = at;
            const q = at - lastSound > SILENCE_MS;
            if (q !== quiet) {
              quiet = q;
              setSilent(q);
            }
          }
          rec.raf = requestAnimationFrame(tick);
        };
        rec.raf = requestAnimationFrame(tick);
      }
    } catch {
      /* no meter: the timer still says it's working */
    }
    recorder.start(1000);
    setStartedAt(now);
    setSilent(false);
    setPhase('recording');
    setLine({ kind: 'recording' });
    say('Listening. Press the mic or Escape to stop.');
  };

  /** Everything stops and nothing is kept (Cancel, chat change, hidden tab). */
  const discard = () => {
    genRef.current++;
    const was = phaseRef.current;
    stopRec(false);
    abortRef.current?.abort();
    abortRef.current = null;
    blobRef.current = null;
    setPhase('idle');
    setLine(null);
    setSilent(false);
    return was;
  };

  const toggle = () => {
    const p = phaseRef.current;
    if (p === 'transcribing' || p === 'asking') return;
    if (p === 'recording') return stopRec(true);
    if (view === 'muted') {
      setPopOpen((x) => !x);
      return;
    }
    if (view === 'ready') void start();
  };

  const cancel = () => {
    discard();
    say('Dictation cancelled.');
    micRef.current?.focus({ preventScroll: true });
  };

  const retry = () => {
    const b = blobRef.current;
    if (b) void send(b.blob, b.seconds);
  };

  const dismiss = () => {
    clearAdded();
    blobRef.current = null;
    setLine(null);
    micRef.current?.focus({ preventScroll: true });
  };

  const canUndo = line?.kind === 'added' && undoDictated(value, line.ins) !== null;
  const undo = () => {
    if (line?.kind !== 'added') return;
    const next = undoDictated(valueRef.current, line.ins);
    if (next === null) return;
    clearAdded();
    valueRef.current = next;
    setValue(next);
    setLine(null);
    say('Dictated words removed.');
    const at = line.ins.from;
    setTimeout(() => {
      const ta = textareaRef.current;
      if (ta) {
        ta.style.height = 'auto';
        ta.style.height = `${Math.min(ta.scrollHeight, 160)}px`;
      }
      if (live.current.phone && live.current.touch) micRef.current?.focus({ preventScroll: true });
      else if (ta) {
        ta.focus();
        ta.setSelectionRange(at, at);
      }
    }, 0);
  };

  const noteTyping = useCallback(() => {
    setLine((l) => (l?.kind === 'added' ? null : l));
  }, []);

  const closePopover = (toMic = true) => {
    setPopOpen(false);
    if (toMic) micRef.current?.focus({ preventScroll: true });
  };

  const hideMic = () => {
    if (spaceId) setMicHidden(spaceId, true);
    setPopOpen(false);
    say('Mic hidden. It comes back when dictation is set up.');
    setTimeout(() => textareaRef.current?.focus({ preventScroll: true }), 0);
  };

  // Esc anywhere stops and transcribes, before the composer's own Esc, unless a dialog or picker is open.
  useEffect(() => {
    if (phase !== 'recording') return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented) return;
      if (document.querySelector('[aria-modal="true"], #kw-mention-list, [data-slash-popover]')) return;
      e.preventDefault();
      e.stopPropagation();
      stopRec(true);
    };
    document.addEventListener('keydown', onKey, true);
    return () => document.removeEventListener('keydown', onKey, true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [phase]);

  // The tab hidden for 30 s while dictating: stop and throw it away.
  useEffect(() => {
    if (phase === 'idle') return;
    let t: ReturnType<typeof setTimeout> | null = null;
    const onVis = () => {
      if (document.visibilityState === 'hidden') {
        t = setTimeout(() => {
          if (discard() !== 'idle') live.current.onToast?.('Dictation stopped: you left the chat.');
        }, HIDDEN_MS);
      } else if (t) {
        clearTimeout(t);
        t = null;
      }
    };
    document.addEventListener('visibilitychange', onVis);
    return () => {
      document.removeEventListener('visibilitychange', onVis);
      if (t) clearTimeout(t);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [phase === 'idle']);

  // Leaving the chat or the space (or the box going away): stop and discard, and say so.
  useEffect(
    () => () => {
      setPopOpen(false);
      clearAdded();
      if (discard() !== 'idle') live.current.onToast?.('Dictation stopped: you left the chat.');
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [spaceId, chatKey],
  );

  // The popover: focus its first button; Esc or a click outside closes it.
  useEffect(() => {
    if (!popOpen) return;
    const t = setTimeout(() => popRef.current?.querySelector<HTMLButtonElement>('button')?.focus({ preventScroll: true }), 0);
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.preventDefault();
      e.stopPropagation();
      closePopover(true);
    };
    const onDown = (e: Event) => {
      const n = e.target as Node | null;
      if (n && (popRef.current?.contains(n) || micRef.current?.contains(n))) return;
      closePopover(false);
    };
    document.addEventListener('keydown', onKey, true);
    document.addEventListener('pointerdown', onDown, true);
    return () => {
      clearTimeout(t);
      document.removeEventListener('keydown', onKey, true);
      document.removeEventListener('pointerdown', onDown, true);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [popOpen]);

  // The popover only belongs to the muted mic.
  useEffect(() => {
    if (view !== 'muted' && popOpen) setPopOpen(false);
  }, [view, popOpen]);

  return {
    view,
    phase,
    busy: phase === 'recording' || phase === 'transcribing' || phase === 'asking',
    line,
    canUndo,
    silent,
    startedAt,
    levelRef,
    secure,
    why,
    popOpen: popOpen && view === 'muted',
    announce,
    micRef,
    popRef,
    toggle,
    cancel,
    retry,
    dismiss,
    undo,
    closePopover,
    hideMic,
    noteTyping,
    noteSent: noteTyping,
  };
}

// ---------------------------------------------------------------------------------------------------
// The mic button

export function MicButton({ d, phone }: { d: Dictation; phone: boolean }) {
  if (d.view === 'hidden') return null;
  const state = d.view === 'muted' ? 'muted' : d.phase === 'recording' ? 'rec' : d.phase === 'transcribing' ? 'busy' : 'ready';
  const size = phone ? 'w-11 h-11 focus-visible:outline-[#b8adff]' : 'w-11 h-11 sm:w-9 sm:h-9 focus-visible:outline-sky-400';
  const look =
    state === 'rec'
      ? 'bg-violet-500/[0.22] shadow-[inset_0_0_0_1px_rgb(167_139_250/0.55)] text-violet-200'
      : state === 'busy'
        ? 'text-violet-300 cursor-default'
        : state === 'muted'
          ? d.popOpen
            ? 'bg-white/[0.06] text-zinc-300'
            : phone
              ? 'text-[#6e6896] hover:bg-white/[0.06]'
              : 'text-zinc-500 hover:text-zinc-300 hover:bg-white/[0.08]'
          : phone
            ? 'text-[#a39dcc] hover:text-zinc-100 hover:bg-white/[0.06] active:bg-white/[0.06]'
            : 'text-zinc-300 hover:text-zinc-50 hover:bg-white/[0.08]';
  const icon = phone ? 18 : 17;
  return (
    <button
      ref={d.micRef}
      type="button"
      data-mic={state}
      onClick={d.toggle}
      aria-label={state === 'muted' ? 'Dictate (not set up)' : 'Dictate'}
      aria-pressed={state === 'muted' ? undefined : state === 'rec'}
      aria-haspopup={state === 'muted' ? 'dialog' : undefined}
      aria-expanded={state === 'muted' ? d.popOpen : undefined}
      aria-disabled={state === 'busy' || d.phase === 'asking' ? true : undefined}
      title={
        phone
          ? undefined
          : state === 'rec'
            ? 'Stop and turn it into text (Esc)'
            : state === 'busy'
              ? 'Turning speech into text…'
              : state === 'muted'
                ? 'Dictation isn’t set up in this space'
                : 'Dictate: speak, then edit before you send'
      }
      className={`${size} ${look} rounded-xl flex items-center justify-center flex-shrink-0 transition-colors focus-visible:outline focus-visible:outline-2`}
    >
      {state === 'rec' ? (
        <Square size={phone ? 13 : 12} fill="#ddd6fe" color="#ddd6fe" aria-hidden="true" />
      ) : state === 'busy' ? (
        <Loader2 size={icon} className="kw-dict-spin" aria-hidden="true" />
      ) : (
        <Mic size={icon} aria-hidden="true" />
      )}
    </button>
  );
}

// ---------------------------------------------------------------------------------------------------
// The dictation line: the form's first row while anything is happening

function LineButton({ phone, onClick, children, label }: { phone: boolean; onClick: () => void; children: React.ReactNode; label?: string }) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={label}
      className={`inline-flex items-center gap-1.5 flex-none rounded-[10px] border border-hairline-strong bg-white/[0.03] text-zinc-200 font-medium hover:bg-white/[0.08] hover:text-zinc-50 focus-visible:outline focus-visible:outline-2 ${
        phone ? 'min-h-11 px-3 text-[13px] focus-visible:outline-[#b8adff]' : 'min-h-7 px-2.5 text-[12px] focus-visible:outline-sky-400'
      }`}
    >
      {children}
    </button>
  );
}

function CloseButton({ phone, onClick }: { phone: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label="Dismiss"
      className={`${phone ? 'w-11 h-11 focus-visible:outline-[#b8adff]' : 'w-7 h-7 focus-visible:outline-sky-400'} flex-none rounded-lg grid place-items-center text-zinc-400 hover:text-zinc-200 hover:bg-white/[0.06] focus-visible:outline focus-visible:outline-2`}
    >
      <X size={14} aria-hidden="true" />
    </button>
  );
}

function Elapsed({ startedAt }: { startedAt: number }) {
  const [, tick] = useReducer((x: number) => x + 1, 0);
  useEffect(() => {
    const t = setInterval(tick, 250);
    return () => clearInterval(t);
  }, []);
  const s = (Date.now() - startedAt) / 1000;
  if (s >= WARN_SECONDS)
    return (
      <span data-dictation-time className="tabular-nums text-[#fcd34d] flex-none" aria-hidden="true">
        {clock(MAX_SECONDS - s)} left
      </span>
    );
  return (
    <span data-dictation-time className="tabular-nums text-zinc-400 flex-none" aria-hidden="true">
      {clock(s)}
    </span>
  );
}

function Level({ levelRef }: { levelRef: RefObject<number> }) {
  const bars = useRef<(HTMLElement | null)[]>([]);
  useEffect(() => {
    let raf = 0;
    let last = 0;
    const loop = (t: number) => {
      if (t - last >= 66) {
        last = t;
        const s = barScales(levelRef.current ?? 0);
        bars.current.forEach((b, i) => {
          if (b) b.style.transform = `scaleY(${s[i]})`;
        });
      }
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(raf);
  }, [levelRef]);
  return (
    <span data-dictation-level className="flex items-center gap-0.5 h-4 flex-none" aria-hidden="true">
      {[0, 1, 2, 3, 4].map((i) => (
        <i
          key={i}
          ref={(el) => {
            bars.current[i] = el;
          }}
          className="block w-[3px] h-4 rounded-sm bg-[#c4b5fd] origin-center"
          style={{ transform: 'scaleY(0.2)' }}
        />
      ))}
    </span>
  );
}

export function DictationLine({ d, phone, botName, reduced }: { d: Dictation; phone: boolean; botName: string; reduced: boolean }) {
  const l = d.line;
  if (!l) return null;
  const multi = l.kind === 'bad' || l.kind === 'warn';
  const box = `basis-full min-w-0 flex ${multi ? 'items-start pt-1' : 'items-center'} gap-2 ${
    phone ? 'min-h-11 text-[13px] pl-1.5' : 'min-h-8 text-[12.5px] pl-2'
  } pr-1 leading-[18px] text-zinc-300 border-b border-hairline pb-1.5 mb-0.5`;
  const lead = `flex ${multi ? 'items-start' : 'items-center'} gap-2 min-w-0 flex-1`;
  if (l.kind === 'asking')
    return (
      <div data-dictation-line="asking" className={box}>
        <span className="min-w-0">Allow the microphone in your browser’s prompt.</span>
      </div>
    );
  if (l.kind === 'recording')
    return (
      <div data-dictation-line="recording" className={box}>
        <span className={lead}>
          <span className="w-2 h-2 rounded-full bg-[#fb7185] shadow-[0_0_0_3px_rgb(251_113_133/0.18)] flex-none" aria-hidden="true" />
          <b className="font-semibold text-zinc-100 flex-none">Listening</b>
          <Elapsed startedAt={d.startedAt} />
          {!reduced && <Level levelRef={d.levelRef} />}
          {d.silent ? (
            <span data-dictation-silent className="text-[#fcd34d] truncate min-w-0">
              Can’t hear anything. Check the microphone.
            </span>
          ) : (
            !phone && <span className="text-zinc-400 truncate min-w-0">· Esc or the mic stops · nothing goes to {botName} until you press Send</span>
          )}
        </span>
        <LineButton phone={phone} onClick={d.cancel}>
          Cancel
        </LineButton>
      </div>
    );
  if (l.kind === 'transcribing')
    return (
      <div data-dictation-line="transcribing" className={box}>
        <span className={lead}>
          <Loader2 size={14} className="kw-dict-spin text-[#c4b5fd] flex-none" aria-hidden="true" />
          <b className="font-semibold text-zinc-100 truncate">Turning speech into text…</b>
          <span className="tabular-nums text-zinc-400 flex-none">{clock(l.seconds)} recorded</span>
        </span>
        <LineButton phone={phone} onClick={d.cancel}>
          Cancel
        </LineButton>
      </div>
    );
  if (l.kind === 'added')
    return (
      <div data-dictation-line="added" className={box}>
        <span className={lead}>
          <Check size={14} className="text-[#5eead4] flex-none" aria-hidden="true" />
          <span className="text-zinc-200 min-w-0">Added {wordsLabel(l.words)}. Read them over, then press Send.</span>
        </span>
        {d.canUndo && (
          <LineButton phone={phone} onClick={d.undo}>
            <Undo2 size={13} aria-hidden="true" />
            Undo
          </LineButton>
        )}
      </div>
    );
  if (l.kind === 'bad')
    return (
      <div data-dictation-line="bad" role="alert" className={box}>
        <span className={lead}>
          <AlertCircle size={14} className="text-bad flex-none mt-0.5" aria-hidden="true" />
          <span className="text-bad min-w-0 break-words">{l.text}</span>
        </span>
        {l.retry && (
          <LineButton phone={phone} onClick={d.retry}>
            <RotateCcw size={13} aria-hidden="true" />
            Try again
          </LineButton>
        )}
        <CloseButton phone={phone} onClick={d.dismiss} />
      </div>
    );
  return (
    <div data-dictation-line="warn" role="alert" className={box}>
      <span className={lead}>
        <MicOff size={14} className="text-[#fcd34d] flex-none mt-0.5" aria-hidden="true" />
        <span className="text-[#fde68a] min-w-0 break-words">{l.text}</span>
      </span>
      <CloseButton phone={phone} onClick={d.dismiss} />
    </div>
  );
}

/** The polite live region: one sentence per change. Always mounted, so it can be read. */
export function DictationAnnouncer({ d }: { d: Dictation }) {
  return (
    <span role="status" aria-live="polite" className="sr-only" data-dictation-announce>
      {d.announce.text ? <span key={d.announce.n}>{d.announce.text}</span> : null}
    </span>
  );
}

// ---------------------------------------------------------------------------------------------------
// "Dictation isn't set up in this space"

export function DictationSetupPopover({ d, phone, onOpenAccounts }: { d: Dictation; phone: boolean; onOpenAccounts?: () => void }) {
  if (!d.popOpen) return null;
  const accounts = d.secure && onOpenAccounts;
  const btn = phone ? 'min-h-11 px-3.5 text-[13px]' : 'min-h-8 px-3 text-[12.5px]';
  return (
    <div
      ref={d.popRef}
      role="dialog"
      aria-labelledby="kw-dict-pop-title"
      data-dictation-popover
      className={`kw-dict-pop absolute bottom-full mb-2 z-50 ${phone ? 'left-0 right-0' : 'right-[44px] w-[340px] max-w-[calc(100%-16px)]'} bg-surface-overlay border border-hairline-strong rounded-xl shadow-xl p-3.5`}
    >
      <h4 id="kw-dict-pop-title" className="m-0 mb-1.5 flex items-center gap-2 text-[14px] font-semibold text-zinc-50">
        <Mic size={15} aria-hidden="true" />
        Dictation isn’t set up in this space
      </h4>
      <p className="m-0 mb-3 text-[13px] leading-[19px] text-zinc-300">{d.why}</p>
      <div className="flex flex-wrap gap-2">
        {accounts && (
          <button
            type="button"
            onClick={() => {
              d.closePopover(false);
              onOpenAccounts?.();
            }}
            className={`${btn} inline-flex items-center rounded-[10px] bg-accent-gradient text-white font-semibold hover:brightness-110 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-300`}
          >
            Open Apps &amp; accounts
          </button>
        )}
        <button
          type="button"
          onClick={d.hideMic}
          className={`${btn} inline-flex items-center rounded-[10px] border border-hairline-strong bg-white/[0.03] text-zinc-200 font-medium hover:bg-white/[0.08] focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400`}
        >
          Hide the mic
        </button>
      </div>
      <p className="m-0 mt-3 pt-2.5 border-t border-hairline text-[12px] leading-[17px] text-zinc-400">
        OpenAI turns what you say into text with this space’s key. The words land in this box and nothing goes to a bot until you press Send.
      </p>
    </div>
  );
}
