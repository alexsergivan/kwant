// The three 0.8.0 import sources (Mira's a4-import.md): Found on this computer (F1–F4, F6), a folder
// (D1–D4) and a GitHub link (G1–G4), plus the GitHub source block on the preview (G5/G6).
import React, { useId, useState } from 'react';
import { ChevronRight, ExternalLink, FileUp, Folder, HardDrive, Info, Link as LinkIcon, Loader2, Scale } from 'lucide-react';
import { plural } from '../imports';
import {
  type FolderPlan,
  type GithubInfo,
  type LocalSource,
  type PickedEntry,
  NOT_GITHUB,
  USUAL_PLACES,
  countsLine,
  foundHint,
  formatBytes,
  githubErrorText,
  githubLicenceUrl,
  githubLinkOk,
  githubRefLine,
  githubRepoUrl,
  licenceLine,
} from '../importSources';

const btn =
  'flex items-center justify-center gap-1.5 px-3 py-2 rounded-lg text-xs font-medium text-zinc-300 border border-zinc-700/60 bg-zinc-800/60 hover:bg-zinc-700/70 hover:text-zinc-100 transition-colors disabled:opacity-50';
const card = 'rounded-xl border border-zinc-800 bg-zinc-950/60';
const pill = 'font-mono text-[11px] bg-zinc-900/70 rounded px-1.5 py-px text-zinc-400 break-all';
const ring = 'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400 focus-visible:ring-offset-2 focus-visible:ring-offset-zinc-900';
const spin = 'animate-spin motion-reduce:animate-none';

/** Mira's ErrorBox look with copy written from the error code. */
export const CopyErrorBox: React.FC<{ title: string; detail?: string; children?: React.ReactNode }> = ({ title, detail, children }) => (
  <div role="alert" className="rounded-lg border border-rose-500/40 bg-rose-500/10 p-3 space-y-1.5">
    <p className="text-xs font-medium text-rose-200">{title}</p>
    {detail && <p className="text-xs text-rose-100/80">{detail}</p>}
    {children}
  </div>
);

const SafeLine: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <p className="flex gap-2 text-[11px] text-zinc-400 rounded-lg bg-zinc-900/60 p-2.5">
    <Info size={13} className="shrink-0 mt-px text-sky-400" aria-hidden="true" />
    <span>{children}</span>
  </p>
);

// ------------------------------------------------------------------ Found on this computer

export type LocalState =
  | { kind: 'off' }
  | { kind: 'loading' }
  | { kind: 'ready'; sources: LocalSource[] }
  | { kind: 'error'; error: unknown };

const LOOKING = 'Looking in 5 usual places…';

/** One source row: a label around a real checkbox, the counts, and the names on request. */
const FoundRow: React.FC<{ s: LocalSource; on: boolean; disabled: boolean; onToggle: () => void }> = ({ s, on, disabled, onToggle }) => {
  const [open, setOpen] = useState(false);
  const namesId = useId();
  const off = s.too_large;
  return (
    <li className={`${card} ${on ? 'border-sky-500/45' : ''}`} data-source={s.id}>
      <label className={`flex items-start gap-2.5 p-3 min-h-[60px] ${off || disabled ? 'cursor-default' : 'cursor-pointer'}`}>
        <input
          type="checkbox"
          checked={on}
          disabled={off || disabled}
          onChange={onToggle}
          className={`mt-1 accent-sky-500 rounded ${ring} ${off ? 'opacity-[.55]' : ''}`}
        />
        <span className="flex-1 min-w-0">
          <span className={`flex flex-wrap items-baseline gap-x-2 gap-y-0.5 ${off ? 'opacity-[.55]' : ''}`}>
            <span className="text-sm font-medium text-zinc-100">{s.label}</span>
            <span className={pill}>{s.path}</span>
          </span>
          {off ? (
            <span className="block mt-0.5 text-[11px] text-rose-300">
              Too big to bring from here. Use <strong className="font-medium text-zinc-100">Folder</strong> to pick just the parts you want.
            </span>
          ) : (
            <span className="block mt-0.5 text-[11px] text-zinc-500">{countsLine(s) || 'Nothing counted yet'}</span>
          )}
        </span>
      </label>
      {!off && s.names.length > 0 && (
        <div className="px-3 pb-2.5 -mt-1 pl-[34px]">
          <button type="button" onClick={() => setOpen((v) => !v)} aria-expanded={open} aria-controls={namesId} className={`text-[11px] text-sky-300 hover:text-sky-200 rounded ${ring}`}>
            {open ? 'Hide names' : 'Show names'}
          </button>
          {open && (
            <ul id={namesId} className="mt-1 flex flex-wrap gap-x-2 gap-y-0.5 font-mono text-[11px] text-zinc-400">
              {s.names.slice(0, 50).map((n) => (
                <li key={n} className="break-all">{n}</li>
              ))}
            </ul>
          )}
        </div>
      )}
    </li>
  );
};

export const FoundStep: React.FC<{
  state: LocalState;
  ticked: ReadonlySet<string>;
  onToggle: (id: string) => void;
  /** F6: reading the ticked sources. */
  reading: boolean;
  /** An error from the preview request (F2 with an ErrorBox). */
  previewError: { title: string; detail: string } | null;
  onRetry: () => void;
  onFolder: (() => void) | null;
  onUpload: () => void;
}> = ({ state, ticked, onToggle, reading, previewError, onRetry, onFolder, onUpload }) => {
  if (state.kind === 'loading' || state.kind === 'off') {
    return (
      <div className="space-y-3" data-found-state="loading">
        <p role="status" className="flex items-center gap-2 text-xs text-zinc-400">
          <Loader2 size={13} className={spin} aria-hidden="true" /> {LOOKING}
        </p>
        <ul className="space-y-2" aria-hidden="true">
          {[0, 1, 2].map((i) => (
            <li key={i} className={`${card} p-3 flex items-start gap-2.5`}>
              <span className="mt-0.5 w-3.5 h-3.5 rounded border border-zinc-700" />
              <span className="flex-1 space-y-1.5">
                <span className="block h-3 w-40 rounded bg-zinc-800" />
                <span className="block h-2.5 w-24 rounded bg-zinc-800/70" />
              </span>
            </li>
          ))}
        </ul>
        <SafeLine>Only folder names and counts. This takes a second.</SafeLine>
      </div>
    );
  }
  if (state.kind === 'error') {
    return (
      <div className="space-y-3" data-found-state="error">
        <CopyErrorBox title="Couldn’t look on this computer" detail={errorReason(state.error)}>
          <button type="button" onClick={onRetry} className={btn}>Try again</button>
        </CopyErrorBox>
        <p className="text-xs text-zinc-400">You can still pick a folder or upload a file.</p>
        <div className="flex flex-wrap gap-2">
          {onFolder && (
            <button type="button" onClick={onFolder} className={btn}>
              <Folder size={13} aria-hidden="true" /> Pick a folder
            </button>
          )}
          <button type="button" onClick={onUpload} className={btn}>
            <FileUp size={13} aria-hidden="true" /> Upload a file
          </button>
        </div>
      </div>
    );
  }
  if (state.sources.length === 0) {
    return (
      <div className="space-y-3 text-center py-2" data-found-state="nothing">
        <HardDrive size={24} className="mx-auto text-zinc-500" aria-hidden="true" />
        <p className="text-sm font-medium text-zinc-100">No bots or skills in the usual places</p>
        <p className="text-xs text-zinc-400">kwant looked in:</p>
        <ul className="flex flex-wrap justify-center gap-1.5">
          {USUAL_PLACES.map((p) => (
            <li key={p} className={pill}>{p}</li>
          ))}
        </ul>
        {onFolder ? (
          <>
            <p className="text-xs text-zinc-400">If yours live somewhere else, pick the folder.</p>
            <button type="button" onClick={onFolder} className={`${btn} inline-flex mx-auto`}>
              <Folder size={13} aria-hidden="true" /> Pick a folder
            </button>
          </>
        ) : (
          <>
            <p className="text-xs text-zinc-400">If yours live somewhere else, zip the folder and upload it.</p>
            <button type="button" onClick={onUpload} className={`${btn} inline-flex mx-auto`}>
              <FileUp size={13} aria-hidden="true" /> Upload a file
            </button>
          </>
        )}
      </div>
    );
  }
  return (
    <div className="space-y-3" data-found-state={reading ? 'reading' : 'found'}>
      <p className="text-xs text-zinc-300">
        kwant looked in 5 usual places and read only folder names and counts. Tick what to bring. Nothing inside is read until you press Preview.
      </p>
      {previewError && <CopyErrorBox title={previewError.title} detail={previewError.detail} />}
      <ul className="space-y-2" aria-label="Found on this computer">
        {state.sources.map((s) => (
          <FoundRow key={s.id} s={s} on={ticked.has(s.id)} disabled={reading} onToggle={() => onToggle(s.id)} />
        ))}
      </ul>
      <SafeLine>Read-only. kwant never follows a link out of these folders, and never brings keys or tokens, even when a folder has them.</SafeLine>
    </div>
  );
};

const errorReason = (err: unknown) =>
  (err as { code?: string } | null)?.code === 'network' ? 'kwant couldn’t reach its server. Nothing was read.' : 'Nothing was read. Try again in a moment.';

/** The "Preview N sources" label (F2) and "Reading N sources…" (F6). */
export const foundPrimaryLabel = (n: number, reading: boolean) =>
  reading ? `Reading ${plural(n, 'source')}…` : n > 0 ? `Preview ${plural(n, 'source')}` : 'Preview';

// ------------------------------------------------------------------ Folder

const WhatWorks: React.FC = () => (
  <ul className="text-[11px] text-zinc-500 space-y-0.5 list-disc pl-5">
    <li>A skills folder (SKILL.md files)</li>
    <li>An OpenClaw, Hermes or Codex workspace (SOUL.md, AGENTS.md, skills/)</li>
    <li>A Paperclip company (COMPANY.md, agents/*.md)</li>
    <li>Up to 20 MB once packed. Only files an import reads are sent. Sign-in files, keys and .env stay on this computer.</li>
  </ul>
);

/** Reads a dropped folder (DataTransfer entries) into picked entries with relative paths. */
async function readDropped(items: DataTransferItemList): Promise<PickedEntry[] | null> {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const roots: any[] = [...items].map((i) => (i as DataTransferItem & { webkitGetAsEntry?: () => any }).webkitGetAsEntry?.()).filter(Boolean);
  if (roots.length !== 1 || !roots[0].isDirectory) return null;
  const out: PickedEntry[] = [];
  const walk = async (entry: any, prefix: string): Promise<void> => {
    if (entry.isFile) {
      const file: File = await new Promise((res, rej) => entry.file(res, rej));
      out.push({ path: `${prefix}${entry.name}`, size: file.size, file });
      return;
    }
    const reader = entry.createReader();
    for (;;) {
      const batch: any[] = await new Promise((res, rej) => reader.readEntries(res, rej));
      if (batch.length === 0) break;
      for (const e of batch) await walk(e, `${prefix}${entry.name}/`);
    }
  };
  await walk(roots[0], '');
  return out;
}

/** Files from `<input webkitdirectory>`: their relative paths include the picked folder. */
export const entriesFromFiles = (files: ReadonlyArray<File>): PickedEntry[] =>
  files.map((f) => ({ path: (f as File & { webkitRelativePath?: string }).webkitRelativePath || f.name, size: f.size, file: f }));

export const FolderStep: React.FC<{
  plan: FolderPlan | null;
  onPicked: (entries: PickedEntry[]) => void;
  inputRef: React.RefObject<HTMLInputElement>;
  /** 415 from the server (D4). */
  unsupported: boolean;
}> = ({ plan, onPicked, inputRef, unsupported }) => {
  const [over, setOver] = useState(false);
  const [dropNote, setDropNote] = useState<string | null>(null);
  const dirProps = { webkitdirectory: '', directory: '' } as Record<string, string>;
  return (
    <div className="space-y-3">
      {unsupported && (
        <CopyErrorBox title="Nothing kwant can bring in this folder" detail="kwant looked for SOUL.md, AGENTS.md, SKILL.md and agents/*.md and found none." />
      )}
      {!plan && !unsupported && <WhatWorks />}
      <div
        onDragOver={(e) => {
          e.preventDefault();
          setOver(true);
        }}
        onDragLeave={() => setOver(false)}
        onDrop={async (e) => {
          e.preventDefault();
          setOver(false);
          const got = await readDropped(e.dataTransfer.items).catch(() => null);
          if (got) {
            setDropNote(null);
            onPicked(got);
          } else setDropNote('Drop one folder here. For files, use Upload a file.');
        }}
        className={`rounded-xl border-2 border-dashed p-6 text-center space-y-2 ${over ? 'border-sky-500 bg-sky-500/5' : plan?.problem ? 'border-rose-500/50' : 'border-zinc-700'}`}
      >
        <Folder size={22} className="mx-auto text-zinc-500" aria-hidden="true" />
        {plan ? (
          <p className="text-xs text-zinc-300 break-words" data-picked>
            <strong className="font-semibold text-zinc-100">{plan.name}</strong>
            <span className="block text-[11px] text-zinc-500">
              {plural(plan.kept.length, 'file')} · {formatBytes(plan.keptBytes)}
            </span>
          </p>
        ) : (
          <p className="text-xs text-zinc-300">Drop a folder here, or</p>
        )}
        <button type="button" onClick={() => inputRef.current?.click()} className={`${btn} inline-flex`}>
          {plan ? 'Choose another' : 'Choose a folder'}
        </button>
        <input
          ref={inputRef}
          type="file"
          multiple
          {...dirProps}
          tabIndex={-1}
          aria-hidden="true"
          className="sr-only"
          onChange={(e) => {
            const files = [...(e.target.files ?? [])];
            if (files.length) onPicked(entriesFromFiles(files));
            e.target.value = '';
          }}
        />
        {dropNote && <p className="text-[11px] text-rose-300" role="alert">{dropNote}</p>}
      </div>
      {plan?.problem && (
        <p className="text-xs text-rose-300" role="alert" data-folder-problem>
          {plan.problem}
        </p>
      )}
      {plan && plan.left.length > 0 && (
        <div>
          <p className="text-[11px] font-medium text-zinc-400">Left out before packing</p>
          <ul className="list-disc pl-5 text-[11px] text-zinc-500">
            {plan.left.map((l) => (
              <li key={l.label}>
                {l.label}: {plural(l.count, 'file')}
              </li>
            ))}
          </ul>
        </div>
      )}
      {(plan || unsupported) && <WhatWorks />}
    </div>
  );
};

// ------------------------------------------------------------------ GitHub link

export const GithubStep: React.FC<{
  url: string;
  onUrl: (s: string) => void;
  /** Checked once the field was left or Preview was tried (G2). */
  checked: boolean;
  onCheck: () => void;
  fetching: boolean;
  error: unknown;
  phone: boolean;
}> = ({ url, onUrl, checked, onCheck, fetching, error, phone }) => {
  const id = useId();
  const bad = checked && url.trim() !== '' && !githubLinkOk(url);
  const notGithub = bad || (error as { code?: string } | null)?.code === 'not_github';
  const text = error && !notGithub ? githubErrorText(error) : null;
  return (
    <div className="space-y-3">
      <div>
        <label htmlFor={id} className="block text-xs font-medium text-zinc-300 mb-1">Link to a public GitHub repo</label>
        <input
          id={id}
          type="url"
          inputMode="url"
          autoComplete="off"
          spellCheck={false}
          autoFocus
          value={url}
          disabled={fetching}
          onChange={(e) => onUrl(e.target.value)}
          onBlur={onCheck}
          placeholder="https://github.com/owner/repo"
          aria-invalid={notGithub || undefined}
          aria-describedby={`${id}-help`}
          className={`w-full bg-zinc-950 border rounded-lg px-3 py-2 font-mono ${phone ? 'text-base' : 'text-xs'} text-zinc-100 focus:outline-none ${notGithub ? 'border-rose-500/70 focus:border-rose-400' : 'border-zinc-800 focus:border-sky-500'}`}
        />
        {notGithub && (
          <p className="mt-1 text-[11px] text-rose-300" role="alert">
            {NOT_GITHUB}
          </p>
        )}
        <p id={`${id}-help`} className="mt-1.5 text-[11px] text-zinc-500">
          A repo, a branch or a folder in it. kwant downloads it from github.com only, up to 20 MB.
        </p>
        <p className="mt-0.5 text-[11px] text-zinc-500">
          Example: <span className="font-mono">github.com/owner/repo/tree/main/skills</span>
        </p>
      </div>
      {fetching && (
        <p role="status" className="flex items-center gap-2 text-xs text-zinc-400">
          <Loader2 size={13} className={spin} aria-hidden="true" /> Downloading from github.com…
        </p>
      )}
      {text && <CopyErrorBox title={text.title} detail={text.detail} />}
    </div>
  );
};

/** G5/G6: the repo, ref and licence above the preview. */
export const GithubSourceBlock: React.FC<{ github: GithubInfo }> = ({ github: g }) => {
  const lic = licenceLine(g.licence);
  const licUrl = githubLicenceUrl(g);
  return (
    <div className={`${card} p-3 space-y-1.5`} data-github-source>
      <div className="flex items-center gap-2">
        <LinkIcon size={14} className="shrink-0 text-sky-300" aria-hidden="true" />
        <span className="flex-1 min-w-0 font-mono text-[12.5px] text-zinc-100 break-all">
          {g.owner}/{g.repo}
        </span>
        <a
          href={githubRepoUrl(g)}
          target="_blank"
          rel="noopener noreferrer"
          aria-label={`Open ${g.owner}/${g.repo} on GitHub`}
          className={`shrink-0 flex items-center gap-1 text-xs text-sky-300 hover:text-sky-200 rounded ${ring}`}
        >
          Open <ExternalLink size={12} aria-hidden="true" />
        </a>
      </div>
      <p className="pl-[22px] font-mono text-[11px] text-zinc-500 break-all">{githubRefLine(g)}</p>
      <p className="pl-[22px] flex flex-wrap items-center gap-x-2 gap-y-1">
        <span className={`kw-chip ${lic.warn ? 'kw-chip-warn' : ''} inline-flex items-center gap-1`} data-licence>
          <Scale size={12} aria-hidden="true" /> {lic.chip}
        </span>
        {lic.line && <span className="text-[11px] text-zinc-400">{lic.line}</span>}
        {g.licence && !g.licence.spdx && licUrl && (
          <a href={licUrl} target="_blank" rel="noopener noreferrer" className={`text-[11px] text-sky-300 hover:text-sky-200 underline underline-offset-2 rounded ${ring}`}>
            Read it
          </a>
        )}
      </p>
    </div>
  );
};

/** The choose step's "On this computer" group (C1, C2, C4). Not rendered in the web app (C3). */
export const FoundChooseRow: React.FC<{ state: LocalState; onOpen: () => void; card: string }> = ({ state, onOpen, card: rowCard }) => {
  if (state.kind === 'off') return null;
  const found = state.kind === 'ready' ? state.sources.filter((s) => !s.too_large) : [];
  const n = found.length;
  const hint =
    state.kind === 'loading'
      ? null
      : state.kind === 'error'
        ? 'Couldn’t look just now. Open to try again'
        : n === 0
          ? 'Nothing in the usual places. See where kwant looked'
          : `${foundHint(found)} · nothing is read yet`;
  const lit = state.kind === 'ready' && n > 0;
  return (
    <>
      <h3 className="text-xs font-semibold text-zinc-300">On this computer</h3>
      <button
        type="button"
        onClick={onOpen}
        data-found-row={state.kind === 'ready' ? (n ? 'found' : 'nothing') : state.kind}
        aria-label={lit ? `Found on this computer, ${n} found` : undefined}
        className={`${rowCard} w-full flex items-center gap-3 p-3 min-h-[60px] sm:min-h-0 text-left transition-colors ${lit ? 'border-sky-500/45 bg-sky-500/[.07] hover:border-sky-400/60' : 'hover:border-zinc-600'}`}
      >
        <HardDrive size={18} className={`shrink-0 ${lit ? 'text-sky-300' : 'text-zinc-400'}`} aria-hidden="true" />
        <span className="flex-1 min-w-0">
          <span className="block text-sm font-medium text-zinc-100">Found on this computer</span>
          {hint === null ? (
            <span role="status" className="flex items-center gap-1.5 text-[11px] text-zinc-500">
              <Loader2 size={11} className={spin} aria-hidden="true" /> {LOOKING}
            </span>
          ) : (
            <span className="block text-[11px] text-zinc-500">{hint}</span>
          )}
        </span>
        {lit && <span className="kw-chip kw-chip-accent shrink-0">{n} found</span>}
        <ChevronRight size={16} className="text-zinc-500 shrink-0" aria-hidden="true" />
      </button>
    </>
  );
};
