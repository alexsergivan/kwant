import React from 'react';
import { AlertTriangle, CircleDashed, Clock, XCircle, CheckCircle2 } from 'lucide-react';
import { CheckStatus, VerificationBlock, VerificationCheck, inRange, isObj, isText, optText, parseCommon, uniqueIds } from '../../smartResults';
import type { RendererProps } from './registry';

const STATUSES: CheckStatus[] = ['passed', 'failed', 'error', 'pending'];

/** 1-20 checks against one task, each with a fixed-enum status (plan §A.3). */
export function validVerification(raw: unknown): VerificationBlock | null {
  const b = parseCommon(raw, 'verification');
  if (!b || b.source.kind !== 'slot') return null;
  const target = b.target;
  if (!isObj(target) || target.kind !== 'task' || !isText(target.id, 120) || !isText(target.title)) return null;
  if (!inRange(b.checks, 1, 20) || !uniqueIds(b.checks)) return null;
  if (!isText(b.summary)) return null;
  const checks: VerificationCheck[] = [];
  for (const c of b.checks as Record<string, unknown>[]) {
    if (!isText(c.label) || !isText(c.kind, 40) || !STATUSES.includes(c.status as CheckStatus)) return null;
    const excerpt = optText(c.excerpt, 300);
    if (excerpt === undefined) return null;
    const checkedAt = c.checked_at === null || c.checked_at === undefined ? null : isText(c.checked_at, 60) ? c.checked_at : undefined;
    if (checkedAt === undefined) return null;
    checks.push({ id: c.id as string, label: c.label, kind: c.kind, status: c.status as CheckStatus, excerpt, checked_at: checkedAt });
  }
  return {
    id: b.id,
    type: 'verification',
    title: b.title,
    source: b.source,
    provenance: b.provenance,
    target: { kind: 'task', id: target.id, title: target.title },
    checks,
    summary: b.summary,
  };
}

const STATUS_TEXT: Record<CheckStatus, string> = { passed: 'Passed', failed: 'Failed', error: 'Error', pending: 'Pending' };
const STATUS_ICON: Record<CheckStatus, React.ElementType> = { passed: CheckCircle2, failed: XCircle, error: AlertTriangle, pending: CircleDashed };
const STATUS_COLOR: Record<CheckStatus, string> = {
  passed: 'text-emerald-400',
  failed: 'text-rose-400',
  error: 'text-amber-400',
  pending: 'text-zinc-400',
};

const STATUS_PILL: Record<CheckStatus, string> = {
  passed: 'border-emerald-500/40 bg-emerald-500/10 text-emerald-300',
  failed: 'border-rose-500/40 bg-rose-500/10 text-rose-300',
  error: 'border-amber-500/40 bg-amber-500/10 text-amber-300',
  pending: 'border-zinc-600 bg-zinc-800/60 text-zinc-300',
};

function formatWhen(s: string | null): string | null {
  if (!s) return null;
  const d = new Date(s);
  return Number.isNaN(d.getTime()) ? s : d.toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
}

/** A task's outcome checks: the summary line, then each check with status, an expandable excerpt, and time. */
export const Verification: React.FC<RendererProps<VerificationBlock>> = ({ block }) => (
  <div data-smart-result="verification" data-block-id={block.id} className="my-3.5 rounded-xl border border-zinc-700/60 bg-zinc-900/60 shadow-[var(--kws-shadow)]">
    <div className="border-b border-zinc-700/60 px-3.5 py-3">
      <p className="text-sm font-semibold text-zinc-100">{block.summary}</p>
      <p className="mt-0.5 truncate text-[11px] text-zinc-400" title={block.target.title}>
        {block.target.title}
      </p>
    </div>
    <ul role="list" className="divide-y divide-zinc-700/40">
      {block.checks.map((check) => {
        const Icon = STATUS_ICON[check.status];
        const when = formatWhen(check.checked_at);
        return (
          <li key={check.id} className="px-3.5 py-2.5">
            <div className="flex items-start gap-2">
              <Icon size={14} className={`mt-0.5 shrink-0 ${STATUS_COLOR[check.status]}`} aria-hidden="true" />
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
                  <span className="text-xs font-medium text-zinc-100 [overflow-wrap:anywhere]">{check.label}</span>
                  <span className={`rounded-full border px-2 py-px text-[11px] font-medium ${STATUS_PILL[check.status]}`}>{STATUS_TEXT[check.status]}</span>
                  <span className="text-[11px] text-zinc-400">{check.kind}</span>
                </div>
                {when && (
                  <div className="mt-0.5 flex items-center gap-1 text-[11px] text-zinc-400">
                    <Clock size={10} aria-hidden="true" />
                    {when}
                  </div>
                )}
                {check.excerpt && (
                  <details className="mt-1">
                    <summary className="cursor-pointer rounded text-[11px] text-sky-400 hover:text-sky-300 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-500/70">
                      Details
                    </summary>
                    <pre className="mt-1 whitespace-pre-wrap rounded-lg bg-zinc-900 p-2 font-mono text-[11px] leading-relaxed text-zinc-300">
                      {check.excerpt}
                    </pre>
                  </details>
                )}
              </div>
            </div>
          </li>
        );
      })}
    </ul>
  </div>
);
