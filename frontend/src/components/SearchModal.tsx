import React, { useEffect, useMemo, useRef, useState } from 'react';
import { Hash, Loader2, MessageSquare, Search, X } from 'lucide-react';
import { SearchHit, continuityApi, formatWhen, groupHits, snippetParts } from '../continuity';
import type { Kwant } from './Sidebar';

/** Renders a server snippet as text; only the `<mark>` hits become elements. */
export const Snippet: React.FC<{ text: string }> = ({ text }) => (
  <>
    {snippetParts(text).map((p, i) =>
      p.hit ? (
        <mark key={i} className="bg-amber-400/25 text-amber-100 rounded px-0.5">
          {p.text}
        </mark>
      ) : (
        <React.Fragment key={i}>{p.text}</React.Fragment>
      ),
    )}
  </>
);

const DEBOUNCE_MS = 200;

/** Search every chat; picking a result jumps to that message. */
export const SearchModal: React.FC<{
  isOpen: boolean;
  onClose: () => void;
  kwants: Kwant[];
  onJump: (hit: SearchHit) => void;
}> = ({ isOpen, onClose, kwants, onJump }) => {
  const [q, setQ] = useState('');
  const [hits, setHits] = useState<SearchHit[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const seqRef = useRef(0);

  useEffect(() => {
    if (isOpen) {
      setActive(0);
      requestAnimationFrame(() => inputRef.current?.select());
    }
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    const term = q.trim();
    const seq = ++seqRef.current;
    if (!/[\p{L}\p{N}]/u.test(term)) {
      setHits(null);
      setError(null);
      setLoading(false);
      return;
    }
    setLoading(true);
    const t = setTimeout(() => {
      continuityApi.search(term).then(
        (r) => {
          if (seq !== seqRef.current) return;
          setHits(r);
          setError(null);
          setActive(0);
          setLoading(false);
        },
        (e) => {
          if (seq !== seqRef.current) return;
          setError(e instanceof Error ? e.message : 'Search failed.');
          setLoading(false);
        },
      );
    }, DEBOUNCE_MS);
    return () => clearTimeout(t);
  }, [q, isOpen]);

  const groups = useMemo(() => groupHits(hits ?? [], (id) => kwants.find((k) => k.id === id)?.name), [hits, kwants]);
  const flat = useMemo(() => groups.flatMap((g) => g.hits), [groups]);

  useEffect(() => {
    listRef.current?.querySelector(`[data-index="${active}"]`)?.scrollIntoView({ block: 'nearest' });
  }, [active]);

  if (!isOpen) return null;

  const pick = (h: SearchHit) => {
    onJump(h);
    onClose();
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      onClose();
    } else if (e.key === 'ArrowDown' && flat.length) {
      e.preventDefault();
      setActive((a) => (a + 1) % flat.length);
    } else if (e.key === 'ArrowUp' && flat.length) {
      e.preventDefault();
      setActive((a) => (a - 1 + flat.length) % flat.length);
    } else if (e.key === 'Enter' && flat[active]) {
      e.preventDefault();
      pick(flat[active]);
    }
  };

  let index = -1;
  return (
    <div className="kw-dialog fixed inset-0 z-50 bg-black/70 backdrop-blur-sm flex items-start justify-center p-2 sm:p-4 sm:pt-[12vh]" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Search chats"
        onKeyDown={onKeyDown}
        className="kw-dialog-panel bg-zinc-900 border border-zinc-800 rounded-2xl max-w-xl w-full max-h-[86vh] flex flex-col text-zinc-100 shadow-2xl overflow-hidden"
      >
        <div className="flex items-center gap-2 px-3 sm:px-4 border-b border-zinc-800">
          {loading ? (
            <Loader2 size={16} className="text-zinc-500 animate-spin motion-reduce:animate-none shrink-0" aria-hidden="true" />
          ) : (
            <Search size={16} className="text-zinc-500 shrink-0" aria-hidden="true" />
          )}
          <input
            ref={inputRef}
            value={q}
            onChange={(e) => setQ(e.target.value)}
            placeholder="Search all chats…"
            role="combobox"
            aria-expanded={flat.length > 0}
            aria-controls="search-results"
            aria-activedescendant={flat[active] ? `search-hit-${flat[active].message_id}` : undefined}
            aria-label="Search all chats"
            className="flex-1 min-w-0 bg-transparent py-3.5 text-sm text-zinc-100 placeholder-zinc-500 focus:outline-none"
          />
          <button type="button" onClick={onClose} aria-label="Close search" className="p-1 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 transition-colors">
            <X size={16} />
          </button>
        </div>

        <div ref={listRef} id="search-results" role="listbox" aria-label="Results" className="flex-1 overflow-y-auto p-2">
          {error && (
            <p className="m-2 text-xs text-rose-300" role="alert">
              {error}
            </p>
          )}
          {!hits && !error && (
            <p className="m-3 text-xs text-zinc-500">Type a word to search every channel and direct chat. ↑↓ to move, Enter to open.</p>
          )}
          {hits && hits.length === 0 && !loading && (
            <p className="m-3 text-xs text-zinc-500" role="status">
              No messages match “{q.trim()}”.
            </p>
          )}
          {groups.map((g) => (
            <div key={g.key} role="group" aria-label={g.label} className="mb-2">
              <p className="flex items-center gap-1.5 px-2 py-1 text-[10px] uppercase tracking-wide font-semibold text-zinc-500">
                {g.channel_id ? <Hash size={11} aria-hidden="true" /> : <MessageSquare size={11} aria-hidden="true" />}
                {g.label}
              </p>
              {g.hits.map((h) => {
                index += 1;
                const i = index;
                const isActive = i === active;
                return (
                  <div
                    key={h.message_id}
                    id={`search-hit-${h.message_id}`}
                    data-index={i}
                    role="option"
                    aria-selected={isActive}
                    onMouseMove={() => setActive(i)}
                    onClick={() => pick(h)}
                    className={`px-2.5 py-2 rounded-lg cursor-pointer ${isActive ? 'bg-sky-500/10 ring-1 ring-sky-500/30' : 'hover:bg-zinc-800/60'}`}
                  >
                    <div className="flex items-center gap-2 text-[11px] text-zinc-400">
                      <span className="font-medium text-zinc-200 truncate">{h.author === 'PlayerHero' ? 'You' : h.author}</span>
                      <span className="ml-auto shrink-0 text-zinc-500">{formatWhen(h.created_at)}</span>
                    </div>
                    <p className="mt-0.5 text-xs text-zinc-300 break-words line-clamp-3">
                      <Snippet text={h.snippet} />
                    </p>
                  </div>
                );
              })}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
};
