import React, { useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { AlertCircle, Download, ExternalLink, FileText, Film, Loader2, RotateCw, X } from 'lucide-react';
import { AttachmentRef, PendingUpload, attachKind, attachmentUrl, fileKind, formatBytes, isInlineImage, isVideo, uploadProgressText } from '../attachments';

/**
 * A file on a message: images as a thumbnail (opens full size), everything else as a tile.
 * Both have a Download button; the server sends non-images as downloads only.
 */
export const FileTile: React.FC<{
  file: AttachmentRef;
  small?: boolean;
  /** A bot's shared file: names the bot in the alt text and viewer, and caps the tile at 240 px on desktop. */
  sharedBy?: string;
  /** When it was shared, for the viewer's bottom bar. */
  at?: string | null;
}> = ({ file, small = false, sharedBy, at = null }) => {
  const [broken, setBroken] = useState(false);
  const [viewing, setViewing] = useState(false);
  const thumbRef = useRef<HTMLAnchorElement>(null);
  const closeViewer = useCallback(() => {
    setViewing(false);
    thumbRef.current?.focus();
  }, []);
  const image = isInlineImage(file.mime) && !broken;
  const video = isVideo(file.mime) && !broken;
  // Shared tiles and small copies get an icon-only 32 px chip, so the name has room; a transparent
  // ring around it makes the hit area 44 px. Other tiles are 44 px on phones from min size.
  const iconOnly = small || !!sharedBy;
  const download = (
    <a
      href={attachmentUrl(file.id, true)}
      download={file.name}
      aria-label={`Download ${file.name}`}
      title="Download"
      data-download=""
      className={`inline-flex items-center justify-center gap-1 shrink-0 rounded-lg text-[11px] font-medium text-sky-300 bg-sky-500/10 border border-sky-500/30 hover:bg-sky-500/20 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500 ${
        iconOnly ? "relative w-8 h-8 before:absolute before:-inset-[7px] before:content-['']" : 'px-2 py-1 max-sm:min-w-11 max-sm:min-h-11'
      }`}
    >
      <Download size={iconOnly ? 14 : 12} aria-hidden="true" />
      <span className={iconOnly ? 'sr-only' : 'hidden sm:inline'}>Download</span>
    </a>
  );
  const label = (
    <span className="min-w-0 flex-1">
      <span className="block truncate text-xs font-medium text-zinc-100" title={file.name}>
        {file.name}
      </span>
      <span className={`block text-[10px] text-zinc-500${small ? ' whitespace-nowrap' : ''}`}>
        {fileKind(file.mime, file.name)} · {formatBytes(file.size)}
      </span>
    </span>
  );

  if (image) {
    return (
      <figure className={`${small ? 'w-40' : 'w-full sm:w-auto'} max-w-full ${sharedBy ? 'sm:max-w-[240px]' : 'sm:max-w-xs'} rounded-xl overflow-hidden border border-zinc-800 bg-zinc-900/80`}>
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
          className="block cursor-zoom-in focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-violet-400"
        >
          <img
            src={attachmentUrl(file.id)}
            alt={sharedBy ? `${file.name}, shared by ${sharedBy}` : file.name}
            loading="lazy"
            decoding="async"
            onError={() => setBroken(true)}
            className={`block ${small ? 'w-full h-24' : 'w-full sm:w-auto max-h-60'} object-contain bg-zinc-900`}
          />
        </a>
        {viewing && (
          <ImageViewer
            file={file}
            sharedBy={sharedBy}
            at={at}
            onClose={closeViewer}
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
 * The full-size view of a chat image (P4, Mira's spec): an always-dark 85% scrim, the picture fit to
 * the screen (100dvh, so iOS Safari doesn't cut it off), Close top right, and a bottom bar with the
 * file, bot and time on the left and Open original on the right. Esc, Close or a tap on the scrim
 * closes it; focus starts on Close, Tab stays inside, the page behind doesn't scroll, and pinch-zoom
 * is left to the browser. It fades in over 150 ms, with no motion under reduced motion.
 */
export const ImageViewer: React.FC<{ file: AttachmentRef; sharedBy?: string; at?: string | null; onClose: () => void }> = ({
  file,
  sharedBy,
  at = null,
  onClose,
}) => {
  const closeRef = useRef<HTMLButtonElement>(null);
  const boxRef = useRef<HTMLDivElement>(null);
  // The chat re-renders on every streamed token: keep the latest onClose in a ref so the setup below
  // runs once on open and never pulls focus back to Close while a reply streams.
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;
  useEffect(() => {
    closeRef.current?.focus();
    const body = document.body;
    const prevOverflow = body.style.overflow;
    body.style.overflow = 'hidden';
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        onCloseRef.current();
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
    return () => {
      document.removeEventListener('keydown', onKey, true);
      body.style.overflow = prevOverflow;
    };
  }, []);
  const titleId = `viewer-${file.id}`;
  const when = timeLabel(at);
  const closeOnScrim = (e: React.MouseEvent) => {
    if (e.target === e.currentTarget) onClose();
  };
  return createPortal(
    <div
      ref={boxRef}
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
      data-image-viewer=""
      className="kw-viewer-in fixed inset-0 z-[200] flex flex-col bg-black/85"
      style={{ height: '100dvh' }}
      onClick={closeOnScrim}
    >
      <button
        ref={closeRef}
        type="button"
        onClick={onClose}
        aria-label="Close"
        className="absolute right-2 z-10 inline-flex items-center justify-center w-11 h-11 rounded-full bg-zinc-900/80 text-zinc-100 hover:bg-zinc-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-violet-400"
        style={{ top: 'calc(env(safe-area-inset-top, 0px) + 0.5rem)' }}
      >
        <X size={20} aria-hidden="true" />
      </button>
      <div className="flex-1 min-h-0 flex items-center justify-center sm:p-6" style={{ paddingTop: 'calc(env(safe-area-inset-top, 0px) + 3.5rem)' }} onClick={closeOnScrim}>
        <img
          src={attachmentUrl(file.id)}
          alt={sharedBy ? `${file.name}, shared by ${sharedBy}` : file.name}
          className="block max-w-[100vw] max-h-full object-contain"
          style={{ touchAction: 'pinch-zoom' }}
        />
      </div>
      <div
        className="flex items-center gap-3 px-3 sm:px-4 bg-black text-zinc-300"
        style={{ paddingBottom: 'env(safe-area-inset-bottom, 0px)' }}
      >
        {/* Phones put the bot and time on a second line so the time is never cut off */}
        <p id={titleId} className="min-w-0 flex-1 text-xs py-2 sm:py-3 sm:truncate" title={file.name}>
          <span className="block sm:inline truncate font-medium text-zinc-100">{file.name}</span>
          {(sharedBy || when) && (
            <span className="block sm:inline truncate">
              <span className="hidden sm:inline"> · </span>
              {[sharedBy, when].filter(Boolean).join(' · ')}
            </span>
          )}
        </p>
        <a
          href={attachmentUrl(file.id)}
          target="_blank"
          rel="noopener"
          className="inline-flex items-center gap-1.5 shrink-0 min-h-[44px] px-3 rounded-lg text-xs font-medium text-sky-300 hover:bg-white/10 focus-visible:outline focus-visible:outline-2 focus-visible:outline-violet-400"
        >
          <ExternalLink size={14} aria-hidden="true" />
          Open original
        </a>
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
 * "Shared 3 times · Show 2 earlier" in plain grey text under the tile, oldest first when open.
 */
export const EarlierCopies: React.FC<{
  idBase: string;
  sharedBy?: string;
  copies: Array<{ id: string; at: string | null; files: AttachmentRef[] }>;
}> = ({ idBase, sharedBy, copies }) => {
  const [open, setOpen] = useState(false);
  const hidden = copies.length;
  const listId = `${idBase}-list`;
  const fileName = copies[0]?.files.map((f) => f.name).join(', ') || 'this file';
  return (
    <div className="max-w-full" data-earlier-copies={hidden}>
      <button
        type="button"
        aria-expanded={open}
        aria-controls={listId}
        aria-label={open ? `Hide earlier copies of ${fileName}` : `Show ${hidden} earlier ${hidden === 1 ? 'copy' : 'copies'} of ${fileName}`}
        onClick={() => setOpen((o) => !o)}
        // 44 px tall hit area on phones from padding; the text stays 12 px
        className="block text-left py-3.5 sm:py-1 text-xs text-zinc-400 hover:text-zinc-200 rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-violet-400"
      >
        {open ? 'Hide earlier' : `Shared ${hidden + 1} times · Show ${hidden} earlier`}
      </button>
      {open && (
        <ol id={listId} className="mt-1 flex flex-col gap-2" aria-label="Earlier copies, oldest first">
          {copies.map((c) => {
            const when = timeLabel(c.at);
            return (
              <li key={c.id} data-earlier-copy={c.id} className="max-w-full">
                {when && <span className="block mb-1 text-[11px] text-zinc-500">{when}</span>}
                <ul className="flex flex-wrap gap-2">
                  {c.files.map((f) => (
                    <li key={f.id} className="max-w-full">
                      <FileTile file={f} small sharedBy={sharedBy} at={c.at} />
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

export const FileTiles: React.FC<{ files: AttachmentRef[]; align?: 'start' | 'end'; sharedBy?: string; at?: string | null }> = ({
  files,
  align = 'start',
  sharedBy,
  at = null,
}) => (
  <ul className={`mt-2 flex flex-wrap gap-2 w-full ${align === 'end' ? 'justify-end' : ''}`} aria-label={files.length === 1 ? 'Attached file' : `${files.length} attached files`}>
    {files.map((f) => (
      <li key={f.id} className={`max-w-full${sharedBy ? ' w-full sm:w-auto' : ''}`}>
        <FileTile file={f} sharedBy={sharedBy} at={at} />
      </li>
    ))}
  </ul>
);

/** A file in the composer: uploading (with progress), ready, or failed (with the reason and Retry). */
/**
 * `fresh` is the chip's place in a drop that just landed (0, 1, 2…): it drops into the tray with a
 * short stagger and a tint that fades (0.8.0 drag and drop). Everything else about the chip is as before.
 */
export const UploadChip: React.FC<{ item: PendingUpload; onRemove: () => void; onRetry?: () => void; fresh?: number }> = ({ item, onRemove, onRetry, fresh }) => {
  const pct = Math.round(item.progress * 100);
  const failed = item.state === 'error';
  const video = isVideo(item.mime || item.attachment?.mime) || attachKind({ name: item.name, type: item.mime }) === 'video';
  return (
    <li
      className={`relative flex items-center gap-2 max-w-full sm:max-w-[16rem] pl-2 pr-1 py-1 rounded-lg border text-[11px] overflow-hidden ${
        failed ? 'border-rose-500/40 bg-rose-500/10' : 'border-zinc-700/70 bg-zinc-800/70'
      }${fresh !== undefined ? ' kw-chip-new' : ''}`}
      data-fresh={fresh !== undefined ? 'true' : undefined}
      style={fresh !== undefined ? ({ '--kw-chip-delay': `${40 + (fresh === 0 ? 0 : fresh === 1 ? 30 : 60)}ms` } as React.CSSProperties) : undefined}
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
