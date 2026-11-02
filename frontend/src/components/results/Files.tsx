import React from 'react';
import { File as FileIcon, Image as ImageIcon } from 'lucide-react';
import { FileItem, FilePreview, FilesBlock, inRange, isIndex, isObj, isText, parseCommon, uniqueIds } from '../../smartResults';
import type { RendererProps } from './registry';

function validPreview(v: unknown): FilePreview | undefined {
  if (v === null || v === undefined) return null;
  if (!isObj(v)) return undefined;
  if (v.kind === 'table') {
    if (!Array.isArray(v.columns) || !v.columns.every((c) => isText(c, 80))) return undefined;
    const columns = v.columns as string[];
    if (!Array.isArray(v.rows) || !v.rows.every((r) => Array.isArray(r) && r.length === columns.length && r.every((c) => isText(c, 300, true))))
      return undefined;
    return { kind: 'table', columns, rows: v.rows as string[][] };
  }
  if (v.kind === 'text') {
    if (!isText(v.excerpt, 2000, true)) return undefined;
    return { kind: 'text', excerpt: v.excerpt };
  }
  if (v.kind === 'image') return { kind: 'image' };
  return undefined;
}

/** 1-10 attached files, each with a name, type, size, and an optional preview (plan §A.3). */
export function validFiles(raw: unknown): FilesBlock | null {
  const b = parseCommon(raw, 'files');
  if (!b || b.source.kind !== 'slot' || !inRange(b.items, 1, 10) || !uniqueIds(b.items)) return null;
  const items: FileItem[] = [];
  for (const it of b.items as Record<string, unknown>[]) {
    if (!isText(it.attachment_id, 200) || !isText(it.name) || !isText(it.mime, 100)) return null;
    if (!isIndex(it.size_bytes)) return null;
    const preview = validPreview(it.preview);
    if (preview === undefined) return null;
    items.push({ id: it.id as string, attachment_id: it.attachment_id, name: it.name, mime: it.mime, size_bytes: it.size_bytes, preview });
  }
  return { id: b.id, type: 'files', title: b.title, source: b.source, provenance: b.provenance, items };
}

function humanSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let v = bytes / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v >= 10 ? Math.round(v) : Math.round(v * 10) / 10} ${units[i]}`;
}

const TablePreview: React.FC<{ columns: string[]; rows: string[][] }> = ({ columns, rows }) => (
  <div className="mt-2 overflow-x-auto rounded-lg border border-zinc-800">
    <table className="w-full min-w-max border-collapse text-left text-[11px]">
      <thead>
        <tr className="border-b border-zinc-800 bg-zinc-900/60">
          {columns.map((c, i) => (
            <th key={i} className="whitespace-nowrap px-2 py-1.5 font-medium text-zinc-400">
              {c}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {rows.map((r, ri) => (
          <tr key={ri} className="border-b border-zinc-900 last:border-0">
            {r.map((c, ci) => (
              <td key={ci} className="whitespace-nowrap px-2 py-1.5 text-zinc-300">
                {c}
              </td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  </div>
);

const FilePreviewView: React.FC<{ item: FileItem }> = ({ item }) => {
  const preview = item.preview;
  if (!preview) return null;
  if (preview.kind === 'table') return <TablePreview columns={preview.columns} rows={preview.rows} />;
  if (preview.kind === 'text') {
    return (
      <pre className="mt-2 max-h-48 overflow-auto whitespace-pre-wrap rounded-lg border border-zinc-800 bg-zinc-900/60 p-2 font-mono text-[11px] leading-relaxed text-zinc-300">
        {preview.excerpt}
      </pre>
    );
  }
  // image previews have no renderable source in this contract yet; show a labelled placeholder
  // rather than guessing at a URL (the attachment route isn't built — see the PR notes).
  return (
    <div className="mt-2 flex items-center gap-2 rounded-lg border border-dashed border-zinc-800 bg-zinc-900/40 px-3 py-4 text-[11px] text-zinc-500">
      <ImageIcon size={14} aria-hidden="true" />
      Image preview not available yet
    </div>
  );
};

/** Attached files: name, type, size, and a preview of the data itself when one is available. */
export const Files: React.FC<RendererProps<FilesBlock>> = ({ block }) => (
  <div data-smart-result="files" data-block-id={block.id}>
    <ul role="list" className="my-3.5 space-y-2">
      {block.items.map((item) => (
        <li key={item.id} className="rounded-xl border border-zinc-800 bg-zinc-950/70 p-3 shadow-sm">
          <div className="flex items-start gap-2">
            <FileIcon size={14} className="mt-0.5 shrink-0 text-zinc-500" aria-hidden="true" />
            <div className="min-w-0 flex-1">
              <div className="truncate text-xs font-medium text-zinc-200" title={item.name}>
                {item.name}
              </div>
              <div className="mt-0.5 text-[11px] text-zinc-500">
                {item.mime} · {humanSize(item.size_bytes)}
              </div>
              <FilePreviewView item={item} />
            </div>
          </div>
        </li>
      ))}
    </ul>
  </div>
);
