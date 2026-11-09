import React, { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { AlertCircle, ChevronDown, ChevronRight, Download, ExternalLink, FileText, Film, Loader2, RotateCw, X } from 'lucide-react';
import { AttachmentRef, PendingUpload, attachKind, attachmentUrl, fileKind, formatBytes, isInlineImage, isVideo, uploadProgressText } from '../attachments';

/**
 * A file on a message: images as a thumbnail (opens full size), everything else as a tile.
 * Both have a Download button; the server sends non-images as downloads only.
 */
export const FileTile: React.FC<{ file: AttachmentRef; small?: boolean }> = ({ file, small = false }) => {
  const [broken, setBroken] = useState(false);
  const [viewing, setViewing] = useState(false);
  const thumbRef = useRef<HTMLAnchorElement>(null);
  const image = isInlineImage(file.mime) && !broken;
  const video = isVideo(file.mime) && !broken;
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
      <figure className={`${small ? 'w-40' : 'w-full sm:w-auto'} max-w-full sm:max-w-xs rounded-xl overflow-hidden border border-zinc-800 bg-zinc-900/80`}>
        {/* A plain click opens the in-app viewer; Cmd/Ctrl/middle click still opens the original in a new tab */}
        <a
          ref={thumbRef}
          href={attachmentUrl(file.id)}
          target="_blank"
          rel="noopener noreferrer"
          aria-label={`View ${file.name} full size`}
          aria-haspopup="dialog"
          data-image-thumb=""
          onClick={(e) => {
            if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
            e.preventDefault();
            setViewing(true);
          }}
          className="block cursor-zoom-in focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
        >
          <img
            src={attachmentUrl(file.id)}
            alt={file.name}
            loading="lazy"
            decoding="async"
            onError={() => setBroken(true)}
            className={`block w-full sm:w-auto ${small ? 'max-h-24' : 'max-h-60'} object-contain bg-zinc-950`}
          />
        </a>
        {viewing && (
          <ImageViewer
            file={file}
            onClose={() => {
              setViewing(false);
              thumbRef.current?.focus();
            }}
          />
        )}
        <figcaption className="flex items-center gap-2 px-2.5 py-1.5">
          {label}
          {download}
        </figcaption>
      </figure>
    );
  }
  if (video) {
    // P2: a player for sent videos. No autoplay; only the metadata loads until Play. A browser that
    // can't decode it (some .mov in Chrome) errors and falls back to the file card below.
    return (
      <figure className="w-full sm:w-80 max-w-full rounded-xl overflow-hidden border border-zinc-800 bg-zinc-900/80">
        <video
          src={attachmentUrl(file.id)}
          controls
          preload="metadata"
          playsInline
          aria-label={file.name}
          onError={() => setBroken(true)}
          className="block w-full max-h-72 bg-black"
        />
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
        {isVideo(file.mime) ? <Film size={15} /> : <FileText size={15} />}
      </span>
      {label}
      {download}
    </div>
  );
};

/**
 * The full-size view of a chat image (P4): a dark scrim over the app, the picture fit to the screen,
 * Close (44 px) and Open original. Esc, Close or a click on the scrim closes it; focus starts on Close
 * and Tab stays inside. Nothing animates.
 */
export const ImageViewer: React.FC<{ file: AttachmentRef; onClose: () => void }> = ({ file, onClose }) => {
  const closeRef = useRef<HTMLButtonElement>(null);
  const boxRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    closeRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        onClose();
        return;
      }
      if (e.key !== 'Tab' || !boxRef.current) return;
      const stops = Array.from(boxRef.current.querySelectorAll<HTMLElement>('a[href], button'));
      if (!stops.length) return;
      const first = stops[0];
      const last = stops[stops.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    };
    document.addEventListener('keydown', onKey, true);
    return () => document.removeEventListener('keydown', onKey, true);
  }, [onClose]);
  const titleId = `viewer-${file.id}`;
  return createPortal(
    <div
      ref={boxRef}
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
      data-image-viewer=""
      className="fixed inset-0 z-[200] flex flex-col bg-black/95"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="flex items-center gap-2 px-3 py-2 sm:px-4 text-zinc-200 bg-black">
        <h2 id={titleId} className="min-w-0 flex-1 truncate text-sm font-medium" title={file.name}>
          {file.name}
        </h2>
        <a
          href={attachmentUrl(file.id)}
          target="_blank"
          rel="noopener noreferrer"
          className="inline-flex items-center gap-1.5 min-h-[44px] px-3 rounded-lg text-xs font-medium text-sky-300 hover:bg-white/10 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
        >
          <ExternalLink size={14} aria-hidden="true" />
          <span className="hidden sm:inline">Open original</span>
          <span className="sm:hidden">Original</span>
        </a>
        <button
          ref={closeRef}
          type="button"
          onClick={onClose}
          aria-label="Close"
          className="inline-flex items-center justify-center w-11 h-11 shrink-0 rounded-lg text-zinc-200 hover:bg-white/10 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
        >
          <X size={20} aria-hidden="true" />
        </button>
      </div>
      <div
        className="flex-1 min-h-0 flex items-center justify-center px-0 pb-0 sm:px-6 sm:pb-6"
        onClick={(e) => {
          if (e.target === e.currentTarget) onClose();
        }}
      >
        <img src={attachmentUrl(file.id)} alt={file.name} className="block max-w-full max-h-full object-contain" />
      </div>
    </div>,
    document.body,
  );
};

const timeLabel = (at: string | null) => {
  const t = at ? new Date(at) : null;
  return t && !Number.isNaN(t.getTime()) ? t.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' }) : null;
};

/**
 * Older copies of a file a bot shared again within a few minutes (P4): folded behind the newest as
 * "×3 · Show earlier versions", oldest first when open.
 */
export const EarlierCopies: React.FC<{ idBase: string; copies: Array<{ id: string; at: string | null; files: AttachmentRef[] }> }> = ({ idBase, copies }) => {
  const [open, setOpen] = useState(false);
  const total = copies.length + 1;
  const listId = `${idBase}-list`;
  return (
    <div className="mt-1.5 max-w-full" data-earlier-copies={copies.length}>
      <button
        type="button"
        aria-expanded={open}
        aria-controls={listId}
        onClick={() => setOpen((o) => !o)}
        className="inline-flex items-center gap-1 min-h-[32px] px-2 -ml-2 rounded-lg text-[11px] text-zinc-400 hover:text-zinc-200 hover:bg-zinc-900 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
      >
        {open ? <ChevronDown size={12} aria-hidden="true" /> : <ChevronRight size={12} aria-hidden="true" />}
        <span>
          ×{total} · {open ? 'Hide earlier versions' : 'Show earlier versions'}
        </span>
      </button>
      {open && (
        <ol id={listId} className="mt-1 flex flex-col gap-2" aria-label="Earlier versions, oldest first">
          {copies.map((c) => {
            const when = timeLabel(c.at);
            return (
              <li key={c.id} data-earlier-copy={c.id} className="max-w-full">
                {when && <span className="block mb-1 text-[10px] text-zinc-500">{when}</span>}
                <ul className="flex flex-wrap gap-2">
                  {c.files.map((f) => (
                    <li key={f.id} className="max-w-full">
                      <FileTile file={f} small />
                    </li>
                  ))}
                </ul>
              </li>
            );
          })}
        </ol>
      )}
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
  const video = isVideo(item.mime || item.attachment?.mime) || attachKind({ name: item.name, type: item.mime }) === 'video';
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
      ) : video ? (
        <Film size={12} className="shrink-0 text-zinc-400" aria-hidden="true" />
      ) : (
        <FileText size={12} className="shrink-0 text-zinc-400" aria-hidden="true" />
      )}
      <span className="min-w-0 flex-1">
        <span className="block truncate text-zinc-100" title={item.name}>
          {item.name}
        </span>
        <span className={`block ${failed ? 'text-rose-300 break-words' : 'truncate text-zinc-500'}`} role={failed ? 'alert' : undefined}>
          {failed ? item.error : item.state === 'uploading' ? (item.retrying ? 'Retrying…' : uploadProgressText(item.progress, item.size)) : formatBytes(item.size)}
        </span>
      </span>
      {failed && onRetry && item.retryable !== false && (
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
          aria-valuetext={item.retrying ? 'Retrying' : uploadProgressText(item.progress, item.size)}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={pct}
        />
      )}
    </li>
  );
};
