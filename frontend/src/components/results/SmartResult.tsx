import React, { useEffect, useId, useRef, useState } from 'react';
import { Check, ChevronDown, Copy } from 'lucide-react';
import type { SmartBlock } from '../../smartResults';
import { readDoneSteps } from '../../smartResults';
import { playSfx } from '../../audio/chiptune';
import { ReplySource, resolveBlock } from './registry';
import { blockCsv, blockJson, blockMarkdown } from './exporters';
import { SmartActions, actionClass } from './SmartActions';

/** "Ask about this": ChatView puts `text` in the composer and focuses it. Nothing is sent. */
export const COMPOSE_PREFILL_EVENT = 'kwnt:compose-prefill';
export const askAbout = (text: string) => window.dispatchEvent(new CustomEvent(COMPOSE_PREFILL_EVENT, { detail: { text } }));

const textKey = (messageId: string) => `kwnt:smart-text:${messageId}`;
const readAsText = (messageId: string) => {
  try {
    return localStorage.getItem(textKey(messageId)) === '1';
  } catch {
    return false;
  }
};
const saveAsText = (messageId: string, on: boolean) => {
  try {
    if (on) localStorage.setItem(textKey(messageId), '1');
    else localStorage.removeItem(textKey(messageId));
  } catch {
    // Private mode: the choice lasts until the page reloads.
  }
};

const TYPE_LABEL: Record<SmartBlock['type'], string> = {
  comparison: 'Comparison',
  chart: 'Chart',
  metrics: 'Key numbers',
  facts: 'Facts',
  steps: 'Steps',
  cards: 'Links',
  findings: 'Findings',
  verification: 'Checks',
  files: 'Files',
};

/** Where the block's data came from, in plain words (plan §A.6). */
export function evidenceText(block: SmartBlock): string {
  const p = block.provenance;
  if (p.kind !== 'kwant_data') return 'From this reply';
  if (p.ref.startsWith('task:')) return 'From task checks';
  const id = p.ref.slice('attachment:'.length);
  const file = block.type === 'files' ? block.items.find((f) => f.attachment_id === id) : undefined;
  return file ? `From attachment ${file.name}` : block.type === 'files' && block.items.length > 1 ? 'From attachments' : 'From an attachment';
}

/** A block that throws while rendering shows the reply's markdown instead. */
export class SmartResultBoundary extends React.Component<
  { fallback: React.ReactNode; children: React.ReactNode },
  { failed: boolean }
> {
  state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  componentDidCatch(error: unknown) {
    console.warn('A smart result failed to render, so the reply shows as text.', error);
  }

  render() {
    return this.state.failed ? this.props.fallback : this.props.children;
  }
}

type Format = 'Markdown' | 'CSV' | 'JSON';
const FORMATS: Format[] = ['Markdown', 'CSV', 'JSON'];

/** Copy data: one button, a three-item menu. Arrow keys move, Escape closes and returns focus. */
const CopyData: React.FC<{ block: SmartBlock; messageId: string }> = ({ block, messageId }) => {
  const [open, setOpen] = useState(false);
  const [said, setSaid] = useState<{ ok: boolean; text: string } | null>(null);
  const menuId = useId();
  const wrap = useRef<HTMLSpanElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const items = useRef<(HTMLButtonElement | null)[]>([]);
  const timer = useRef<number>();

  useEffect(() => {
    if (!open) return;
    items.current[0]?.focus();
    const away = (e: PointerEvent) => {
      if (!wrap.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('pointerdown', away);
    return () => document.removeEventListener('pointerdown', away);
  }, [open]);
  useEffect(() => () => window.clearTimeout(timer.current), []);

  const copy = async (f: Format) => {
    setOpen(false);
    button.current?.focus();
    const text =
      f === 'CSV' ? blockCsv(block) : f === 'JSON' ? blockJson(block) : blockMarkdown(block, block.type === 'steps' && block.variant === 'checklist' ? readDoneSteps(messageId, block) : undefined);
    try {
      await navigator.clipboard.writeText(text);
      setSaid({ ok: true, text: `Copied as ${f}` });
      playSfx('click');
    } catch {
      setSaid({ ok: false, text: 'Could not copy' });
    }
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setSaid(null), 2000);
  };

  const onMenuKey = (e: React.KeyboardEvent) => {
    const i = items.current.findIndex((el) => el === document.activeElement);
    const move = (to: number) => {
      e.preventDefault();
      items.current[(to + FORMATS.length) % FORMATS.length]?.focus();
    };
    if (e.key === 'ArrowDown') move(i + 1);
    else if (e.key === 'ArrowUp') move(i - 1);
    else if (e.key === 'Home') move(0);
    else if (e.key === 'End') move(FORMATS.length - 1);
    else if (e.key === 'Escape' || e.key === 'Tab') {
      if (e.key === 'Escape') e.preventDefault();
      setOpen(false);
      if (e.key === 'Escape') button.current?.focus();
    }
  };

  return (
    <span ref={wrap} className="relative">
      <button
        ref={button}
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        onClick={() => setOpen((o) => !o)}
        onKeyDown={(e) => {
          if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
            e.preventDefault();
            setOpen(true);
          }
        }}
        className={actionClass}
      >
        {said?.ok ? <Check size={12} className="text-emerald-400" /> : <Copy size={12} />}
        <span className={said?.ok ? 'font-medium text-emerald-400' : said ? 'text-rose-300' : ''}>{said?.ok ? 'Copied' : said ? 'Copy failed' : 'Copy data'}</span>
        <ChevronDown size={11} aria-hidden="true" />
      </button>
      <span className="sr-only" aria-live="polite">
        {said?.text ?? ''}
      </span>
      {open && (
        <span
          id={menuId}
          role="menu"
          aria-label="Copy data as"
          onKeyDown={onMenuKey}
          className="absolute bottom-full left-0 z-20 mb-1 flex min-w-[8rem] flex-col rounded-lg border border-zinc-800 bg-zinc-900 p-1 shadow-lg"
        >
          {FORMATS.map((f, i) => (
            <button
              key={f}
              ref={(el) => (items.current[i] = el)}
              type="button"
              role="menuitem"
              tabIndex={-1}
              onClick={() => copy(f)}
              className="rounded-md px-2.5 py-1.5 text-left text-[11px] text-zinc-300 hover:bg-zinc-800 hover:text-zinc-100 focus:bg-zinc-800 focus:text-zinc-100 focus:outline-none"
            >
              {f}
            </button>
          ))}
        </span>
      )}
    </span>
  );
};

/** The footer every block shares, in the message's action row: Evidence, Show as text, Copy data. */
const Footer: React.FC<{ block: SmartBlock; messageId: string; asText: boolean; onToggle: () => void }> = ({ block, messageId, asText, onToggle }) => (
  <SmartActions messageId={messageId}>
    <span className="px-1 text-[11px] text-zinc-500" data-evidence>
      {evidenceText(block)}
    </span>
    <button type="button" aria-pressed={asText} onClick={onToggle} className={`${actionClass} ${asText ? 'bg-zinc-900 text-zinc-300' : ''}`}>
      Show as text
    </button>
    <CopyData block={block} messageId={messageId} />
  </SmartActions>
);

export interface SmartResultProps {
  /** The message's block for this place, unchecked. */
  raw: unknown;
  /** The reply's own table or list here; null for the block drawn after the reply. */
  reply: ReplySource | null;
  messageId: string;
  /** What the reply shows without a block: its markdown here, or (for a slot block) nothing. */
  fallback: React.ReactNode;
  /** Draws markdown, for "Show as text" on a block that has no text of its own in the reply. */
  renderText?: (markdown: string) => React.ReactNode;
}

/**
 * One block in its frame. A block that replaces the reply's table or list keeps that spot's exact size
 * (its title is the group's accessible name, not a heading); a block after the reply shows its title.
 */
export const SmartResult: React.FC<SmartResultProps> = ({ raw, reply, messageId, fallback, renderText }) => {
  const resolved = resolveBlock(raw, reply);
  const [asText, setAsText] = useState(() => readAsText(messageId));
  if (!resolved) return <>{fallback}</>;
  const { block, renderer } = resolved;
  const { Component } = renderer;
  const slot = reply === null;
  const text = slot ? renderText?.(blockMarkdown(block)) ?? null : fallback;
  const toggle = () => {
    saveAsText(messageId, !asText);
    setAsText(!asText);
  };
  const name = block.title ?? TYPE_LABEL[block.type];

  return (
    <div role="group" aria-label={name} data-smart-frame={block.type} className={slot ? 'mt-3' : undefined}>
      {slot && block.title && <div className="mb-1.5 text-xs font-semibold text-zinc-300">{block.title}</div>}
      {asText ? (
        text
      ) : (
        <SmartResultBoundary fallback={text}>
          <Component block={block} messageId={messageId} reply={reply} ask={askAbout} />
        </SmartResultBoundary>
      )}
      <Footer block={block} messageId={messageId} asText={asText} onToggle={toggle} />
    </div>
  );
};
