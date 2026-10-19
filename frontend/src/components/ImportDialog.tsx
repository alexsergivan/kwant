import React, { useEffect, useRef, useState } from 'react';
import { AlertTriangle, ArrowLeft, Check, ChevronRight, ClipboardCopy, FileUp, Info, Loader2, Undo2, Upload, X } from 'lucide-react';
import {
  Applied,
  ExportPrompt,
  IMPORT_ACCEPT,
  ImportError,
  ImportRecord,
  Preview,
  canImport,
  fileTooBig,
  importApi,
  importErrorText,
  plural,
  previewRows,
  sourceName,
  textTooLong,
} from '../imports';

const btn =
  'flex items-center justify-center gap-1.5 px-3 py-2 rounded-lg text-xs font-medium text-zinc-300 border border-zinc-700/60 bg-zinc-800/60 hover:bg-zinc-700/70 hover:text-zinc-100 transition-colors disabled:opacity-50';
const primary =
  'flex items-center justify-center gap-1.5 px-3 py-2 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 transition-colors disabled:opacity-50';
const danger =
  'flex items-center justify-center gap-1.5 px-3 py-2 rounded-lg text-xs font-medium text-white bg-rose-600 hover:bg-rose-500 transition-colors disabled:opacity-50';
const card = 'rounded-xl border border-zinc-800 bg-zinc-950/60';

/** One line under each source in "Bring a bot from…". */
const SOURCE_HINT: Record<string, string> = {
  grok: 'A Grok Project or custom Grok bot',
  chatgpt: 'A custom GPT or ChatGPT Project',
  claude: 'A Claude Project',
  other: 'Any other bot or assistant',
};

export const ErrorBox: React.FC<{ error: unknown; children?: React.ReactNode }> = ({ error, children }) => {
  const { title, detail } = importErrorText(error);
  const problems = error instanceof ImportError ? error.problems : [];
  return (
    <div role="alert" className="rounded-lg border border-rose-500/40 bg-rose-500/10 p-3 space-y-1.5">
      <p className="flex items-center gap-2 text-xs font-medium text-rose-200">
        <AlertTriangle size={14} aria-hidden="true" /> {title}
      </p>
      {detail && <p className="text-xs text-rose-100/80">{detail}</p>}
      {problems.length > 0 && (
        <ul className="list-disc pl-5 text-xs text-rose-100/80">
          {problems.map((n, i) => <li key={i}>{n.message}</li>)}
        </ul>
      )}
      {children}
    </div>
  );
};

export const SourceList: React.FC<{
  prompts: ExportPrompt[] | null;
  error: unknown;
  onPick: (p: ExportPrompt) => void;
  onUpload: () => void;
  onRetry: () => void;
}> = ({ prompts, error, onPick, onUpload, onRetry }) => (
  <div className="space-y-3">
    <p className="text-xs text-zinc-400">
      Pick where your bot lives now. Nothing is created until you check a preview and press Import.
    </p>
    <h3 className="text-xs font-semibold text-zinc-300">Bring a bot from…</h3>
    {error ? (
      <ErrorBox error={error}>
        <button type="button" onClick={onRetry} className={btn}>Try again</button>
      </ErrorBox>
    ) : !prompts ? (
      <p className="flex items-center gap-2 text-xs text-zinc-500" role="status">
        <Loader2 size={13} className="animate-spin motion-reduce:animate-none" /> Loading…
      </p>
    ) : (
      <ul className="space-y-1.5">
        {prompts.map((p) => (
          <li key={p.id}>
            <button type="button" onClick={() => onPick(p)} className={`${card} w-full flex items-center gap-3 p-3 text-left hover:border-zinc-600 transition-colors`}>
              <span className="flex-1 min-w-0">
                <span className="block text-sm font-medium text-zinc-100">{p.label}</span>
                <span className="block text-[11px] text-zinc-500">{SOURCE_HINT[p.id] ?? 'Copy a message, paste the reply'}</span>
              </span>
              <ChevronRight size={16} className="text-zinc-500 shrink-0" aria-hidden="true" />
            </button>
          </li>
        ))}
      </ul>
    )}
    <button type="button" onClick={onUpload} className={`${card} w-full flex items-center gap-3 p-3 text-left hover:border-zinc-600 transition-colors`}>
      <FileUp size={18} className="text-zinc-400 shrink-0" aria-hidden="true" />
      <span className="flex-1 min-w-0">
        <span className="block text-sm font-medium text-zinc-100">Upload a file</span>
        <span className="block text-[11px] text-zinc-500">A Claude Code subagent or SKILL.md, a character card, or a Grok Bot, OpenMausBot or .kwant.json export</span>
      </span>
      <ChevronRight size={16} className="text-zinc-500 shrink-0" aria-hidden="true" />
    </button>
  </div>
);

/** The prompt to copy, with steps, and the box for the bot's reply. */
export const PromptStep: React.FC<{
  prompt: ExportPrompt;
  reply: string;
  onReply: (s: string) => void;
  copied: boolean;
  onCopy: () => void;
}> = ({ prompt, reply, onReply, copied, onCopy }) => (
  <div className="space-y-4">
    <ol className="space-y-1.5 text-xs text-zinc-300 list-decimal pl-5">
      <li>{prompt.where_to_send}</li>
      <li>Copy the message below and send it to your bot.</li>
      <li>Copy everything your bot replies and paste it in the box at the bottom.</li>
    </ol>
    <div className={`${card} p-3 space-y-2`}>
      <div className="flex items-center justify-between gap-2">
        <p className="text-xs font-medium text-zinc-300" id="import-prompt-label">Message for your bot</p>
        <button type="button" onClick={onCopy} className={copied ? `${btn} text-emerald-300` : btn}>
          {copied ? <Check size={13} /> : <ClipboardCopy size={13} />} {copied ? 'Copied' : 'Copy'}
        </button>
      </div>
      <pre
        aria-labelledby="import-prompt-label"
        tabIndex={0}
        className="max-h-40 overflow-y-auto whitespace-pre-wrap break-words text-[11px] leading-relaxed text-zinc-400 font-mono bg-zinc-900/60 rounded-lg p-2.5 select-text"
      >
        {prompt.prompt}
      </pre>
    </div>
    <div>
      <label htmlFor="import-reply" className="block text-xs font-medium text-zinc-300 mb-1">
        Your bot’s reply
      </label>
      <textarea
        id="import-reply"
        value={reply}
        onChange={(e) => onReply(e.target.value)}
        rows={6}
        placeholder="Paste the whole reply here"
        className="w-full bg-zinc-950 border border-zinc-800 focus:border-sky-500 rounded-lg px-3 py-2 text-xs text-zinc-100 font-mono focus:outline-none"
      />
      {textTooLong(reply) && <p className="mt-1 text-[11px] text-rose-300">That’s longer than 256 KB. Paste only your bot’s reply.</p>}
    </div>
  </div>
);

export const UploadStep: React.FC<{ file: File | null; onFile: (f: File | null) => void; inputRef: React.RefObject<HTMLInputElement> }> = ({ file, onFile, inputRef }) => {
  const [over, setOver] = useState(false);
  return (
    <div className="space-y-3">
      <div
        onDragOver={(e) => {
          e.preventDefault();
          setOver(true);
        }}
        onDragLeave={() => setOver(false)}
        onDrop={(e) => {
          e.preventDefault();
          setOver(false);
          onFile(e.dataTransfer.files?.[0] ?? null);
        }}
        className={`rounded-xl border-2 border-dashed p-6 text-center space-y-2 ${over ? 'border-sky-500 bg-sky-500/5' : 'border-zinc-700'}`}
      >
        <Upload size={22} className="mx-auto text-zinc-500" aria-hidden="true" />
        <p className="text-xs text-zinc-300">{file ? file.name : 'Drop a file here, or'}</p>
        <label className={`${btn} inline-flex cursor-pointer`}>
          {file ? 'Choose another file' : 'Choose a file'}
          <input ref={inputRef} type="file" accept={IMPORT_ACCEPT} className="sr-only" onChange={(e) => onFile(e.target.files?.[0] ?? null)} />
        </label>
        {file && fileTooBig(file.size) && <p className="text-[11px] text-rose-300">That file is over 20 MB, so it can’t be imported.</p>}
      </div>
      <ul className="text-[11px] text-zinc-500 space-y-0.5 list-disc pl-5">
        <li>Claude Code subagent (.md) or a SKILL.md</li>
        <li>Character card (.json or .png)</li>
        <li>Grok Bot or OpenMausBot export (.json)</li>
        <li>A .kwant.json file exported from here</li>
        <li>A .zip of any of these. Chat history inside is skipped.</li>
      </ul>
    </div>
  );
};

const Notes: React.FC<{ items: string[]; tone: 'warn' | 'drop' | 'problem'; label: string }> = ({ items, tone, label }) =>
  items.length === 0 ? null : (
    <div>
      <p className={`text-[11px] font-medium ${tone === 'problem' ? 'text-rose-300' : tone === 'warn' ? 'text-amber-300' : 'text-zinc-400'}`}>{label}</p>
      <ul className={`list-disc pl-5 text-[11px] ${tone === 'problem' ? 'text-rose-200/90' : tone === 'warn' ? 'text-amber-100/80' : 'text-zinc-500'}`}>
        {items.map((m, i) => <li key={i}>{m}</li>)}
      </ul>
    </div>
  );

/** What the import will create. Each bot can be left out. */
export const PreviewView: React.FC<{ preview: Preview; skip: ReadonlySet<string>; onToggle: (ref: string) => void }> = ({ preview, skip, onToggle }) => {
  const { bots, general } = previewRows(preview);
  const teams = preview.bundle.teams ?? [];
  const shared = preview.bundle.skills ?? [];
  const omitted = preview.bundle.omitted ?? [];
  const kept = bots.filter((b) => !skip.has(b.ref)).length;
  const from = sourceName(preview.bundle.source.kind);
  return (
    <div className="space-y-3">
      <p className="text-xs text-zinc-300">
        {bots.length > 0 ? `${plural(kept, 'bot')} will be created` : 'No bots in this one'}
        {from ? ` from ${from}` : ''}. Untick any you don’t want.
      </p>
      <Notes items={general.problems} tone="problem" label="Must be fixed before importing" />
      <ul className="space-y-2">
        {bots.map((b) => {
          const off = skip.has(b.ref);
          return (
            <li key={b.ref} className={`${card} p-3 space-y-2 ${off ? 'opacity-50' : ''}`}>
              <label className="flex items-start gap-2.5 cursor-pointer">
                <input type="checkbox" checked={!off} onChange={() => onToggle(b.ref)} className="mt-0.5 accent-sky-500" aria-label={`Import ${b.name}`} />
                <span className="flex-1 min-w-0">
                  <span className="block text-sm font-medium text-zinc-100 break-words">{b.name}</span>
                  {b.renamedFrom && (
                    <span className="block text-[11px] text-amber-300">
                      Renamed from {b.renamedFrom}: that name is already taken here.
                    </span>
                  )}
                  {b.title && <span className="block text-[11px] text-zinc-500">{b.title}</span>}
                </span>
              </label>
              {b.excerpt && <p className="text-[11px] text-zinc-400 italic break-words">“{b.excerpt}”</p>}
              <p className="text-[11px] text-zinc-500">
                {plural(b.memories, 'memory', 'memories')} · {plural(b.skills, 'skill')} · {plural(b.routines, 'routine')}
              </p>
              <Notes items={b.problems} tone="problem" label="Must be fixed before importing" />
              <Notes items={b.warnings} tone="warn" label="Worth a look" />
              <Notes items={b.dropped} tone="drop" label="Not brought over" />
            </li>
          );
        })}
      </ul>
      {(teams.length > 0 || shared.length > 0) && (
        <p className="text-[11px] text-zinc-400">
          Also: {[teams.length ? plural(teams.length, 'team') : '', shared.length ? plural(shared.length, 'shared skill') : ''].filter(Boolean).join(' and ')}
          {teams.length ? ` (${teams.map((t) => t.name).join(', ')})` : ''}.
        </p>
      )}
      <Notes items={general.warnings} tone="warn" label="Worth a look" />
      <Notes items={[...general.dropped, ...omitted]} tone="drop" label="Not brought over" />
      <p className="flex gap-2 text-[11px] text-zinc-400 rounded-lg bg-zinc-900/60 p-2.5">
        <Info size={13} className="shrink-0 mt-px text-sky-400" aria-hidden="true" />
        <span>To keep you safe, imported bots start with no tools, their skills wait for your OK, and routines start paused. Keys are removed.</span>
      </p>
    </div>
  );
};

export type UndoState =
  | { kind: 'idle' }
  | { kind: 'busy' }
  | { kind: 'confirm_edited'; names: string[] }
  | { kind: 'in_use'; names: string[] }
  | { kind: 'undone' }
  | { kind: 'error'; error: unknown };

/** After Import: what was made, and Undo. */
export const DoneView: React.FC<{ applied: Applied; undo: UndoState; onUndo: (force: boolean) => void; onKeep: () => void }> = ({ applied, undo, onUndo, onKeep }) => {
  const c = applied.import.created;
  const names = c.kwants.map((k) => k.name);
  if (undo.kind === 'undone') {
    return (
      <div className="space-y-2" role="status">
        <p className="flex items-center gap-2 text-sm text-zinc-100">
          <Undo2 size={16} className="text-sky-400" /> Import undone
        </p>
        <p className="text-xs text-zinc-400">{names.length ? `${names.join(', ')} ${names.length === 1 ? 'was' : 'were'} removed.` : 'Everything it made was removed.'}</p>
      </div>
    );
  }
  const extras = [
    c.teams.length ? plural(c.teams.length, 'team') : '',
    c.skills.length ? `${plural(c.skills.length, 'skill')} to review` : '',
    c.routines.length ? `${plural(c.routines.length, 'paused routine')}` : '',
    c.memories ? plural(c.memories, 'memory', 'memories') : '',
  ].filter(Boolean);
  return (
    <div className="space-y-3">
      <div role="status" className="space-y-1">
        <p className="flex items-center gap-2 text-sm text-zinc-100">
          <Check size={16} className="text-emerald-400" /> Imported {plural(names.length, 'bot')}
        </p>
        {names.length > 0 && <p className="text-xs text-zinc-300 break-words">{names.join(', ')}</p>}
        {extras.length > 0 && <p className="text-[11px] text-zinc-500">Also {extras.join(', ')}.</p>}
      </div>
      <Notes items={applied.warnings.map((n) => n.message)} tone="warn" label="Worth a look" />
      <p className="text-[11px] text-zinc-400">Give them an account and tools in Space settings when you’re ready. Skills and routines are there to review.</p>
      {undo.kind === 'confirm_edited' && (
        <div role="alert" className="rounded-lg border border-amber-500/40 bg-amber-500/10 p-3 space-y-2">
          <p className="text-xs text-amber-100">
            {undo.names.join(', ')} changed since the import. Undo anyway removes {undo.names.length === 1 ? 'it' : 'them'} with your changes.
          </p>
          <div className="flex flex-wrap gap-2">
            <button type="button" onClick={onKeep} className={btn}>Keep them</button>
            <button type="button" onClick={() => onUndo(true)} className={danger}>Undo anyway</button>
          </div>
        </div>
      )}
      {undo.kind === 'in_use' && (
        <div role="alert" className="rounded-lg border border-zinc-700 bg-zinc-900 p-3">
          <p className="text-xs text-zinc-300">
            {undo.names.length ? `${undo.names.join(', ')} ${undo.names.length === 1 ? 'has' : 'have'}` : 'Something it made has'} already been used, so this import can’t be undone in one step. Delete those bots yourself in Kwant bots.
          </p>
        </div>
      )}
      {undo.kind === 'error' && <ErrorBox error={undo.error} />}
    </div>
  );
};

type Step = { kind: 'choose' } | { kind: 'prompt'; prompt: ExportPrompt } | { kind: 'upload' } | { kind: 'preview'; from: 'prompt' | 'upload'; prompt?: ExportPrompt } | { kind: 'done' };

interface Props {
  spaceName: string;
  onClose: () => void;
  /** After Import or Undo, so the sidebar and lists load again. */
  onChanged: () => void;
}

/** "Import bots": pick a source, paste a reply or upload a file, check the preview, import, undo. */
export const ImportDialog: React.FC<Props> = ({ spaceName, onClose, onChanged }) => {
  const [step, setStep] = useState<Step>({ kind: 'choose' });
  const [prompts, setPrompts] = useState<ExportPrompt[] | null>(null);
  const [promptsError, setPromptsError] = useState<unknown>(null);
  const [reply, setReply] = useState('');
  const [copied, setCopied] = useState(false);
  const [file, setFile] = useState<File | null>(null);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [skip, setSkip] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [applied, setApplied] = useState<Applied | null>(null);
  const [undo, setUndo] = useState<UndoState>({ kind: 'idle' });
  const fileRef = useRef<HTMLInputElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;

  const loadPrompts = () => {
    setPromptsError(null);
    importApi.prompts().then(setPrompts, setPromptsError);
  };
  useEffect(loadPrompts, []);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // Space settings underneath leaves Escape to this dialog while it's open.
      if (e.key === 'Escape') closeRef.current();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
  // Each step starts at the top, with focus inside the dialog.
  useEffect(() => {
    bodyRef.current?.scrollTo?.(0, 0);
    bodyRef.current?.querySelector<HTMLElement>('button, textarea, input, [tabindex="0"]')?.focus();
  }, [step.kind]);

  const go = (s: Step) => {
    setError(null);
    setStep(s);
  };

  const copy = async () => {
    if (step.kind !== 'prompt') return;
    try {
      await navigator.clipboard.writeText(step.prompt.prompt);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // No clipboard (plain http): select the text so Ctrl+C works.
      const pre = bodyRef.current?.querySelector('pre');
      if (pre) window.getSelection()?.selectAllChildren(pre);
    }
  };

  const check = async () => {
    setBusy(true);
    setError(null);
    // From the preview ("Check again" after a name was taken), read the same input again.
    const prompt = step.kind === 'prompt' ? step.prompt : step.kind === 'preview' ? step.prompt : undefined;
    try {
      const p = prompt ? await importApi.previewText(reply) : file ? await importApi.previewFile(file, file.name) : null;
      if (!p) return;
      setPreview(p);
      setSkip(new Set());
      go({ kind: 'preview', from: prompt ? 'prompt' : 'upload', prompt });
    } catch (err) {
      setError(err);
    } finally {
      setBusy(false);
    }
  };

  const doImport = async () => {
    if (!preview) return;
    setBusy(true);
    setError(null);
    try {
      const a = await importApi.apply(preview.bundle, [...skip]);
      setApplied(a);
      setUndo({ kind: 'idle' });
      go({ kind: 'done' });
      onChanged();
    } catch (err) {
      setError(err);
    } finally {
      setBusy(false);
    }
  };

  const doUndo = async (force: boolean) => {
    if (!applied) return;
    setUndo({ kind: 'busy' });
    try {
      await importApi.undo(applied.import.id, force);
      setUndo({ kind: 'undone' });
      onChanged();
    } catch (err) {
      if (err instanceof ImportError && err.code === 'edited') setUndo({ kind: 'confirm_edited', names: err.names });
      else if (err instanceof ImportError && err.code === 'in_use') setUndo({ kind: 'in_use', names: err.names });
      else if (err instanceof ImportError && err.code === 'already_undone') setUndo({ kind: 'undone' });
      else setUndo({ kind: 'error', error: err });
    }
  };

  const back = () => {
    if (step.kind === 'prompt' || step.kind === 'upload') go({ kind: 'choose' });
    else if (step.kind === 'preview') go(step.from === 'prompt' && step.prompt ? { kind: 'prompt', prompt: step.prompt } : { kind: 'upload' });
  };

  const title =
    step.kind === 'prompt' ? `Bring a bot from ${step.prompt.label}` : step.kind === 'upload' ? 'Upload a file' : step.kind === 'preview' ? 'Check before importing' : step.kind === 'done' ? 'Import bots' : `Import bots into ${spaceName}`;

  let footer: React.ReactNode = null;
  if (step.kind === 'prompt' || step.kind === 'upload') {
    const ready = step.kind === 'prompt' ? reply.trim() !== '' && !textTooLong(reply) : !!file && !fileTooBig(file.size);
    footer = (
      <button type="button" onClick={check} disabled={!ready || busy} className={primary}>
        {busy && <Loader2 size={13} className="animate-spin motion-reduce:animate-none" />} {busy ? 'Reading…' : 'Preview'}
      </button>
    );
  } else if (step.kind === 'preview' && preview) {
    const n = preview.bundle.bots.filter((b) => !skip.has(b.ref)).length;
    const isNameRace = error instanceof ImportError && error.code === 'name_taken';
    footer = isNameRace ? (
      <button type="button" onClick={check} disabled={busy} className={primary}>
        Check again
      </button>
    ) : (
      <button type="button" onClick={doImport} disabled={!canImport(preview, skip) || busy} className={primary}>
        {busy && <Loader2 size={13} className="animate-spin motion-reduce:animate-none" />} {busy ? 'Importing…' : n > 0 ? `Import ${plural(n, 'bot')}` : 'Import'}
      </button>
    );
  } else if (step.kind === 'done' && applied) {
    footer = (
      <>
        {undo.kind !== 'undone' && undo.kind !== 'confirm_edited' && undo.kind !== 'in_use' && (
          <button type="button" onClick={() => doUndo(false)} disabled={undo.kind === 'busy'} className={btn}>
            {undo.kind === 'busy' ? <Loader2 size={13} className="animate-spin motion-reduce:animate-none" /> : <Undo2 size={13} />} Undo import
          </button>
        )}
        <button type="button" onClick={onClose} className={primary}>Done</button>
      </>
    );
  }

  return (
    <div className="fixed inset-0 z-50 bg-black/75 backdrop-blur-sm flex sm:items-center justify-center sm:p-4" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="import-title"
        className="bg-zinc-900 sm:border border-zinc-800 sm:rounded-2xl w-full sm:max-w-xl h-full sm:h-auto sm:max-h-[90vh] flex flex-col text-zinc-100 shadow-2xl"
      >
        <div className="flex items-center gap-2 border-b border-zinc-800 px-4 py-3">
          {step.kind !== 'choose' && step.kind !== 'done' && (
            <button type="button" onClick={back} aria-label="Back" className="p-1.5 -ml-1 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800">
              <ArrowLeft size={17} />
            </button>
          )}
          <h2 id="import-title" className="flex-1 min-w-0 text-sm font-semibold truncate">{title}</h2>
          <button type="button" onClick={onClose} aria-label="Close" className="p-1.5 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800">
            <X size={18} />
          </button>
        </div>
        <div ref={bodyRef} className="flex-1 overflow-y-auto p-4 space-y-3">
          {step.kind === 'choose' && (
            <SourceList prompts={prompts} error={promptsError} onRetry={loadPrompts} onPick={(p) => go({ kind: 'prompt', prompt: p })} onUpload={() => go({ kind: 'upload' })} />
          )}
          {step.kind === 'prompt' && (
            <PromptStep prompt={step.prompt} reply={reply} onReply={setReply} copied={copied} onCopy={copy} />
          )}
          {step.kind === 'upload' && <UploadStep file={file} onFile={setFile} inputRef={fileRef} />}
          {step.kind === 'preview' && preview && (
            <PreviewView
              preview={preview}
              skip={skip}
              onToggle={(ref) =>
                setSkip((s) => {
                  const next = new Set(s);
                  if (next.has(ref)) next.delete(ref);
                  else next.add(ref);
                  return next;
                })
              }
            />
          )}
          {step.kind === 'done' && applied && <DoneView applied={applied} undo={undo} onUndo={doUndo} onKeep={() => setUndo({ kind: 'idle' })} />}
          {error != null && <ErrorBox error={error} />}
        </div>
        {footer && <div className="flex flex-wrap justify-end gap-2 border-t border-zinc-800 px-4 py-3">{footer}</div>}
      </div>
    </div>
  );
};

/** Recent imports in this space, each with Undo while it still applies. */
export const RecentImports: React.FC<{ records: ImportRecord[]; onUndo: (r: ImportRecord) => void; busyId: string | null }> = ({ records, onUndo, busyId }) => {
  if (records.length === 0) return <p className="text-[11px] text-zinc-500">No imports yet.</p>;
  return (
    <ul className="space-y-1.5">
      {records.slice(0, 10).map((r) => {
        const names = r.created.kwants.map((k) => k.name);
        return (
          <li key={r.id} className={`${card} flex items-center gap-3 p-2.5`}>
            <div className="flex-1 min-w-0">
              <p className="text-xs text-zinc-200 truncate">{names.length ? names.join(', ') : 'Skills only'}</p>
              <p className="text-[11px] text-zinc-500">
                {new Date(r.created_at).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' })}
                {r.status === 'undone' ? ' · undone' : ''}
              </p>
            </div>
            {r.status === 'applied' && (
              <button type="button" onClick={() => onUndo(r)} disabled={busyId === r.id} className={btn} aria-label={`Undo import of ${names.join(', ') || 'skills'}`}>
                {busyId === r.id ? <Loader2 size={12} className="animate-spin motion-reduce:animate-none" /> : <Undo2 size={12} />} Undo
              </button>
            )}
          </li>
        );
      })}
    </ul>
  );
};
