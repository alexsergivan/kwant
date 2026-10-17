import React, { useState } from 'react';
import { AlertCircle, Download, FileText, Loader2, RotateCw, X } from 'lucide-react';
import { AttachmentRef, PendingUpload, attachmentUrl, fileKind, formatBytes, isInlineImage } from '../attachments';

/**
 * A file on a message: images as a thumbnail (opens full size), everything else as a tile.
 * Both have a Download button; the server sends non-images as downloads only.
 */
export const FileTile: React.FC<{ file: AttachmentRef }> = ({ file }) => {
  const [broken, setBroken] = useState(false);
  const image = isInlineImage(file.mime) && !broken;
  const download = (
    <a
      href={attachmentUrl(file.id, true)}
      download={file.name}
      aria-label={`Download ${file.name}`}
      className="inline-flex items-center gap-1 shrink-0 px-2 py-1 rounded-lg text-[11px] font-medium text-sky-300 bg-sky-500/10 border border-sky-500/30 hover:bg-sky-500/20 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
    >
      <Download size={12} aria-hidden="true" />
      <span className="hidden sm:inline">Download</span>
    </a>
  );
  const label = (
    <span className="min-w-0 flex-1">
      <span className="block truncate text-xs font-medium text-zinc-100" title={file.name}>
        {file.name}
      </span>
      <span className="block text-[10px] text-zinc-500">
        {fileKind(file.mime, file.name)} · {formatBytes(file.size)}
      </span>
    </span>
  );

  if (image) {
    return (
      <figure className="w-full sm:w-auto max-w-full sm:max-w-xs rounded-xl overflow-hidden border border-zinc-800 bg-zinc-900/80">
        <a href={attachmentUrl(file.id)} target="_blank" rel="noopener noreferrer" className="block focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500">
          <img
            src={attachmentUrl(file.id)}
            alt={file.name}
            loading="lazy"
            decoding="async"
            onError={() => setBroken(true)}
            className="block w-full sm:w-auto max-h-60 object-contain bg-zinc-950"
          />
        </a>
        <figcaption className="flex items-center gap-2 px-2.5 py-1.5">
          {label}
          {download}
        </figcaption>
      </figure>
    );
  }
  return (
    <div className="flex items-center gap-2.5 w-full sm:w-72 max-w-full px-2.5 py-2 rounded-xl border border-zinc-800 bg-zinc-900/80">
      <span className="w-8 h-8 shrink-0 rounded-lg bg-zinc-800 border border-zinc-700/60 flex items-center justify-center text-zinc-400" aria-hidden="true">
        <FileText size={15} />
      </span>
      {label}
      {download}
    </div>
  );
};

export const FileTiles: React.FC<{ files: AttachmentRef[]; align?: 'start' | 'end' }> = ({ files, align = 'start' }) => (
  <ul className={`mt-2 flex flex-wrap gap-2 w-full ${align === 'end' ? 'justify-end' : ''}`} aria-label={files.length === 1 ? 'Attached file' : `${files.length} attached files`}>
    {files.map((f) => (
      <li key={f.id} className="max-w-full">
        <FileTile file={f} />
      </li>
    ))}
  </ul>
);

/** A file in the composer: uploading (with progress), ready, or failed (with the reason and Retry). */
export const UploadChip: React.FC<{ item: PendingUpload; onRemove: () => void; onRetry?: () => void }> = ({ item, onRemove, onRetry }) => {
  const pct = Math.round(item.progress * 100);
  const failed = item.state === 'error';
  return (
    <li
      className={`relative flex items-center gap-2 max-w-full sm:max-w-[16rem] pl-2 pr-1 py-1 rounded-lg border text-[11px] overflow-hidden ${
        failed ? 'border-rose-500/40 bg-rose-500/10' : 'border-zinc-700/70 bg-zinc-800/70'
      }`}
    >
      {item.state === 'uploading' ? (
        <Loader2 size={12} className="shrink-0 text-sky-300 animate-spin motion-reduce:animate-none" aria-hidden="true" />
      ) : failed ? (
        <AlertCircle size={12} className="shrink-0 text-rose-300" aria-hidden="true" />
      ) : (
        <FileText size={12} className="shrink-0 text-zinc-400" aria-hidden="true" />
      )}
      <span className="min-w-0 flex-1">
        <span className="block truncate text-zinc-100" title={item.name}>
          {item.name}
        </span>
        <span className={`block ${failed ? 'text-rose-300 break-words' : 'truncate text-zinc-500'}`} role={failed ? 'alert' : undefined}>
          {failed ? item.error : item.state === 'uploading' ? `Uploading… ${pct}%` : formatBytes(item.size)}
        </span>
      </span>
      {failed && onRetry && (
        <button
          type="button"
          onClick={onRetry}
          aria-label={`Retry ${item.name}`}
          className="p-1 rounded text-zinc-300 hover:text-white hover:bg-zinc-700 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
        >
          <RotateCw size={12} aria-hidden="true" />
        </button>
      )}
      <button
        type="button"
        onClick={onRemove}
        aria-label={item.state === 'uploading' ? `Cancel uploading ${item.name}` : `Remove ${item.name}`}
        className="p-1 rounded text-zinc-400 hover:text-white hover:bg-zinc-700 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
      >
        <X size={12} aria-hidden="true" />
      </button>
      {item.state === 'uploading' && (
        <span
          className="absolute left-0 bottom-0 h-0.5 bg-sky-400 transition-[width] duration-150"
          style={{ width: `${pct}%` }}
          role="progressbar"
          aria-label={`Uploading ${item.name}`}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={pct}
        />
      )}
    </li>
  );
};
