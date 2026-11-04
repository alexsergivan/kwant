import React, { useEffect, useId, useMemo, useRef, useState } from 'react';
import { ChartBlock, ChartPoint, ChartSeries, inRange, isCell, isNum, isObj, isText, optText, parseCommon, repeatsReply, uniqueIds } from '../../smartResults';
import type { RendererProps } from './registry';
import { ReplyText } from './ReplyText';

const MAX_VALUE = 1e15;

/** 1-4 series (unique by key) and 3-60 points, each with one cell/value per series (plan §A.3). */
export function validChart(raw: unknown): ChartBlock | null {
  const b = parseCommon(raw, 'chart');
  if (!b) return null;
  if (b.chart_kind !== 'line' && b.chart_kind !== 'bar') return null;
  const x = b.x;
  if (!isObj(x) || !isCell(x.label) || (x.kind !== 'time' && x.kind !== 'category')) return null;
  if (!inRange(b.series, 1, 4)) return null;
  const series: ChartSeries[] = [];
  const keys = new Set<string>();
  for (const s of b.series as Record<string, unknown>[]) {
    if (!isObj(s) || !isText(s.key, 60) || keys.has(s.key) || !isCell(s.label)) return null;
    const unit = optText(s.unit, 40);
    if (unit === undefined) return null;
    keys.add(s.key);
    series.push({ key: s.key, label: s.label, unit });
  }
  // One shared Y axis means one unit: a chart whose series differ (EUR next to "rating (1-5)") would flatten
  // the smaller one, so it abstains and the reply's own table shows instead (plan §A.3).
  if (series.some((s) => s.unit !== series[0].unit)) return null;
  if (!inRange(b.points, 3, 60) || !uniqueIds(b.points)) return null;
  // A bar chart needs 5 or more rows (plan §A.3); a smaller table stays a comparison.
  if (b.chart_kind === 'bar' && (b.points as unknown[]).length < 5) return null;
  const points: ChartPoint[] = [];
  for (const p of b.points as Record<string, unknown>[]) {
    if (!isCell(p.x)) return null;
    if (!Array.isArray(p.cells) || p.cells.length !== series.length || !p.cells.every((c) => isCell(c, true))) return null;
    // Finite and within what an SVG coordinate can carry: 1e308 turns every scaled y into NaN.
    if (!Array.isArray(p.values) || p.values.length !== series.length || !p.values.every((v) => v === null || (isNum(v) && Math.abs(v) <= MAX_VALUE))) return null;
    points.push({ id: p.id as string, x: p.x, cells: p.cells as string[], values: p.values as (number | null)[] });
  }
  const summary = optText(b.summary);
  if (summary === undefined) return null;
  return {
    id: b.id,
    type: 'chart',
    title: b.title,
    source: b.source,
    provenance: b.provenance,
    chart_kind: b.chart_kind,
    x: { label: x.label, kind: x.kind },
    series,
    points,
    summary,
  };
}

const HEIGHT = 260;
const PAD = { top: 16, right: 16, bottom: 36, left: 44 };
const PLOT_H = HEIGHT - PAD.top - PAD.bottom;

// Up to 4 series: told apart by shape/pattern as well as colour, never colour alone.
const MARKERS: React.FC<{ cx: number; cy: number; size: number }>[] = [
  ({ cx, cy, size }) => <circle cx={cx} cy={cy} r={size} />,
  ({ cx, cy, size }) => <rect x={cx - size} y={cy - size} width={size * 2} height={size * 2} />,
  ({ cx, cy, size }) => (
    <polygon points={`${cx},${cy - size * 1.2} ${cx + size * 1.1},${cy + size} ${cx - size * 1.1},${cy + size}`} />
  ),
  ({ cx, cy, size }) => (
    <polygon points={`${cx},${cy - size * 1.2} ${cx + size * 1.2},${cy} ${cx},${cy + size * 1.2} ${cx - size * 1.2},${cy}`} />
  ),
];
const DASHES = [undefined, '6 3', '2 3', '8 2 2 2'];
const SERIES_COLORS = ['#8270f5', '#38bdf8', '#34d399', '#fb923c'];

function niceTicks(min: number, max: number, count: number): number[] {
  if (min === max) {
    min -= 1;
    max += 1;
  }
  const span = max - min;
  const step0 = span / count;
  const mag = 10 ** Math.floor(Math.log10(step0));
  const residual = step0 / mag;
  const step = (residual > 5 ? 10 : residual > 2 ? 5 : residual > 1 ? 2 : 1) * mag;
  const start = Math.floor(min / step) * step;
  const ticks: number[] = [];
  for (let v = start; v <= max + step / 2; v += step) ticks.push(Math.round(v * 1e6) / 1e6);
  return ticks;
}

function fmtTick(n: number): string {
  return new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 }).format(n);
}

const BarPattern: React.FC<{ id: string; variant: number; color: string }> = ({ id, variant, color }) => {
  if (variant === 0) return <pattern id={id} />; // solid fill, no pattern needed
  if (variant === 1)
    return (
      <pattern id={id} width="6" height="6" patternTransform="rotate(45)" patternUnits="userSpaceOnUse">
        <rect width="6" height="6" fill={color} fillOpacity={0.35} />
        <line x1="0" y1="0" x2="0" y2="6" stroke={color} strokeWidth="3" />
      </pattern>
    );
  if (variant === 2)
    return (
      <pattern id={id} width="5" height="5" patternUnits="userSpaceOnUse">
        <rect width="5" height="5" fill={color} fillOpacity={0.35} />
        <circle cx="2.5" cy="2.5" r="1.1" fill={color} />
      </pattern>
    );
  return (
    <pattern id={id} width="6" height="6" patternUnits="userSpaceOnUse">
      <rect width="6" height="6" fill={color} fillOpacity={0.35} />
      <path d="M0 0L6 6M6 0L0 6" stroke={color} strokeWidth="1.2" />
    </pattern>
  );
};

/**
 * A reply's table rendered as a hand-rolled SVG chart: line for a time axis, grouped bars for
 * categories. Up to 4 series, told apart by marker shape or fill pattern, never colour alone. Every
 * value comes verbatim from the block; axis gridlines are scale, not claims. A null value is a gap.
 */
export const Chart: React.FC<RendererProps<ChartBlock>> = ({ block }) => {
  const uid = useId();
  const replyText = React.useContext(ReplyText);
  const [view, setView] = useState<'chart' | 'table'>('chart');
  const [active, setActive] = useState<{ seriesIdx: number; pointIdx: number } | null>(null);
  // Drawn at the card's real width (1 SVG unit = 1 px) so axis text stays readable on a 390 px phone.
  const svgBox = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(640);
  useEffect(() => {
    const el = svgBox.current;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const measure = () => setWidth(Math.max(240, Math.round(el.clientWidth)) || 640);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [view]);
  const WIDTH = width;
  const narrow = width < 480;
  const PLOT_W = WIDTH - PAD.left - PAD.right;

  const allValues = block.points.flatMap((p) => p.values).filter((v): v is number => v !== null);
  const dataMin = allValues.length ? Math.min(0, ...allValues) : 0;
  const dataMax = allValues.length ? Math.max(0, ...allValues) : 1;
  const ticks = useMemo(() => niceTicks(dataMin, dataMax, 4), [dataMin, dataMax]);
  const yMin = Math.min(dataMin, ticks[0] ?? dataMin);
  const yMax = Math.max(dataMax, ticks[ticks.length - 1] ?? dataMax);
  const yScale = (v: number) => PAD.top + PLOT_H - ((v - yMin) / (yMax - yMin || 1)) * PLOT_H;

  const n = block.points.length;
  // Bars sit in equal bands so none touches the axis; lines run edge to edge.
  const isBar = block.chart_kind === 'bar';
  const xStep = isBar ? PLOT_W / n : n > 1 ? PLOT_W / (n - 1) : 0;
  const xPos = (i: number) => (isBar ? PAD.left + (i + 0.5) * xStep : PAD.left + (n > 1 ? i * xStep : PLOT_W / 2));

  // Bars: every category is named when its band fits a shortened label (5 bars at 390 px do); lines thin out.
  const MIN_BAND_LABEL = 28;
  const labelEvery = isBar ? Math.max(1, Math.ceil(MIN_BAND_LABEL / xStep)) : Math.max(1, Math.ceil(n / (narrow ? 4 : 10)));
  const labelChars = isBar ? Math.max(3, Math.floor((xStep * labelEvery - 4) / 6)) : 10;

  const activePoint = active ? block.points[active.pointIdx] : null;
  const activeSeries = active ? block.series[active.seriesIdx] : null;
  const activeValue = active ? activePoint?.values[active.seriesIdx] : null;
  const activeCell = active ? activePoint?.cells[active.seriesIdx] : null;

  const headers = [block.x.label, ...block.series.map((s) => (s.unit ? `${s.label} (${s.unit})` : s.label))];

  const groupWidth = isBar ? Math.min(xStep * 0.7, 56) : 0;
  const barWidth = isBar ? groupWidth / block.series.length : 0;

  return (
    <div data-smart-result="chart" data-block-id={block.id} className="my-3.5 rounded-xl border border-zinc-700/60 bg-zinc-900/60 p-3.5 shadow-[var(--kws-shadow)]">
      <div className="mb-2 flex items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-3 text-[11px]">
          {block.series.map((s, i) => (
            <span key={s.key} className="flex items-center gap-1.5 text-zinc-400">
              <svg width="14" height="14" viewBox="-7 -7 14 14" aria-hidden="true" className="shrink-0">
                <g fill={SERIES_COLORS[i % SERIES_COLORS.length]} stroke={block.chart_kind === 'line' ? SERIES_COLORS[i % SERIES_COLORS.length] : 'none'}>
                  {React.createElement(MARKERS[i % MARKERS.length], { cx: 0, cy: 0, size: 4 })}
                </g>
              </svg>
              {s.label}
              {s.unit ? ` (${s.unit})` : ''}
            </span>
          ))}
        </div>
        <div role="group" aria-label="Chart view" className="flex shrink-0 rounded-lg border border-zinc-800 text-[11px]">
          {(['chart', 'table'] as const).map((v, i) => (
            <button
              key={v}
              type="button"
              aria-pressed={view === v}
              onClick={() => setView(v)}
              className={`px-2.5 py-1 capitalize cursor-pointer focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-500/70 ${
                i === 0 ? 'rounded-l-lg' : 'rounded-r-lg'
              } ${view === v ? 'bg-sky-500/15 text-sky-300' : 'text-zinc-400 hover:text-zinc-200'}`}
            >
              {v}
            </button>
          ))}
        </div>
      </div>

      {view === 'chart' ? (
        <>
          <div ref={svgBox}>
          <svg
            viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
            role="img"
            aria-labelledby={`${uid}-title`}
            aria-describedby={`${uid}-desc`}
            className="w-full"
            style={{ maxHeight: 280 }}
          >
            <title id={`${uid}-title`}>{block.title ?? `${block.chart_kind === 'line' ? 'Line' : 'Bar'} chart`}</title>
            <desc id={`${uid}-desc`}>{block.summary ?? headers.join(', ')}</desc>
            <defs>
              {block.series.map((s, i) => (
                <BarPattern key={s.key} id={`${uid}-pat-${i}`} variant={i} color={SERIES_COLORS[i % SERIES_COLORS.length]} />
              ))}
            </defs>

            {/* Gridlines: scale, not data claims. */}
            {ticks.map((t) => (
              <g key={t}>
                <line x1={PAD.left} x2={WIDTH - PAD.right} y1={yScale(t)} y2={yScale(t)} stroke="#27272a" strokeWidth={1} />
                <text x={PAD.left - 8} y={yScale(t)} textAnchor="end" dominantBaseline="middle" fontSize={11} fill="#71717a">
                  {fmtTick(t)}
                </text>
              </g>
            ))}
            <line x1={PAD.left} x2={PAD.left} y1={PAD.top} y2={HEIGHT - PAD.bottom} stroke="#3f3f46" strokeWidth={1} />
            <line x1={PAD.left} x2={WIDTH - PAD.right} y1={HEIGHT - PAD.bottom} y2={HEIGHT - PAD.bottom} stroke="#3f3f46" strokeWidth={1} />

            {/* X labels, thinned out so 390 px never clips. */}
            {block.points.map((p, i) => {
              const gap = (n - 1) % labelEvery;
              if (i % labelEvery !== 0 && !(i === n - 1 && gap * 2 >= labelEvery)) return null;
              return (
                <text
                  key={p.id}
                  x={xPos(i)}
                  y={HEIGHT - PAD.bottom + 16}
                  textAnchor={!isBar && i === n - 1 && n > 1 ? 'end' : 'middle'}
                  fontSize={11}
                  fill="#a1a1aa"
                >
                  {p.x.length > labelChars ? (
                    <>
                      <title>{p.x}</title>
                      {`${p.x.slice(0, labelChars - 1)}…`}
                    </>
                  ) : (
                    p.x
                  )}
                </text>
              );
            })}

            {block.chart_kind === 'line'
              ? block.series.map((s, si) => {
                  const segments: { x: number; y: number; i: number }[][] = [];
                  let cur: { x: number; y: number; i: number }[] = [];
                  block.points.forEach((p, i) => {
                    const v = p.values[si];
                    if (v === null) {
                      if (cur.length) segments.push(cur);
                      cur = [];
                      return;
                    }
                    cur.push({ x: xPos(i), y: yScale(v), i });
                  });
                  if (cur.length) segments.push(cur);
                  const color = SERIES_COLORS[si % SERIES_COLORS.length];
                  const Marker = MARKERS[si % MARKERS.length];
                  return (
                    <g key={s.key}>
                      {segments.map((seg, k) => (
                        <path
                          key={k}
                          d={seg.map((pt, idx) => `${idx === 0 ? 'M' : 'L'}${pt.x},${pt.y}`).join(' ')}
                          fill="none"
                          stroke={color}
                          strokeWidth={2}
                          strokeDasharray={DASHES[si % DASHES.length]}
                        />
                      ))}
                      {block.points.map((p, i) => {
                        const v = p.values[si];
                        if (v === null) return null;
                        const isActive = active?.seriesIdx === si && active.pointIdx === i;
                        return (
                          <g
                            key={p.id}
                            tabIndex={0}
                            role="button"
                            aria-label={`${s.label} at ${p.x}: ${p.cells[si] || fmtTick(v)}${s.unit ? ` ${s.unit}` : ''}`}
                            className="cursor-pointer outline-none focus-visible:[&>*]:stroke-zinc-50"
                            fill={color}
                            stroke={color}
                            onFocus={() => setActive({ seriesIdx: si, pointIdx: i })}
                            onBlur={() => setActive(null)}
                            onMouseEnter={() => setActive({ seriesIdx: si, pointIdx: i })}
                            onMouseLeave={() => setActive(null)}
                          >
                            <circle cx={xPos(i)} cy={yScale(v)} r={10} fill="transparent" stroke="none" />
                            <Marker cx={xPos(i)} cy={yScale(v)} size={isActive ? 4.5 : 3.2} />
                          </g>
                        );
                      })}
                    </g>
                  );
                })
              : block.points.map((p, i) => (
                  <g key={p.id}>
                    {block.series.map((s, si) => {
                      const v = p.values[si];
                      if (v === null) return null;
                      const x = xPos(i) - groupWidth / 2 + si * barWidth;
                      const y = yScale(Math.max(0, v));
                      const h = Math.abs(yScale(v) - yScale(0));
                      const isActive = active?.seriesIdx === si && active.pointIdx === i;
                      return (
                        <rect
                          key={s.key}
                          tabIndex={0}
                          role="button"
                          aria-label={`${s.label} at ${p.x}: ${p.cells[si] || fmtTick(v)}${s.unit ? ` ${s.unit}` : ''}`}
                          x={x}
                          y={y}
                          width={Math.max(2, barWidth - 2)}
                          height={Math.max(0, h)}
                          fill={si === 0 ? SERIES_COLORS[0] : `url(#${uid}-pat-${si})`}
                          stroke={SERIES_COLORS[si % SERIES_COLORS.length]}
                          strokeWidth={isActive ? 1.5 : 0}
                          className="cursor-pointer outline-none"
                          onFocus={() => setActive({ seriesIdx: si, pointIdx: i })}
                          onBlur={() => setActive(null)}
                          onMouseEnter={() => setActive({ seriesIdx: si, pointIdx: i })}
                          onMouseLeave={() => setActive(null)}
                        />
                      );
                    })}
                  </g>
                ))}
          </svg>
          </div>
          <div aria-live="polite" className="mt-1 h-4 text-[11px] text-zinc-400">
            {active && activeSeries
              ? `${activeSeries.label} at ${activePoint?.x}: ${activeCell || (activeValue !== null && activeValue !== undefined ? fmtTick(activeValue) : '—')}${
                  activeSeries.unit ? ` ${activeSeries.unit}` : ''
                }`
              : 'Tab through the chart for each value.'}
          </div>
        </>
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full border-collapse text-left text-xs">
            <thead className="border-b border-zinc-800 text-zinc-200">
              <tr>
                {headers.map((h) => (
                  <th key={h} scope="col" className="px-2.5 py-2 font-semibold whitespace-nowrap">
                    {h}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody className="divide-y divide-zinc-800/60 text-zinc-300">
              {block.points.map((p) => (
                <tr key={p.id}>
                  <td className="px-2.5 py-2 whitespace-nowrap">{p.x}</td>
                  {p.cells.map((c, j) => (
                    <td key={j} className="px-2.5 py-2">
                      {c !== '' ? c : '—'}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {block.summary && !repeatsReply(block.summary, replyText) && <p className="mt-2 text-xs leading-relaxed text-zinc-400">{block.summary}</p>}
    </div>
  );
};
