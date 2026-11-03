import React, { useId, useMemo, useState } from 'react';
import { ArrowDown, ArrowUp, ChevronsUpDown, MessageSquare, Minus, Plus } from 'lucide-react';
import { ColumnRole, ComparisonBlock, Sort, nextSort, plainText, sortRows } from '../../smartResults';
import type { RendererProps } from './registry';

// Pros and cons cells: split on ";", "•", "·" or a line break, else on commas when every part is short.
export function shortList(text: string): string[] | null {
  const t = plainText(text);
  let parts = t.split(/\s*(?:;|•|·|\n)\s*/).filter(Boolean);
  if (parts.length < 2) {
    const commas = t.split(/\s*,\s*/).filter(Boolean);
    parts = commas.length >= 2 && commas.every((p) => p.split(/\s+/).length <= 5) ? commas : [];
  }
  return parts.length >= 2 ? parts : null;
}

const ROLE_TINT: Partial<Record<ColumnRole, string>> = { pros: 'text-emerald-300', cons: 'text-rose-300' };

const PickLabel: React.FC<{ reason: string | null; inline?: boolean; className?: string }> = ({ reason, inline, className = '' }) => (
  <span
    className={`${className} ${inline ? 'ml-1.5 ' : ''}rounded border border-sky-500/30 bg-sky-500/10 px-1 py-px align-[1px] text-[10px] font-medium leading-none text-sky-300 [box-decoration-break:clone]`}
    title={reason ?? undefined}
  >
    Pick in this reply
  </span>
);

/** A pros or cons cell in a stacked card: a short list with + or −, else the reply's cell. */
const Cell: React.FC<{ role: ColumnRole; text: string; node: React.ReactNode }> = ({ role, text, node }) => {
  const parts = role === 'pros' || role === 'cons' ? shortList(text) : null;
  if (!parts) return <>{node}</>;
  const Mark = role === 'pros' ? Plus : Minus;
  return (
    <ul role="list" className="space-y-0.5">
      {parts.map((p, i) => (
        <li key={i} className="flex items-start gap-1">
          <Mark size={11} aria-hidden="true" className={`mt-[3px] shrink-0 ${ROLE_TINT[role]}`} />
          <span>{p}</span>
        </li>
      ))}
    </ul>
  );
};

/**
 * A bot reply's table, sortable. On wide screens it keeps the markdown table's exact geometry, so the
 * swap from the streamed table moves nothing; below 640 px it becomes one card per row.
 */
export const Comparison: React.FC<RendererProps<ComparisonBlock>> = ({ block, reply, ask }) => {
  const table = reply?.kind === 'table' ? reply : null;
  // A slot block (declared by the bot, no table in the reply) draws its cells as plain text.
  const head = block.columns.map((c, j) => table?.head[j]?.node ?? plainText(c.label));
  const body = block.rows.map((r, i) => r.cells.map((c, j) => table?.body[i]?.[j]?.node ?? plainText(c)));
  const [sort, setSort] = useState<Sort | null>(null);
  const [said, setSaid] = useState('');
  const uid = useId();
  const order = useMemo(() => {
    const pos = new Map(block.rows.map((r, i) => [r.id, i]));
    return sortRows(block, sort).map((r) => pos.get(r.id) as number);
  }, [block, sort]);
  const labels = block.columns.map((c) => plainText(c.label));
  const askRow = (i: number) => ask(`About ${plainText(block.rows[i].cells[0])} in your ${labels[0]} table: `);

  const applySort = (next: Sort | null) => {
    setSort(next);
    setSaid(next ? `Sorted by ${labels[next.col]}, ${next.dir}` : "Back in the reply's order");
  };

  return (
    <div data-smart-result="comparison" data-block-id={block.id}>
      <span className="sr-only" aria-live="polite">
        {said}
      </span>

      {/* 640 px and up: the reply's table with header sort buttons. */}
      <div className="my-3.5 hidden sm:block">
        <div className="overflow-x-auto rounded-xl border border-zinc-700/60 bg-zinc-900/60 shadow-[var(--kws-shadow)]">
          <table className="w-full text-left border-collapse text-xs font-sans">
            <thead className="bg-zinc-800/70 text-zinc-100 border-b border-zinc-700/60 font-medium">
              <tr>
                {block.columns.map((col, j) => {
                  const dir = sort?.col === j ? sort.dir : null;
                  const Icon = dir === 'ascending' ? ArrowUp : dir === 'descending' ? ArrowDown : ChevronsUpDown;
                  return (
                    <th
                      key={col.key}
                      scope="col"
                      aria-sort={dir ?? 'none'}
                      className="relative px-3.5 py-2.5 text-xs font-semibold text-zinc-200 whitespace-nowrap"
                    >
                      <button
                        type="button"
                        onClick={() => applySort(nextSort(sort, j))}
                        className={`group font-semibold ${ROLE_TINT[col.role] ?? 'text-zinc-200'} hover:text-zinc-50 focus-visible:outline-none after:absolute after:inset-0 after:rounded-sm after:content-[''] focus-visible:after:ring-2 focus-visible:after:ring-inset focus-visible:after:ring-sky-500/70`}
                      >
                        {head[j]}
                        {/* Zero width: the icon sits in the cell's padding, so headers don't widen. */}
                        <span className="inline-block w-0 align-[-1px]" aria-hidden="true">
                          <Icon
                            size={10}
                            className={`ml-1 ${dir ? 'text-sky-400' : 'text-zinc-400 group-hover:text-zinc-200 group-focus-visible:text-zinc-400'}`}
                          />
                        </span>
                      </button>
                    </th>
                  );
                })}
              </tr>
            </thead>
            <tbody className="divide-y divide-zinc-700/40 font-normal text-zinc-300">
              {order.map((i) => {
                const row = block.rows[i];
                const picked = row.id === block.highlight_row;
                return (
                  <tr
                    key={row.id}
                    data-pick={picked ? '' : undefined}
                    className={`group/row transition-colors ${picked ? 'bg-sky-500/[0.07] hover:bg-sky-500/10' : 'hover:bg-zinc-800/40'}`}
                  >
                    {block.columns.map((col, j) => (
                      <td
                        key={col.key}
                        className={`px-3.5 py-2.5 text-xs text-zinc-300 leading-relaxed align-top ${picked && j === 0 ? 'shadow-[inset_2px_0_0_0_theme(colors.sky.400)]' : ''} ${j === block.columns.length - 1 ? 'relative' : ''}`}
                      >
                        {j === block.columns.length - 1 && (
                          // Over the cell's corner, shown on row hover or focus: the table keeps its size.
                          <button
                            type="button"
                            onClick={() => askRow(i)}
                            aria-label={`Ask about ${plainText(row.cells[0])}`}
                            title="Ask about this (fills in the message box, sends nothing)"
                            className="absolute right-1.5 top-1.5 flex h-6 w-6 items-center justify-center rounded-md border border-zinc-700 bg-zinc-900 text-zinc-400 opacity-0 hover:text-zinc-100 focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-500/70 group-hover/row:opacity-100 [@media(hover:none)]:hidden"
                          >
                            <MessageSquare size={12} />
                          </button>
                        )}
                        {picked && j === 0 ? (
                          // The label only takes room left on the cell's first line, so the table keeps the
                          // streamed reply's exact size (see .smart-pick in modern.css).
                          <div className="flex items-baseline">
                            <span>{body[i][j]}</span>
                            <span className="smart-pick min-w-0 flex-1">
                              <PickLabel reason={block.highlight_reason} inline className="smart-pick-full" />
                              <span
                                aria-hidden="true"
                                title={block.highlight_reason ? `Pick in this reply: ${block.highlight_reason}` : 'Pick in this reply'}
                                className="smart-pick-short ml-1.5 rounded border border-sky-500/30 bg-sky-500/10 px-1 py-px align-[1px] text-[10px] font-medium leading-none text-sky-300">
                                Pick
                              </span>
                            </span>
                          </div>
                        ) : (
                          body[i][j]
                        )}
                      </td>
                    ))}
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      </div>

      {/* Below 640 px: one card per row, label: value pairs, nothing clipped. */}
      <div className="my-3.5 space-y-2 sm:hidden">
        <div className="flex items-center gap-2">
          <label htmlFor={`${uid}-sort`} className="shrink-0 text-[11px] text-zinc-400">
            Sort by
          </label>
          <select
            id={`${uid}-sort`}
            value={sort ? `${sort.col}:${sort.dir}` : ''}
            onChange={(e) => {
              const [c, d] = e.target.value.split(':');
              applySort(e.target.value ? { col: Number(c), dir: d as Sort['dir'] } : null);
            }}
            className="h-8 min-w-0 flex-1 truncate rounded-lg border border-zinc-800 bg-zinc-900 px-2 text-xs text-zinc-200 focus:outline-none focus-visible:ring-2 focus-visible:ring-sky-500/70"
          >
            <option value="">Reply order</option>
            {block.columns.map((col, j) => (
              <React.Fragment key={col.key}>
                <option value={`${j}:ascending`}>
                  {labels[j]}, {col.numeric ? 'low to high' : 'A to Z'}
                </option>
                <option value={`${j}:descending`}>
                  {labels[j]}, {col.numeric ? 'high to low' : 'Z to A'}
                </option>
              </React.Fragment>
            ))}
          </select>
        </div>
        <ul role="list" aria-label={labels[0]} className="space-y-2">
          {order.map((i) => {
            const row = block.rows[i];
            const picked = row.id === block.highlight_row;
            return (
              <li
                key={row.id}
                data-pick={picked ? '' : undefined}
                className={`rounded-xl border px-3 py-2.5 text-xs ${picked ? 'border-sky-500/40 bg-sky-500/[0.07]' : 'border-zinc-800 bg-zinc-900/60'}`}
              >
                <div className="text-[13px] font-semibold leading-snug text-zinc-100 [overflow-wrap:anywhere]">
                  {body[i][0]}
                  {picked && <PickLabel reason={block.highlight_reason} inline />}
                </div>
                <dl className="mt-1.5 grid grid-cols-[minmax(0,2fr)_minmax(0,3fr)] gap-x-3 gap-y-1">
                  {block.columns.slice(1).map((col, k) => (
                    <React.Fragment key={col.key}>
                      <dt className="text-zinc-400 [overflow-wrap:anywhere]">{head[k + 1]}</dt>
                      <dd className="text-zinc-300 leading-relaxed [overflow-wrap:anywhere]">
                        <Cell role={col.role} text={row.cells[k + 1]} node={body[i][k + 1]} />
                      </dd>
                    </React.Fragment>
                  ))}
                </dl>
                <button
                  type="button"
                  onClick={() => askRow(i)}
                  className="-mx-1 mt-1.5 flex items-center gap-1.5 rounded-md px-1 py-1 text-[11px] text-zinc-400 hover:text-zinc-300 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-500/70"
                >
                  <MessageSquare size={12} aria-hidden="true" />
                  Ask about this
                </button>
              </li>
            );
          })}
        </ul>
      </div>

    </div>
  );
};
