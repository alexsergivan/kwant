import React from 'react';
import { ExternalLink } from 'lucide-react';
import { CardItem, CardsBlock, inRange, isHttpUrl, isText, parseCommon, uniqueIds } from '../../smartResults';
import type { RendererProps } from './registry';

/** 3-12 link cards; a url must be http(s) or absent (plan §A.3). */
export function validCards(raw: unknown): CardsBlock | null {
  const b = parseCommon(raw, 'cards');
  if (!b || !inRange(b.items, 3, 12) || !uniqueIds(b.items)) return null;
  const items: CardItem[] = [];
  for (const it of b.items as Record<string, unknown>[]) {
    if (!isText(it.title) || !isText(it.description, 300, true)) return null;
    const url = it.url === null || it.url === undefined ? null : isHttpUrl(it.url) ? it.url : undefined;
    if (url === undefined) return null;
    if (!Array.isArray(it.meta) || !it.meta.every((m) => isText(m, 40))) return null;
    items.push({ id: it.id as string, title: it.title, url, description: it.description, meta: it.meta as string[] });
  }
  return { id: b.id, type: 'cards', title: b.title, source: b.source, provenance: b.provenance, items };
}

/** A link's domain, without a leading "www.", or the raw string if it doesn't parse. */
function urlDomain(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, '');
  } catch {
    return url;
  }
}

/** Compact cards: title, description, and the link's domain. Links open in a new tab, http(s) only. */
export const Cards: React.FC<RendererProps<CardsBlock>> = ({ block }) => (
  <div data-smart-result="cards" data-block-id={block.id}>
    <ul role="list" className="my-3.5 grid grid-cols-1 gap-2.5 sm:grid-cols-2">
      {block.items.map((item) => {
        const Wrapper = item.url ? 'a' : 'div';
        const wrapperProps = item.url ? { href: item.url, target: '_blank', rel: 'noopener noreferrer' } : {};
        return (
          <li key={item.id} className="min-w-0">
            <Wrapper
              {...wrapperProps}
              className="block h-full rounded-xl border border-zinc-800 bg-zinc-950/70 p-3 shadow-sm transition-colors motion-reduce:transition-none hover:border-zinc-700 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-500/70"
            >
              <div className="flex items-start justify-between gap-2">
                <h4 className="text-[13px] font-semibold leading-snug text-zinc-100 [overflow-wrap:anywhere]">{item.title}</h4>
                {item.url && <ExternalLink size={12} className="mt-0.5 shrink-0 text-zinc-600" aria-hidden="true" />}
              </div>
              <p className="mt-1 text-xs leading-relaxed text-zinc-400 [overflow-wrap:anywhere]">{item.description}</p>
              <div className="mt-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px] text-zinc-500">
                {item.url && <span className="truncate">{urlDomain(item.url)}</span>}
                {item.meta.map((m, i) => (
                  <span key={i} className="rounded border border-zinc-800 px-1.5 py-0.5">
                    {m}
                  </span>
                ))}
              </div>
            </Wrapper>
          </li>
        );
      })}
    </ul>
  </div>
);
