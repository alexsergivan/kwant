import React, { useCallback, useEffect, useRef, useState } from 'react';
import { AlertTriangle, Copy, FileText, Loader2, RotateCw, Trash2, Upload, X } from 'lucide-react';
import {
  DOC_ACCEPT,
  DOC_MAX_COUNT,
  SpaceDoc,
  SpaceDocList,
  checkFile,
  docKind,
  docMeta,
  formatSize,
  markOf,
  shortFolder,
  sortDocs,
  spaceDocsApi,
} from '../spaceDocs';
import { DocMark } from './DocMark';
import { DocViewerOpen, DocViewerSheet } from './DocViewerSheet';

const focusRing = 'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400';
const primaryBtn = `inline-flex items-center justify-center gap-1.5 px-3 rounded-lg text-xs font-medium text-white bg-violet-600 hover:bg-violet-500 transition-colors motion-reduce:transition-none disabled:opacity-50 ${focusRing}`;
const quietBtn = `inline-flex items-center justify-center gap-1.5 px-3 rounded-lg text-xs font-medium text-zinc-200 border border-zinc-700 bg-zinc-900 hover:bg-zinc-800 ${focusRing}`;
const dangerBtn = `inline-flex items-center justify-center gap-1.5 px-3 rounded-lg text-xs font-medium bg-rose-300/[.12] border border-rose-300/40 text-rose-100 hover:bg-rose-300/20 ${focusRing}`;
const tooBig = (size: number) => `It’s ${formatSize(size)}, over the 2 MB bots can read. Save a smaller copy or split it.`;

/** A file the person chose: being added, or why it wasn't. */
type Pending =
  | { id: number; file: File; state: 'adding' }
  | { id: number; file: File; state: 'problem'; text: string }
  | { id: number; file: File; state: 'exists' };

/** Words for a file that wasn't added (design §1 States). */
export function problemText(kind: string, file: { name: string; size: number }): string {
  switch (kind) {
    case 'bad_name':
      return 'Names can’t start with a dot or contain “..”. Rename it, then add it.';
    case 'wrong_kind':
      return 'Bots read .md, .txt and .pdf. Save it as a PDF, then add it.';
    case 'too_large':
      return tooBig(file.size);
    case 'no_text':
      return 'It has no text bots can read (it looks like a scan). Export it with text, or paste the words into a .txt.';
    case 'limit':
      return 'This space already has 200 documents. Remove one to add another.';
    default:
      return 'Something went wrong while adding it. Try again.';
  }
}

/** Settings › Documents (design §1): see, add and remove this space's files. */
export const SpaceDocsTab: React.FC<{ spaceId: string; spaceName: string; isPhone?: boolean }> = ({ spaceId, isPhone = false }) => {
  const [list, setList] = useState<SpaceDocList | null>(null);
  const [failed, setFailed] = useState(false);
  // Add, remove and the page text come in the second server step: hidden until the server has them.
  const [canEdit, setCanEdit] = useState(false);
  const [pending, setPending] = useState<Pending[]>([]);
  const [confirm, setConfirm] = useState<string | null>(null);
  const [said, setSaid] = useState('');
  const [over, setOver] = useState(false);
  const [viewer, setViewer] = useState<DocViewerOpen | null>(null);
  const [copied, setCopied] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const chooseRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const nextId = useRef(1);
  const queue = useRef<Promise<void>>(Promise.resolve());

  const load = useCallback(() => {
    setFailed(false);
    return spaceDocsApi.list(spaceId).then(
      (l) => setList(l),
      () => setFailed(true),
    );
  }, [spaceId]);

  useEffect(() => {
    setList(null);
    setPending([]);
    setConfirm(null);
    load();
    let live = true;
    spaceDocsApi.hasRoutes(spaceId).then((yes) => live && setCanEdit(yes));
    return () => {
      live = false;
    };
  }, [spaceId, load]);

  const docs = list ? sortDocs(list.docs) : [];
  const atLimit = docs.length >= DOC_MAX_COUNT;

  const update = (id: number, p: Pending | null) => setPending((all) => (p ? all.map((x) => (x.id === id ? p : x)) : all.filter((x) => x.id !== id)));

  const send = async (id: number, file: File, replace = false) => {
    update(id, { id, file, state: 'adding' });
    const r = await spaceDocsApi.add(spaceId, file, replace);
    if (r.ok) {
      update(id, null);
      setSaid(`${file.name} added`);
      await load();
    } else if (r.kind === 'exists') update(id, { id, file, state: 'exists' });
    else if (r.kind === 'no_route') {
      update(id, null);
      setCanEdit(false);
    } else update(id, { id, file, state: 'problem', text: problemText(r.kind, file) });
  };

  /** Several files at once go one at a time, each with its own row. */
  const addFiles = (files: File[]) => {
    if (!canEdit || !files.length) return;
    const rows: Pending[] = files.map((file) => {
      const id = nextId.current++;
      const bad = checkFile(file);
      return bad ? { id, file, state: 'problem', text: problemText(bad.kind, file) } : { id, file, state: 'adding' };
    });
    setPending((all) => [...rows, ...all]);
    for (const row of rows) if (row.state === 'adding') queue.current = queue.current.then(() => send(row.id, row.file));
  };

  const remove = async (name: string) => {
    const at = docs.findIndex((d) => d.name === name);
    const r = await spaceDocsApi.remove(spaceId, name);
    setConfirm(null);
    if (r === 'no_route') {
      setCanEdit(false);
      return;
    }
    if (r === 'failed') {
      setSaid(`${name} wasn’t removed. Try again.`);
      return;
    }
    setSaid(`${name} removed`);
    const next = docs.filter((d) => d.name !== name);
    await load();
    // Focus moves to the next row's remove, or to Choose files when the list is empty.
    requestAnimationFrame(() => {
      const target = next[Math.min(at, next.length - 1)];
      const el = target ? panelRef.current?.querySelector<HTMLElement>(`[data-doc-remove="${CSS.escape(target.name)}"]`) : chooseRef.current;
      (el ?? chooseRef.current)?.focus();
    });
  };

  // Drop anywhere on the tab.
  const dragProps = canEdit && !isPhone && !atLimit
    ? {
        onDragOver: (e: React.DragEvent) => {
          if (![...e.dataTransfer.types].includes('Files')) return;
          e.preventDefault();
          setOver(true);
        },
        onDragLeave: (e: React.DragEvent) => {
          if (!panelRef.current?.contains(e.relatedTarget as Node)) setOver(false);
        },
        onDrop: (e: React.DragEvent) => {
          e.preventDefault();
          setOver(false);
          addFiles([...e.dataTransfer.files]);
        },
      }
    : {};

  const choose = (
    <button
      ref={chooseRef}
      type="button"
      onClick={() => inputRef.current?.click()}
      className={`${primaryBtn} ${isPhone ? 'w-full h-11' : 'h-8'}`}
      data-choose-files
    >
      <Upload size={14} aria-hidden="true" /> Choose files
    </button>
  );

  const dropZone =
    !canEdit || !list || docs.length === 0 ? null : atLimit ? (
      <div className="flex items-start gap-2.5 rounded-xl border border-zinc-800 bg-zinc-900/60 p-3.5 text-[13px] text-zinc-300" data-docs-limit>
        <AlertTriangle size={14} className="mt-[3px] shrink-0 text-amber-300" aria-hidden="true" />
        <p>
          <strong className="font-semibold text-zinc-100">200 documents</strong>, the most bots can use in one space. Remove one to add another.
        </p>
      </div>
    ) : isPhone ? (
      <div className="space-y-2 p-4 rounded-xl border border-zinc-800 bg-zinc-900/50" data-docs-drop>
        <p className="text-[13px] font-medium text-zinc-100">Add your files</p>
        <p className="text-xs text-zinc-400">.md, .txt or a PDF with text in it · up to 2 MB each</p>
        {choose}
      </div>
    ) : (
      <div
        className={`flex items-center gap-3.5 p-4 rounded-xl border border-dashed transition-colors motion-reduce:transition-none ${over ? 'border-violet-400 bg-violet-400/[.06]' : 'border-zinc-700 bg-zinc-900/50'}`}
        data-docs-drop
        data-over={over || undefined}
      >
        <span className="shrink-0 inline-flex items-center justify-center w-9 h-9 rounded-[10px] bg-zinc-900 border border-zinc-800" aria-hidden="true">
          <Upload size={18} className="text-violet-300" />
        </span>
        <div className="min-w-0 flex-1">
          <p className="text-[13px] font-medium text-zinc-100">Drop files here, or choose them</p>
          <p className="text-xs text-zinc-400">.md, .txt or a PDF with text in it · up to 2 MB each</p>
        </div>
        {choose}
      </div>
    );

  const empty = list && docs.length === 0 && (
    <div
      className={`flex flex-col items-center text-center gap-2 px-4 py-8 rounded-xl border ${isPhone ? 'border-zinc-800' : 'border-dashed'} ${
        over ? 'border-violet-400 bg-violet-400/[.06]' : `${isPhone ? '' : 'border-zinc-700'} bg-zinc-900/50`
      }`}
      data-docs-empty
    >
      <span className="inline-flex items-center justify-center w-10 h-10 rounded-[10px] bg-zinc-900 border border-zinc-800" aria-hidden="true">
        <FileText size={18} className="text-violet-300" />
      </span>
      <p className="text-sm font-medium text-zinc-100">No documents yet</p>
      <p className="max-w-[420px] text-[13px] text-zinc-300 leading-relaxed">
        Add your FAQ, returns policy or price list. Bots in this space quote them word for word, so answers come from your own papers, not from memory.
      </p>
      {canEdit && <div className={isPhone ? 'w-full mt-1' : 'mt-1'}>{choose}</div>}
      <p className="text-xs text-zinc-400">
        {canEdit && !isPhone ? 'or drop them here · ' : ''}.md, .txt or a PDF with text in it · up to 2 MB each
      </p>
    </div>
  );

  const pendingRow = (p: Pending) => {
    const name = p.file.name;
    if (p.state === 'adding')
      return (
        <li key={`p${p.id}`} className={`flex items-center gap-3 ${isPhone ? 'p-3' : 'px-3.5 py-3'}`} data-doc-pending="adding">
          <DocMark name={name} />
          <div className="min-w-0 flex-1">
            <p className="text-sm font-medium text-zinc-100 truncate" title={name}>
              {name}
            </p>
            <p className="flex items-center gap-1 text-xs text-zinc-400">
              <Loader2 size={12} className="motion-safe:animate-spin" aria-hidden="true" /> Adding…
            </p>
          </div>
        </li>
      );
    const dismiss = (
      <button
        type="button"
        aria-label="Dismiss"
        onClick={() => update(p.id, null)}
        className={`shrink-0 inline-flex items-center justify-center rounded-lg text-zinc-400 hover:bg-zinc-800 hover:text-zinc-100 ${isPhone ? 'w-11 h-11 -my-1.5 -mr-2' : 'w-8 h-8'} ${focusRing}`}
      >
        <X size={16} aria-hidden="true" />
      </button>
    );
    if (p.state === 'exists')
      return (
        <li key={`p${p.id}`} className={`bg-amber-300/[.05] ${isPhone ? 'p-3' : 'px-3.5 py-3'}`} data-doc-pending="exists">
          <div className={isPhone ? 'space-y-2.5' : 'flex items-center gap-3'}>
            <div className="flex items-center gap-3 min-w-0 flex-1">
              <DocMark name={name} />
              <div className="min-w-0 flex-1">
                <p className="text-sm font-medium text-zinc-100 truncate" title={name}>
                  {name} is already here
                </p>
                <p className="text-xs text-zinc-300">Replace it with the one you just chose?</p>
              </div>
            </div>
            <div className={isPhone ? 'grid grid-cols-2 gap-2' : 'flex shrink-0 gap-2'}>
              <button type="button" className={`${quietBtn} ${isPhone ? 'h-11' : 'h-7'}`} onClick={() => update(p.id, null)}>
                Keep old
              </button>
              <button type="button" className={`${primaryBtn} ${isPhone ? 'h-11' : 'h-7'}`} onClick={() => (queue.current = queue.current.then(() => send(p.id, p.file, true)))}>
                Replace
              </button>
            </div>
          </div>
        </li>
      );
    const shownMark = docKind(name) ? name : `x.${markOf(name).toLowerCase()}`;
    return (
      <li key={`p${p.id}`} className={`flex items-start gap-3 bg-amber-300/[.05] ${isPhone ? 'p-3' : 'px-3.5 py-3'}`} data-doc-pending="problem">
        <DocMark name={shownMark} />
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium text-zinc-100 truncate" title={name}>
            {name} wasn’t added
          </p>
          <p className="flex items-start gap-1 text-xs text-zinc-300">
            <AlertTriangle size={12} className="mt-0.5 shrink-0 text-amber-300" aria-hidden="true" />
            <span>{p.text}</span>
          </p>
        </div>
        {dismiss}
      </li>
    );
  };

  const docRow = (d: SpaceDoc) => {
    if (confirm === d.name)
      return (
        <li key={d.name} className={`bg-rose-300/[.06] ${isPhone ? 'p-3' : 'px-3.5 py-3'}`} data-doc-confirm={d.name}>
          <RemoveConfirm name={d.name} isPhone={isPhone} onCancel={() => setConfirm(null)} onRemove={() => remove(d.name)} />
        </li>
      );
    return (
      <li key={d.name} className={`flex items-center gap-3 ${isPhone ? 'p-3' : 'px-3.5 py-3'}`} data-doc={d.name}>
        <button
          type="button"
          className={`flex items-center gap-3 min-w-0 flex-1 -m-1.5 p-1.5 rounded-lg text-left hover:bg-zinc-900/70 ${focusRing}`}
          onClick={() =>
            setViewer({ space: spaceId, file: d.name, page: 1, size: d.size, modified: d.modified, pages: d.pages })
          }
          data-doc-open={d.name}
        >
          <DocMark name={d.name} />
          <span className="min-w-0 flex-1">
            <span className="block text-sm font-medium text-zinc-100 truncate" title={d.name}>
              {d.name}
            </span>
            {d.too_large ? (
              <span className="flex items-start gap-1 text-xs text-amber-200">
                <AlertTriangle size={12} className="mt-0.5 shrink-0 text-amber-300" aria-hidden="true" />
                <span>{formatSize(d.size)}, over the 2 MB bots can read. Save a smaller copy or split it.</span>
              </span>
            ) : (
              <span className="block text-xs text-zinc-400 truncate">{docMeta(d)}</span>
            )}
          </span>
        </button>
        {canEdit && (
          <button
            type="button"
            aria-label={`Remove ${d.name}`}
            onClick={() => setConfirm(d.name)}
            data-doc-remove={d.name}
            className={`shrink-0 inline-flex items-center justify-center rounded-lg text-zinc-400 hover:bg-zinc-800 hover:text-zinc-100 ${isPhone ? 'w-11 h-11 -my-1.5 -mr-2' : 'w-8 h-8'} ${focusRing}`}
          >
            <Trash2 size={16} aria-hidden="true" />
          </button>
        )}
      </li>
    );
  };

  const count = docs.length;
  const copyFolder = () => {
    if (!list) return;
    navigator.clipboard?.writeText(list.folder).catch(() => {});
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1500);
  };

  return (
    <div ref={panelRef} className="relative" data-space-docs {...dragProps}>
      <p className="text-[13px] text-zinc-300 leading-relaxed mb-3.5">
        Your FAQ, policies and price lists. Bots in this space answer from these files and quote them word for word, and every quote is checked against the file.
      </p>
      <input
        ref={inputRef}
        type="file"
        multiple
        accept={DOC_ACCEPT}
        className="hidden"
        tabIndex={-1}
        aria-hidden="true"
        onChange={(e) => {
          addFiles([...(e.target.files ?? [])]);
          e.target.value = '';
        }}
      />
      {list === null && !failed && canEdit && !isPhone && (
        <div className="flex items-center gap-3.5 p-4 rounded-xl border border-dashed border-zinc-700 bg-zinc-900/50">
          <span className="shrink-0 inline-flex items-center justify-center w-9 h-9 rounded-[10px] bg-zinc-900 border border-zinc-800" aria-hidden="true">
            <Upload size={18} className="text-violet-300" />
          </span>
          <div className="min-w-0 flex-1">
            <p className="text-[13px] font-medium text-zinc-100">Drop files here, or choose them</p>
            <p className="text-xs text-zinc-400">.md, .txt or a PDF with text in it · up to 2 MB each</p>
          </div>
          {choose}
        </div>
      )}
      {dropZone}
      {empty}

      {(list === null || failed || count > 0 || pending.length > 0) && (
        <div className="mt-5">
          <div className="flex items-baseline justify-between gap-3 mb-2">
            <p className="text-[11px] font-semibold tracking-[.08em] uppercase text-zinc-300" role={list === null && !failed ? 'status' : undefined}>
              {list === null && !failed ? 'Loading documents…' : 'In this space'}
            </p>
            {list && count > 0 && <p className="text-[11px] text-zinc-500">{count === 1 ? '1 document' : `${count} documents`}</p>}
          </div>
          <ul className="rounded-xl border border-zinc-800 bg-zinc-900/60 divide-y divide-zinc-800 overflow-hidden" aria-label="Documents in this space">
            {pending.map(pendingRow)}
            {failed ? (
              <li className="flex flex-wrap items-center gap-2 px-3.5 py-3 text-[13px] text-zinc-300" data-docs-failed>
                <AlertTriangle size={14} className="text-amber-300" aria-hidden="true" />
                Couldn’t load this space’s documents.
                <button type="button" className={`${quietBtn} h-8`} onClick={() => load()}>
                  <RotateCw size={12} aria-hidden="true" /> Try again
                </button>
              </li>
            ) : list === null ? (
              [48, 36, 56].map((w, i) => (
                <li key={i} className="flex items-center gap-3 px-3.5 py-3" aria-hidden="true" data-docs-skeleton>
                  <span className="w-8 h-8 rounded-lg border border-zinc-700" />
                  <span className="flex-1 space-y-1.5">
                    <span className="block h-3 rounded bg-zinc-800" style={{ width: `${w}%` }} />
                    <span className="block h-2.5 rounded bg-zinc-800" style={{ width: `${w - 14}%` }} />
                  </span>
                </li>
              ))
            ) : (
              docs.map(docRow)
            )}
          </ul>
          {list?.more && (
            <p className="mt-2 text-xs text-zinc-400" data-docs-more>
              {list.more_count
                ? `The folder has ${list.more_count} more files than this. Bots only see the first 200, by name.`
                : 'The folder has more files than this. Bots only see the first 200, by name.'}
            </p>
          )}
        </div>
      )}

      {list && (
        <p className="mt-4 text-xs text-zinc-400 leading-relaxed [overflow-wrap:anywhere]" data-docs-folder>
          Files you put in{' '}
          <code className="text-zinc-300" title={list.folder}>
            {shortFolder(list.folder)}
          </code>{' '}
          show here too.{' '}
          <button type="button" onClick={copyFolder} className={`inline-flex items-center gap-1 align-middle rounded text-violet-300 hover:text-violet-200 hover:underline ${isPhone ? 'min-h-[44px]' : ''} ${focusRing}`}>
            <Copy size={12} aria-hidden="true" /> {copied ? 'Copied' : 'Copy folder'}
          </button>
        </p>
      )}

      <p className="sr-only" role="status" aria-live="polite">
        {said}
      </p>
      {viewer && <DocViewerSheet open={viewer} onClose={() => setViewer(null)} />}
    </div>
  );
};

/** The inline remove confirm: focus on Cancel, Esc cancels. */
const RemoveConfirm: React.FC<{ name: string; isPhone: boolean; onCancel: () => void; onRemove: () => void }> = ({ name, isPhone, onCancel, onRemove }) => {
  const cancelRef = useRef<HTMLButtonElement>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => cancelRef.current?.focus(), []);
  return (
    <div
      className={isPhone ? 'space-y-2.5' : 'flex items-center gap-3'}
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          e.stopPropagation();
          onCancel();
        }
      }}
    >
      <div className="flex items-center gap-3 min-w-0 flex-1">
        <DocMark name={name} />
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium text-zinc-100 truncate" title={name}>
            Remove {name}?
          </p>
          <p className="text-xs text-zinc-300">Bots can’t quote it after this. Replies that already quote it keep their words.</p>
        </div>
      </div>
      <div className={isPhone ? 'grid grid-cols-2 gap-2' : 'flex shrink-0 gap-2'}>
        <button ref={cancelRef} type="button" className={`${quietBtn} ${isPhone ? 'h-11' : 'h-8'}`} onClick={onCancel}>
          Cancel
        </button>
        <button
          type="button"
          disabled={busy}
          className={`${dangerBtn} ${isPhone ? 'h-11' : 'h-8'}`}
          onClick={() => {
            setBusy(true);
            onRemove();
          }}
        >
          <Trash2 size={14} aria-hidden="true" /> Remove
        </button>
      </div>
    </div>
  );
};
