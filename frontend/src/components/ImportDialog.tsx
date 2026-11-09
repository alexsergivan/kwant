import React, { useEffect, useId, useRef, useState } from 'react';
import { AlertTriangle, ArrowLeft, Check, ChevronRight, ClipboardCopy, FileUp, Folder, Info, Link as LinkIcon, Loader2, Undo2, Upload, X } from 'lucide-react';
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
  ToolNote,
  toolNote,
  trapFocusTarget,
} from '../imports';
import { useEscapeLayer } from '../escape';
import { pickProblem, uploadFor } from '../importZip';
import {
  type FolderPlan,
  type GithubInfo,
  type LocalOrigin,
  type LocalSource,
  type PickedEntry,
  folderPickSupported,
  folderPlan,
  folderZip,
  githubFromLine,
  githubLinkOk,
  isAttribution,
  localErrorText,
  localOrigin,
} from '../importSources';
import { inDesktopApp } from '../externalLinks';
import { useMedia } from '../useMedia';
import { PHONE_QUERY } from '../layout';
import { FolderStep, FoundChooseRow, FoundStep, GithubSourceBlock, GithubStep, type LocalState, foundPrimaryLabel } from './ImportSources';

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

const row = `${card} w-full flex items-center gap-3 p-3 min-h-[60px] sm:min-h-0 text-left hover:border-zinc-600 transition-colors`;

export const SourceList: React.FC<{
  prompts: ExportPrompt[] | null;
  error: unknown;
  onPick: (p: ExportPrompt) => void;
  onUpload: () => void;
  onRetry: () => void;
  /** Found on this computer: `off` in the web app, where the group isn't drawn at all. */
  local?: LocalState;
  onFound?: () => void;
  /** Null where this browser can't pick a folder (the row is hidden). */
  onFolder?: (() => void) | null;
  onGithub?: () => void;
}> = ({ prompts, error, onPick, onUpload, onRetry, local = { kind: 'off' }, onFound, onFolder = null, onGithub }) => (
  <div className="space-y-3">
    <p className="text-xs text-zinc-400">
      Pick where your bot lives now. Nothing is created until you check a preview and press Import.
    </p>
    {onFound && <FoundChooseRow state={local} onOpen={onFound} card={card} />}
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
            <button type="button" onClick={() => onPick(p)} className={row}>
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
    <h3 className="text-xs font-semibold text-zinc-300">Or bring files</h3>
    <ul className="space-y-1.5">
      <li>
        <button type="button" onClick={onUpload} className={row}>
          <FileUp size={18} className="text-zinc-400 shrink-0" aria-hidden="true" />
          <span className="flex-1 min-w-0">
            <span className="block text-sm font-medium text-zinc-100">Upload a file</span>
            <span className="block text-[11px] text-zinc-500">One or several files: Claude Code subagents or SKILL.md, character cards, or Grok Bot, OpenMausBot or .kwant.json exports</span>
          </span>
          <ChevronRight size={16} className="text-zinc-500 shrink-0" aria-hidden="true" />
        </button>
      </li>
      {onFolder && (
        <li>
          <button type="button" onClick={onFolder} className={row} data-folder-row>
            <Folder size={18} className="text-zinc-400 shrink-0" aria-hidden="true" />
            <span className="flex-1 min-w-0">
              <span className="block text-sm font-medium text-zinc-100">Folder</span>
              <span className="block text-[11px] text-zinc-500">A skills folder, or an OpenClaw, Hermes or Codex workspace</span>
            </span>
            <ChevronRight size={16} className="text-zinc-500 shrink-0" aria-hidden="true" />
          </button>
        </li>
      )}
      {onGithub && (
        <li>
          <button type="button" onClick={onGithub} className={row} data-github-row>
            <LinkIcon size={18} className="text-zinc-400 shrink-0" aria-hidden="true" />
            <span className="flex-1 min-w-0">
              <span className="block text-sm font-medium text-zinc-100">GitHub link</span>
              <span className="block text-[11px] text-zinc-500">A public repo of bots or skills. You see its licence first</span>
            </span>
            <ChevronRight size={16} className="text-zinc-500 shrink-0" aria-hidden="true" />
          </button>
        </li>
      )}
    </ul>
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

/** A short label for what's picked: the file's name, or "3 files: a.json, b.json and c.md". */
export function pickedLabel(files: ReadonlyArray<{ name: string }>): string {
  if (files.length === 0) return '';
  if (files.length === 1) return files[0].name;
  const names = files.map((f) => f.name);
  const shown = names.slice(0, 3);
  const rest = names.length - shown.length;
  const list = rest > 0 ? `${shown.join(', ')} and ${rest} more` : `${shown.slice(0, -1).join(', ')} and ${shown[shown.length - 1]}`;
  return `${files.length} files: ${list}`;
}

export const UploadStep: React.FC<{ files: File[]; onFiles: (f: File[]) => void; inputRef: React.RefObject<HTMLInputElement> }> = ({ files, onFiles, inputRef }) => {
  const [over, setOver] = useState(false);
  const one = files.length === 1 ? files[0] : null;
  const problem = pickProblem(files);
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
          onFiles([...(e.dataTransfer.files ?? [])]);
        }}
        className={`rounded-xl border-2 border-dashed p-6 text-center space-y-2 ${over ? 'border-sky-500 bg-sky-500/5' : 'border-zinc-700'}`}
      >
        <Upload size={22} className="mx-auto text-zinc-500" aria-hidden="true" />
        <p className="text-xs text-zinc-300 break-words" data-picked>{files.length ? pickedLabel(files) : 'Drop one or more files here, or'}</p>
        <label className={`${btn} inline-flex cursor-pointer`}>
          {files.length ? 'Choose other files' : 'Choose files'}
          <input ref={inputRef} type="file" multiple accept={IMPORT_ACCEPT} className="sr-only" onChange={(e) => onFiles([...(e.target.files ?? [])])} />
        </label>
        {one && fileTooBig(one.size) && <p className="text-[11px] text-rose-300">That file is over 20 MB, so it can’t be imported.</p>}
        {problem && (
          <p className="text-[11px] text-rose-300" role="alert">
            {problem}
          </p>
        )}
      </div>
      <ul className="text-[11px] text-zinc-500 space-y-0.5 list-disc pl-5">
        <li>Claude Code subagent (.md) or a SKILL.md</li>
        <li>Character card (.json or .png)</li>
        <li>Grok Bot or OpenMausBot export (.json)</li>
        <li>A .kwant.json file exported from here</li>
        <li>Several of these at once: pick or drop them together, no need to zip them</li>
        <li>A .zip of any of these. Chat history inside is skipped.</li>
      </ul>
    </div>
  );
};

/** How many app names a tool notice shows before "Show all". */
const APPS_SHOWN = 5;

/** The "It used tools (…)" notice as a short list of app names, with the rest behind "Show all". */
const ToolNoteItem: React.FC<{ note: ToolNote }> = ({ note }) => {
  const [open, setOpen] = useState(false);
  const listId = useId();
  const n = note.apps.length;
  const shown = open ? note.apps : note.apps.slice(0, APPS_SHOWN);
  const hidden = n - shown.length;
  return (
    <li>
      <span>
        {open
          ? `It used tools from ${n === 1 ? note.apps[0].name : `${n} apps, listed below`}. `
          : `It used tools from ${n === 1 ? '' : `${n} apps: `}${shown.map((a) => a.name).join(', ')}${hidden > 0 ? ` and ${hidden} more` : ''}. `}
        {note.tail}
      </span>{' '}
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        aria-controls={listId}
        className="underline underline-offset-2 text-amber-200 hover:text-amber-100 rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400"
      >
        {open ? 'Show less' : hidden > 0 ? 'Show all' : 'Show tool names'}
      </button>
      {open && (
        <ul id={listId} className="mt-1 space-y-0.5 pl-1 list-none">
          {note.apps.map((a) => (
            <li key={a.name} className="break-words">
              <span className="text-amber-100">{a.name}</span>
              {a.tools.length > 1 || a.tools[0] !== a.name ? <span className="text-zinc-400">: {a.tools.join(', ')}</span> : null}
            </li>
          ))}
        </ul>
      )}
    </li>
  );
};

const Notes: React.FC<{ items: string[]; tone: 'warn' | 'drop' | 'problem'; label: string }> = ({ items, tone, label }) =>
  items.length === 0 ? null : (
    <div>
      <p className={`text-[11px] font-medium ${tone === 'problem' ? 'text-rose-300' : tone === 'warn' ? 'text-amber-300' : 'text-zinc-400'}`}>{label}</p>
      <ul className={`list-disc pl-5 text-[11px] break-words ${tone === 'problem' ? 'text-rose-200/90' : tone === 'warn' ? 'text-amber-100/80' : 'text-zinc-500'}`}>
        {items.map((m, i) => {
          const tools = tone === 'warn' ? toolNote(m) : null;
          return tools ? <ToolNoteItem key={i} note={tools} /> : <li key={i}>{m}</li>;
        })}
      </ul>
    </div>
  );

/** What the import will create. Each bot can be left out. */
export const PreviewView: React.FC<{
  preview: Preview;
  skip: ReadonlySet<string>;
  onToggle: (ref: string) => void;
  /** Several picked files went up as one zip: say "from N files", not "from a zip". */
  pickedFiles?: number;
  /** A picked folder: "from <name>". */
  folderName?: string;
  /** Found on this computer: where the bots and shared skills came from, when that's certain. */
  local?: { ticked: LocalSource[]; origin: LocalOrigin };
  /** A GitHub link: the source block above everything. */
  github?: GithubInfo;
  spaceName?: string;
}> = ({ preview, skip, onToggle, pickedFiles = 0, folderName, local, github, spaceName = 'this space' }) => {
  const { bots, general } = previewRows(preview);
  const teams = preview.bundle.teams ?? [];
  const shared = preview.bundle.skills ?? [];
  const packed = pickedFiles > 1;
  const omitted = (preview.bundle.omitted ?? [])
    .filter((t) => !github || !isAttribution(t))
    .map((t) => (packed || folderName ? t.replace(/^Files in the zip that weren't read:/, "Files that weren't read:") : t));
  const kept = bots.filter((b) => !skip.has(b.ref)).length;
  const from = folderName
    ? folderName
    : packed
      ? `${pickedFiles} files`
      : local
        ? `${plural(local.ticked.length, 'source')} on this computer`
        : github
          ? null
          : sourceName(preview.bundle.source.kind);
  const botFrom = local?.origin.botSource;
  const skillFrom = local?.origin.skillSource;
  const skillsOnly = bots.length === 0 && shared.length > 0;
  const SHOWN = 8;
  return (
    <div className="space-y-3">
      {github && <GithubSourceBlock github={github} />}
      {skillsOnly ? (
        <p className="text-xs text-zinc-300">
          No bots in this one. <strong className="font-semibold text-zinc-100">{plural(shared.length, 'skill')}</strong> will be added to {spaceName}.
        </p>
      ) : (
        <p className="text-xs text-zinc-300">
          {bots.length > 0 ? `${plural(kept, 'bot')} will be created` : 'No bots in this one'}
          {from ? ` from ${from}` : ''}. Untick any you don’t want.
        </p>
      )}
      {skillsOnly && (
        <div className={`${card} p-3`}>
          <ul className="flex flex-wrap items-center gap-1.5" aria-label="Skills">
            {shared.slice(0, SHOWN).map((k, i) => (
              <li key={`${k.name}-${i}`} className="font-mono text-[11px] text-zinc-300 border border-zinc-700 rounded-full px-2 py-0.5">{k.name}</li>
            ))}
            {shared.length > SHOWN && <li className="text-[11px] text-zinc-500">and {shared.length - SHOWN} more</li>}
          </ul>
        </div>
      )}
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
                  {botFrom && (
                    <span className="block text-[11px] text-zinc-500 break-all">
                      From {botFrom.label} · <span className="font-mono">{botFrom.path}</span>
                    </span>
                  )}
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
      {!skillsOnly && skillFrom && teams.length === 0 && shared.length > 0 ? (
        <p className="text-[11px] text-zinc-400">
          Also: {plural(shared.length, 'shared skill')} from <span className="font-mono">{skillFrom.path}</span>.
        </p>
      ) : !skillsOnly && (teams.length > 0 || shared.length > 0) && (
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
export const DoneView: React.FC<{ applied: Applied; undo: UndoState; onUndo: (force: boolean) => void; onKeep: () => void; github?: GithubInfo }> = ({
  applied,
  undo,
  onUndo,
  onKeep,
  github,
}) => {
  const c = applied.import.created;
  const names = c.kwants.map((k) => k.name);
  const skillsOnly = names.length === 0 && c.skills.length > 0;
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
    c.skills.length && !skillsOnly ? `${plural(c.skills.length, 'skill')} to review` : '',
    c.routines.length ? `${plural(c.routines.length, 'paused routine')}` : '',
    c.memories ? plural(c.memories, 'memory', 'memories') : '',
  ].filter(Boolean);
  return (
    <div className="space-y-3">
      <div role="status" className="space-y-1">
        <p className="flex items-center gap-2 text-sm text-zinc-100">
          <Check size={16} className="text-emerald-400" /> Imported {skillsOnly ? plural(c.skills.length, 'skill') : plural(names.length, 'bot')}
        </p>
        {github && (
          <p className="text-[11px] text-zinc-400 break-words" data-github-done>
            {githubFromLine(github)} They wait for your OK in Settings › This space › Skills.
          </p>
        )}
        {names.length > 0 && <p className="text-xs text-zinc-300 break-words">{names.join(', ')}</p>}
        {extras.length > 0 && <p className="text-[11px] text-zinc-500">Also {extras.join(', ')}.</p>}
      </div>
      <Notes items={applied.warnings.map((n) => n.message)} tone="warn" label="Worth a look" />
      {!skillsOnly && (
        <>
          <p className="text-[11px] text-zinc-400">Give them an account and tools in Settings › This space when you’re ready. Skills and routines are there to review.</p>
          <p className="text-[11px] text-zinc-400">Tools you add to this space later will be on for these bots too.</p>
        </>
      )}
      {skillsOnly && !github && <p className="text-[11px] text-zinc-400">They wait for your OK in Settings › This space › Skills.</p>}
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

type From = 'prompt' | 'upload' | 'found' | 'folder' | 'github';
type Step =
  | { kind: 'choose' }
  | { kind: 'prompt'; prompt: ExportPrompt }
  | { kind: 'upload' }
  | { kind: 'found' }
  | { kind: 'folder' }
  | { kind: 'github' }
  | { kind: 'preview'; from: From; prompt?: ExportPrompt }
  | { kind: 'done' };

interface Props {
  spaceName: string;
  onClose: () => void;
  /** After Import or Undo, so the sidebar and lists load again. */
  onChanged: () => void;
  /** Found on this computer, already asked for by the opener (Get started step 3), so it isn't asked twice. */
  localSources?: LocalSources;
}

export type LocalSources = [LocalState, () => Promise<LocalSource[] | null>];

const FOCUSABLE = 'button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), a[href], [tabindex]:not([tabindex="-1"])';

/** Found on this computer, as `GET /api/import/local` says (`off` in the web app or with `available: false`). */
export function useLocalSources(enabled: boolean = inDesktopApp()): LocalSources {
  const [state, setState] = useState<LocalState>(enabled ? { kind: 'loading' } : { kind: 'off' });
  const load = async () => {
    if (!enabled) return null;
    setState({ kind: 'loading' });
    try {
      const l = await importApi.local();
      if (!l.available) {
        setState({ kind: 'off' });
        return null;
      }
      setState({ kind: 'ready', sources: l.sources });
      return l.sources;
    } catch (err) {
      setState({ kind: 'error', error: err });
      return null;
    }
  };
  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return [state, load];
}

/** "Import bots": pick a source, paste a reply, upload files, a folder or a GitHub link, check the preview, import, undo. */
export const ImportDialog: React.FC<Props> = ({ spaceName, onClose, onChanged, localSources }) => {
  const [step, setStep] = useState<Step>({ kind: 'choose' });
  const [prompts, setPrompts] = useState<ExportPrompt[] | null>(null);
  const [promptsError, setPromptsError] = useState<unknown>(null);
  const [reply, setReply] = useState('');
  const [copied, setCopied] = useState(false);
  const [files, setFiles] = useState<File[]>([]);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [skip, setSkip] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [applied, setApplied] = useState<Applied | null>(null);
  const [undo, setUndo] = useState<UndoState>({ kind: 'idle' });
  // Found on this computer.
  const own = useLocalSources(!localSources && inDesktopApp());
  const [local, reloadLocal] = localSources ?? own;
  const [ticked, setTicked] = useState<Set<string>>(new Set());
  const [localError, setLocalError] = useState<{ title: string; detail: string } | null>(null);
  const [previewLocal, setPreviewLocal] = useState<{ ticked: LocalSource[]; origin: LocalOrigin } | null>(null);
  // Folder.
  const [plan, setPlan] = useState<FolderPlan | null>(null);
  const [folderUnsupported, setFolderUnsupported] = useState(false);
  // GitHub link.
  const [ghUrl, setGhUrl] = useState('');
  const [ghChecked, setGhChecked] = useState(false);
  const [github, setGithub] = useState<GithubInfo | null>(null);
  const phone = useMedia(PHONE_QUERY);
  const fileRef = useRef<HTMLInputElement>(null);
  const folderRef = useRef<HTMLInputElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  const canFolder = folderPickSupported();

  const loadPrompts = () => {
    setPromptsError(null);
    importApi.prompts().then(setPrompts, setPromptsError);
  };
  useEffect(loadPrompts, []);
  // Escape closes this dialog and not Settings under it, and keeps a pasted reply that isn't imported yet.
  useEscapeLayer(dialogRef, true, () => closeRef.current(), { unsaved: reply.trim() !== '' && !applied });
  // Focus goes back to whatever opened the dialog when it closes.
  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    return () => opener?.focus?.();
  }, []);
  // Tab and Shift+Tab stay inside the dialog.
  const onTrapKey = (e: React.KeyboardEvent) => {
    if (e.key !== 'Tab' || !dialogRef.current) return;
    const items = [...dialogRef.current.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((el) => el.offsetParent !== null || el === document.activeElement);
    const next = trapFocusTarget(items.length, items.indexOf(document.activeElement as HTMLElement), e.shiftKey);
    if (next === null) return;
    e.preventDefault();
    items[next].focus();
  };
  // Each step starts at the top, with focus inside the dialog.
  useEffect(() => {
    bodyRef.current?.scrollTo?.(0, 0);
    bodyRef.current?.querySelector<HTMLElement>('button, textarea, input:not([type="file"]), [tabindex="0"]')?.focus();
  }, [step.kind]);

  const go = (s: Step) => {
    setError(null);
    setLocalError(null);
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

  const foundList = local.kind === 'ready' ? local.sources : [];
  const tickedSources = foundList.filter((s) => ticked.has(s.id) && !s.too_large);

  const showPreview = (p: Preview, from: From, prompt?: ExportPrompt) => {
    setPreview(p);
    setSkip(new Set());
    go({ kind: 'preview', from, prompt });
  };

  const check = async () => {
    setBusy(true);
    setError(null);
    // From the preview ("Check again" after a name was taken), read the same input again.
    const from: From = step.kind === 'preview' ? step.from : step.kind === 'prompt' ? 'prompt' : step.kind === 'found' ? 'found' : step.kind === 'folder' ? 'folder' : step.kind === 'github' ? 'github' : 'upload';
    const prompt = step.kind === 'prompt' ? step.prompt : step.kind === 'preview' ? step.prompt : undefined;
    try {
      if (from === 'found') {
        setLocalError(null);
        const picked = tickedSources;
        if (picked.length === 0) return;
        try {
          const p = await importApi.localPreview(picked.map((s) => s.id));
          setPreviewLocal({ ticked: picked, origin: localOrigin(picked) });
          showPreview(p, 'found');
        } catch (err) {
          const text = localErrorText(err, picked);
          if (step.kind === 'preview') go({ kind: 'found' });
          setLocalError(text);
          // A source went away: look again and drop the ticks that vanished.
          if (err instanceof ImportError && err.code === 'not_found') {
            const now = await reloadLocal();
            if (now) setTicked((t) => new Set([...t].filter((id) => now.some((s) => s.id === id && !s.too_large))));
          }
        }
        return;
      }
      if (from === 'github') {
        setGhChecked(true);
        if (!githubLinkOk(ghUrl)) return;
        const p = await importApi.githubPreview(ghUrl);
        setGithub(p.github);
        showPreview(p, 'github');
        return;
      }
      if (from === 'folder') {
        if (!plan || plan.problem || plan.kept.length === 0) return;
        setFolderUnsupported(false);
        const up = await folderZip(plan);
        try {
          showPreview(await importApi.previewFile(up.body, up.filename), 'folder');
        } catch (err) {
          if (err instanceof ImportError && err.code === 'unsupported') {
            if (step.kind === 'preview') go({ kind: 'folder' });
            setFolderUnsupported(true);
            return;
          }
          throw err;
        }
        return;
      }
      let p = null;
      if (prompt) p = await importApi.previewText(reply);
      else if (files.length > 0 && !pickProblem(files)) {
        // Several files go up as one zip, which the server reads into one preview.
        const up = await uploadFor(files);
        p = await importApi.previewFile(up.body, up.filename);
      }
      if (!p) return;
      showPreview(p, prompt ? 'prompt' : 'upload', prompt);
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
    if (step.kind === 'prompt' || step.kind === 'upload' || step.kind === 'found' || step.kind === 'folder' || step.kind === 'github') go({ kind: 'choose' });
    else if (step.kind === 'preview') go(step.from === 'prompt' && step.prompt ? { kind: 'prompt', prompt: step.prompt } : { kind: step.from === 'prompt' ? 'upload' : step.from });
  };

  const pickFolder = () => folderRef.current?.click();
  const openFolder = () => {
    go({ kind: 'folder' });
  };
  const onFolderPicked = (entries: PickedEntry[]) => {
    setFolderUnsupported(false);
    setError(null);
    setPlan(entries.length ? folderPlan(entries) : null);
  };

  const TITLES: Partial<Record<Step['kind'], string>> = {
    upload: 'Upload a file',
    found: 'Found on this computer',
    folder: 'Bring a folder',
    github: 'Bring from a GitHub link',
    preview: 'Check before importing',
    done: 'Import bots',
  };
  const title = step.kind === 'prompt' ? `Bring a bot from ${step.prompt.label}` : TITLES[step.kind] ?? `Import bots into ${spaceName}`;

  const fb = phone ? 'flex-1 min-h-11 text-sm justify-center' : '';
  const cancel = (
    <button type="button" onClick={onClose} className={`${btn} ${fb}`}>
      Cancel
    </button>
  );
  const spin = <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />;
  let footer: React.ReactNode = null;
  if (step.kind === 'prompt' || step.kind === 'upload') {
    const ready =
      step.kind === 'prompt'
        ? reply.trim() !== '' && !textTooLong(reply)
        : files.length > 1
          ? !pickProblem(files)
          : files.length === 1 && !fileTooBig(files[0].size);
    footer = (
      <>
        {cancel}
        <button type="button" onClick={check} disabled={!ready || busy} className={`${primary} ${fb}`}>
          {busy && spin} {busy ? 'Reading…' : 'Preview'}
        </button>
      </>
    );
  } else if (step.kind === 'found') {
    const nothing = local.kind === 'ready' && local.sources.length === 0;
    footer = nothing ? (
      <button type="button" onClick={back} className={`${btn} ${fb}`}>
        Back
      </button>
    ) : (
      <>
        {cancel}
        <button type="button" onClick={check} disabled={tickedSources.length === 0 || busy || local.kind !== 'ready'} className={`${primary} ${fb}`} data-found-primary>
          {busy && spin} {foundPrimaryLabel(tickedSources.length, busy)}
        </button>
      </>
    );
  } else if (step.kind === 'folder') {
    footer = (
      <>
        {plan ? (
          <button type="button" onClick={pickFolder} className={`${btn} ${fb}`}>
            Change folder
          </button>
        ) : (
          cancel
        )}
        <button type="button" onClick={check} disabled={!plan || !!plan.problem || plan.kept.length === 0 || busy} className={`${primary} ${fb}`}>
          {busy && spin} {busy ? 'Reading…' : 'Preview'}
        </button>
      </>
    );
  } else if (step.kind === 'github') {
    const ok = !!githubLinkOk(ghUrl);
    footer = (
      <>
        {cancel}
        <button type="button" onClick={check} disabled={busy || ghUrl.trim() === '' || (ghChecked && !ok)} className={`${primary} ${fb}`}>
          {busy && spin} {busy ? 'Fetching…' : error != null ? 'Try again' : 'Preview'}
        </button>
      </>
    );
  } else if (step.kind === 'preview' && preview) {
    const n = preview.bundle.bots.filter((b) => !skip.has(b.ref)).length;
    const k = preview.bundle.skills?.length ?? 0;
    const isNameRace = error instanceof ImportError && error.code === 'name_taken';
    footer = (
      <>
        {cancel}
        {isNameRace ? (
          <button type="button" onClick={check} disabled={busy} className={`${primary} ${fb}`}>
            Check again
          </button>
        ) : (
          <button type="button" onClick={doImport} disabled={!canImport(preview, skip) || busy} className={`${primary} ${fb}`}>
            {busy && spin} {busy ? 'Importing…' : n > 0 ? `Import ${plural(n, 'bot')}` : preview.bundle.bots.length === 0 && k > 0 ? `Import ${plural(k, 'skill')}` : 'Import'}
          </button>
        )}
      </>
    );
  } else if (step.kind === 'done' && applied) {
    footer = (
      <>
        {undo.kind !== 'undone' && undo.kind !== 'confirm_edited' && undo.kind !== 'in_use' && (
          <button type="button" onClick={() => doUndo(false)} disabled={undo.kind === 'busy'} className={`${btn} ${fb}`}>
            {undo.kind === 'busy' ? spin : <Undo2 size={13} />} Undo import
          </button>
        )}
        <button type="button" onClick={onClose} className={`${primary} ${fb}`}>Done</button>
      </>
    );
  }

  const iconBtn = phone ? 'w-11 h-11 flex items-center justify-center' : 'p-1.5';
  return (
    <div className="kw-dialog fixed inset-0 z-50 bg-black/75 backdrop-blur-sm flex sm:items-center justify-center sm:p-4" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div
        ref={dialogRef}
        onKeyDown={onTrapKey}
        role="dialog"
        aria-modal="true"
        aria-labelledby="import-title"
        className="kw-dialog-panel bg-zinc-900 sm:border border-zinc-800 sm:rounded-2xl w-full sm:max-w-xl h-full sm:h-auto sm:max-h-[90vh] flex flex-col text-zinc-100 shadow-2xl"
      >
        <div className={`flex items-center gap-2 border-b border-zinc-800 px-4 ${phone ? 'h-14 px-2' : 'py-3'}`}>
          {step.kind !== 'choose' && step.kind !== 'done' && (
            <button type="button" onClick={back} aria-label="Back" className={`${iconBtn} -ml-1 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800`}>
              <ArrowLeft size={17} />
            </button>
          )}
          <h2 id="import-title" className="flex-1 min-w-0 text-sm font-semibold truncate">{title}</h2>
          <button type="button" onClick={onClose} aria-label="Close" className={`${iconBtn} rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800`}>
            <X size={18} />
          </button>
        </div>
        <div ref={bodyRef} className="flex-1 overflow-y-auto p-4 space-y-3">
          {step.kind === 'choose' && (
            <SourceList
              prompts={prompts}
              error={promptsError}
              onRetry={loadPrompts}
              onPick={(p) => go({ kind: 'prompt', prompt: p })}
              onUpload={() => go({ kind: 'upload' })}
              local={local}
              onFound={local.kind === 'off' ? undefined : () => go({ kind: 'found' })}
              onFolder={canFolder ? openFolder : null}
              onGithub={() => go({ kind: 'github' })}
            />
          )}
          {step.kind === 'prompt' && (
            <PromptStep prompt={step.prompt} reply={reply} onReply={setReply} copied={copied} onCopy={copy} />
          )}
          {step.kind === 'upload' && <UploadStep files={files} onFiles={setFiles} inputRef={fileRef} />}
          {step.kind === 'found' && (
            <FoundStep
              state={local}
              ticked={ticked}
              onToggle={(id) =>
                setTicked((t) => {
                  const next = new Set(t);
                  if (next.has(id)) next.delete(id);
                  else next.add(id);
                  return next;
                })
              }
              reading={busy}
              previewError={localError}
              onRetry={() => void reloadLocal()}
              onFolder={canFolder ? openFolder : null}
              onUpload={() => go({ kind: 'upload' })}
            />
          )}
          {step.kind === 'folder' && <FolderStep plan={plan} onPicked={onFolderPicked} inputRef={folderRef} unsupported={folderUnsupported} />}
          {step.kind === 'github' && (
            <GithubStep
              url={ghUrl}
              onUrl={(u) => {
                setGhUrl(u);
                setError(null);
              }}
              checked={ghChecked}
              onCheck={() => setGhChecked(true)}
              fetching={busy}
              error={error}
              phone={phone}
            />
          )}
          {step.kind === 'preview' && preview && (
            <PreviewView
              preview={preview}
              pickedFiles={step.from === 'upload' ? files.length : 0}
              folderName={step.from === 'folder' ? plan?.name : undefined}
              local={step.from === 'found' && previewLocal ? previewLocal : undefined}
              github={step.from === 'github' && github ? github : undefined}
              spaceName={spaceName}
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
          {step.kind === 'done' && applied && (
            <DoneView applied={applied} undo={undo} onUndo={doUndo} onKeep={() => setUndo({ kind: 'idle' })} github={github && applied.import.created.kwants.length === 0 ? github : undefined} />
          )}
          {error != null && step.kind !== 'github' && <ErrorBox error={error} />}
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
