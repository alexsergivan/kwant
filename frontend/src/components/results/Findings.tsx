import React from 'react';
import { FindingItem, FindingRef, FindingsBlock, Severity, inRange, isIndex, isObj, isText, parseCommon, uniqueIds } from '../../smartResults';
import type { RendererProps } from './registry';

const SEVERITIES: Severity[] = ['critical', 'high', 'medium', 'low', 'info'];
const ORDER: Severity[] = ['critical', 'high', 'medium', 'low', 'info'];

/** 2-30 findings; severity is one of the fixed enum values, refs name a path and an optional line. */
export function validFindings(raw: unknown): FindingsBlock | null {
  const b = parseCommon(raw, 'findings');
  if (!b || !inRange(b.items, 2, 30) || !uniqueIds(b.items)) return null;
  const items: FindingItem[] = [];
  for (const it of b.items as Record<string, unknown>[]) {
    if (!SEVERITIES.includes(it.severity as Severity) || !isText(it.severity_text, 40)) return null;
    // The server sends detail "" for a bullet with no " — detail" part (its own check allows that), so blank is valid here; title stays required.
    if (!isText(it.title) || !(it.detail === undefined || it.detail === null || isText(it.detail, 2000, true))) return null;
    if (!Array.isArray(it.refs)) return null;
    const refs: FindingRef[] = [];
    for (const r of it.refs) {
      if (!isObj(r) || !isText(r.path, 400)) return null;
      if (r.line !== null && r.line !== undefined && !isIndex(r.line)) return null;
      refs.push({ path: r.path, line: r.line === undefined ? null : (r.line as number | null) });
    }
    items.push({ id: it.id as string, severity: it.severity as Severity, severity_text: it.severity_text, title: it.title, detail: typeof it.detail === 'string' ? it.detail : '', refs });
  }
  return { id: b.id, type: 'findings', title: b.title, source: b.source, provenance: b.provenance, items };
}

// Severity is always paired with its word, never colour alone; the dot is decorative.
const DOT: Record<Severity, string> = {
  critical: 'bg-rose-500',
  high: 'bg-orange-500',
  medium: 'bg-amber-500',
  low: 'bg-sky-500',
  info: 'bg-zinc-500',
};

const refLabel = (r: FindingItem['refs'][number]) => `${r.path}${r.line !== null ? `:${r.line}` : ''}`;

/** Findings grouped by severity, with counts; severity is text, refs are code. */
export const Findings: React.FC<RendererProps<FindingsBlock>> = ({ block }) => {
  const groups = ORDER.map((sev) => ({ sev, items: block.items.filter((i) => i.severity === sev) })).filter((g) => g.items.length > 0);

  return (
    <div data-smart-result="findings" data-block-id={block.id} className="my-3.5 space-y-3">
      <div className="flex flex-wrap gap-x-3 gap-y-1 text-[11px] text-zinc-500">
        {groups.map((g) => (
          <span key={g.sev} className="flex items-center gap-1.5">
            <span className={`h-1.5 w-1.5 rounded-full ${DOT[g.sev]}`} aria-hidden="true" />
            {block.items.find((i) => i.severity === g.sev)?.severity_text ?? g.sev}: {g.items.length}
          </span>
        ))}
      </div>
      {groups.map((g) => (
        <div key={g.sev}>
          <h4 className="mb-1.5 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wide text-zinc-500">
            <span className={`h-1.5 w-1.5 rounded-full ${DOT[g.sev]}`} aria-hidden="true" />
            {g.items[0].severity_text} ({g.items.length})
          </h4>
          <ul role="list" className="space-y-1.5">
            {g.items.map((item) => (
              <li key={item.id} className="rounded-xl border border-zinc-800 bg-zinc-950/70 p-3 text-xs shadow-sm">
                <div className="font-semibold leading-snug text-zinc-100 [overflow-wrap:anywhere]">{item.title}</div>
                {item.detail.trim() !== '' && <p className="mt-1 leading-relaxed text-zinc-400 [overflow-wrap:anywhere]">{item.detail}</p>}
                {item.refs.length > 0 && (
                  <div className="mt-1.5 flex flex-wrap gap-1.5">
                    {item.refs.map((r, i) => (
                      <code key={i} className="rounded bg-zinc-900 px-1.5 py-0.5 font-mono text-[11px] text-sky-300">
                        {refLabel(r)}
                      </code>
                    ))}
                  </div>
                )}
              </li>
            ))}
          </ul>
        </div>
      ))}
    </div>
  );
};
