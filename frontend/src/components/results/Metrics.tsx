import React from 'react';
import { MetricItem, MetricsBlock, inRange, isNum, isText, optText, parseCommon, uniqueIds } from '../../smartResults';
import type { RendererProps } from './registry';

/** 2-6 key numbers, verbatim from the reply (plan §A.3). */
export function validMetrics(raw: unknown): MetricsBlock | null {
  const b = parseCommon(raw, 'metrics');
  if (!b || !inRange(b.items, 2, 6) || !uniqueIds(b.items)) return null;
  const items: MetricItem[] = [];
  for (const it of b.items as Record<string, unknown>[]) {
    if (!isText(it.label) || !isText(it.value_text) || !isNum(it.value)) return null;
    const unit = optText(it.unit, 40);
    const note = optText(it.note);
    if (unit === undefined || note === undefined) return null;
    items.push({ id: it.id as string, label: it.label, value_text: it.value_text, value: it.value, unit, note });
  }
  return { id: b.id, type: 'metrics', title: b.title, source: b.source, provenance: b.provenance, items };
}

/** Tiles for 2-6 key numbers, verbatim from the reply. Two per row on a phone. */
export const Metrics: React.FC<RendererProps<MetricsBlock>> = ({ block }) => (
  <div data-smart-result="metrics" data-block-id={block.id}>
    <ul role="list" className="my-3.5 grid grid-cols-2 gap-2.5 sm:grid-cols-3">
      {block.items.map((item) => (
        <li key={item.id} className="relative overflow-hidden rounded-xl border border-zinc-700/60 bg-zinc-900/60 px-3.5 py-3 shadow-[var(--kws-shadow)] min-w-0 before:absolute before:inset-x-0 before:top-0 before:h-0.5 before:bg-gradient-to-r before:from-sky-400/80 before:to-sky-400/0">
          <div className="truncate text-xs text-zinc-400" title={item.label}>
            {item.label}
          </div>
          <div className="mt-1 flex items-baseline gap-1 text-xl font-semibold tracking-tight text-zinc-100 [overflow-wrap:anywhere]">
            <span>{item.value_text}</span>
            {item.unit && !item.value_text.includes(item.unit) && <span className="text-xs font-normal text-zinc-400">{item.unit}</span>}
          </div>
          {item.note && <div className="mt-1 text-xs leading-snug text-zinc-400 [overflow-wrap:anywhere]">{item.note}</div>}
        </li>
      ))}
    </ul>
  </div>
);
