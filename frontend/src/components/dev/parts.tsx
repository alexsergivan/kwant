import React from 'react';
import { ExternalLink } from 'lucide-react';
import { ModernAvatar } from '../ModernAvatar';
import type { DevPerson, RiskLevel } from '../../dev/types';
import { RISK_WORD } from '../../dev/logic';

/** "Claude Opus" from a label, or a model id like claude-opus-5-5, else the engine's name. */
export function modelLabel(engine: string | null, model: string | null): string | null {
  if (model && /\s/.test(model)) return model;
  const m = model?.match(/^claude-(opus|sonnet|haiku)/i);
  if (m) return `Claude ${m[1][0].toUpperCase()}${m[1].slice(1).toLowerCase()}`;
  const names: Record<string, string> = { claude: 'Claude', codex: 'Codex', grok: 'Grok', ollama: 'Ollama' };
  return engine ? names[engine] ?? engine : null;
}

export const RISK_CHIP: Record<RiskLevel, string> = { low: 'kw-chip-ok', medium: 'kw-chip-warn', high: 'kw-chip-bad' };

export const RiskChip: React.FC<{ level: RiskLevel }> = ({ level }) => <span className={`kw-chip ${RISK_CHIP[level]}`}>{RISK_WORD[level]}</span>;

/** A 16 px avatar plus the name; the avatar is decoration, the name is read. */
export const Who: React.FC<{ name: string; strong?: boolean }> = ({ name, strong = true }) => (
  <span className="inline-flex items-center gap-1 align-middle">
    <span aria-hidden="true" className="inline-flex shrink-0">
      <ModernAvatar name={name} size={16} />
    </span>
    <span className={strong ? 'font-medium text-zinc-100' : ''}>{name}</span>
  </span>
);

export const personText = (p: DevPerson | null) => {
  if (!p) return null;
  if (p.kind === 'person') return { name: p.name, sub: null };
  const m = modelLabel(p.engine, p.model);
  return { name: p.name, sub: m ? `bot · ${m}` : 'bot' };
};

/** An https link that opens in a new tab and says so. */
export const OutLink: React.FC<{ href: string; children: React.ReactNode; className?: string }> = ({ href, children, className = '' }) => (
  <a
    href={href}
    target="_blank"
    rel="noreferrer noopener"
    className={`kw-focus inline-flex items-center gap-1 rounded text-sky-300 hover:text-sky-200 ${className}`}
  >
    {children}
    <ExternalLink size={12} aria-hidden="true" />
    <span className="sr-only">(opens in a new tab)</span>
  </a>
);

export const SectionHead: React.FC<{ icon?: React.ReactNode; children: React.ReactNode; extra?: React.ReactNode }> = ({ icon, children, extra }) => (
  <h4 className="mb-2 flex flex-wrap items-center gap-1.5 text-2xs font-semibold uppercase tracking-wider text-zinc-400">
    {icon}
    <span>{children}</span>
    {extra}
  </h4>
);

/**
 * Drawn in place of a card the renderer can't trust (wrong shape): the PR number and title as plain text
 * and the GitHub link, nothing computed.
 */
export const CardFallback: React.FC<{ raw: unknown }> = ({ raw }) => {
  const o = (raw && typeof raw === 'object' ? raw : {}) as Record<string, unknown>;
  const pr = typeof o.pr === 'number' ? o.pr : null;
  const title = typeof o.title === 'string' ? o.title : typeof o.headline === 'string' ? o.headline : null;
  const links = (o.links && typeof o.links === 'object' ? o.links : {}) as Record<string, unknown>;
  const href = typeof links.pr === 'string' && /^https:\/\//.test(links.pr) ? links.pr : null;
  return (
    <div data-dev-card="fallback" className="kw-card p-4 text-body text-zinc-300">
      <span className="[overflow-wrap:anywhere]">{pr !== null ? `PR #${pr}` : 'A card'}{title ? `: ${title}` : ''}</span>
      <span className="text-zinc-500"> · This card couldn’t be shown.</span>
      {href && (
        <>
          {' '}
          <OutLink href={href}>Open on GitHub</OutLink>
        </>
      )}
    </div>
  );
};
