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
        <li key={item.id} className="rounded-xl border border-zinc-800 bg-zinc-950/70 px-3 py-2.5 shadow-sm min-w-0">
          <div className="truncate text-[11px] text-zinc-500" title={item.label}>
            {item.label}
          </div>
          <div className="mt-0.5 flex items-baseline gap-1 text-lg font-semibold text-zinc-100 [overflow-wrap:anywhere]">
            <span>{item.value_text}</span>
            {item.unit && <span className="text-xs font-normal text-zinc-500">{item.unit}</span>}
          </div>
          {item.note && <div className="mt-0.5 text-[11px] leading-snug text-zinc-500 [overflow-wrap:anywhere]">{item.note}</div>}
        </li>
      ))}
    </ul>
  </div>
);
