import React from 'react';
import { FactItem, FactsBlock, inRange, isText, parseCommon, uniqueIds } from '../../smartResults';
import type { RendererProps } from './registry';

/** 3-12 label/value facts, built from parseCommon plus isText/inRange/uniqueIds (plan §A.3). */
export function validFacts(raw: unknown): FactsBlock | null {
  const b = parseCommon(raw, 'facts');
  if (!b || !inRange(b.items, 3, 12) || !uniqueIds(b.items)) return null;
  const items: FactItem[] = [];
  for (const it of b.items as Record<string, unknown>[]) {
    if (!isText(it.label) || !isText(it.value, 300, true)) return null;
    items.push({ id: it.id as string, label: it.label, value: it.value });
  }
  return { id: b.id, type: 'facts', title: b.title, source: b.source, provenance: b.provenance, items };
}

/** A definition-list card for 3-12 label/value facts, mostly non-numeric. */
export const Facts: React.FC<RendererProps<FactsBlock>> = ({ block }) => (
  <div data-smart-result="facts" data-block-id={block.id}>
    <dl className="my-3.5 divide-y divide-zinc-800/60 rounded-xl border border-zinc-800 bg-zinc-950/70 px-3.5 shadow-sm">
      {block.items.map((item) => (
        <div key={item.id} className="grid grid-cols-[minmax(0,2fr)_minmax(0,3fr)] gap-x-3 gap-y-0.5 py-2.5 text-xs">
          <dt className="text-zinc-500 [overflow-wrap:anywhere]">{item.label}</dt>
          <dd className="text-zinc-200 leading-relaxed [overflow-wrap:anywhere]">{item.value || '—'}</dd>
        </div>
      ))}
    </dl>
  </div>
);
