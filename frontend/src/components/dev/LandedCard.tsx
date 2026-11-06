import React from 'react';
import { AlertTriangle, ChevronRight, FileText, FlaskConical, GitCommitHorizontal, GitMerge, ShieldCheck } from 'lucide-react';
import { Sticker } from '../Sticker';
import type { Forge, LandedCard as Landed, LandedListProps, ReviewCard } from '../../dev/types';
import { byDay, clock, forgeOpen, landedNode, openFindings, riskReasons, showVerified, testsWords } from '../../dev/logic';
import { DiffStat, Led, OutLink, RiskChip, Sha, Who, personText } from './parts';

const isTest = (path: string) => /(^|\/)(tests?|__tests__|e2e)\/|\.(test|spec)\.[a-z]+$|_test\.[a-z]+$/.test(path);

const TESTS_TONE = { passed: 'text-ok', failed: 'text-bad', none: 'text-zinc-400', unknown: 'text-zinc-400' } as const;
const TESTS_LED = { passed: 'ok', failed: 'bad', none: 'off', unknown: 'off' } as const;

/** One merge in plain words (design §4). A commit pushed without a PR (`pr: null`) is the same card, named by its commit. */
export const LandedCard: React.FC<{ card: Landed; reviews?: ReviewCard[]; onOpenReview?: (pr: number, repo?: string | null) => void; forge?: Forge | null; onRail?: boolean; repoLabel?: (repo: string | null, name?: string | null) => string | null }> = ({
  card,
  reviews = [],
  onOpenReview,
  forge = null,
  onRail = false,
  repoLabel,
}) => {
  const repoChip = repoLabel ? repoLabel(card.repo, card.repo_name) : null;
  const verified = showVerified(card, reviews);
  const writer = personText(card.written_by);
  const merged = clock(card.merged_at);
  const files = card.under_hood.files;
  const add = files.reduce((n, f) => n + f.add, 0);
  const del = files.reduce((n, f) => n + f.del, 0);
  const openFollowUps = card.follow_ups;
  // S1: a finding still open in this PR's review is something to do, even when CI and follow-ups are clear.
  const pr = card.pr;
  const findingsOpen = pr === null ? 0 : openFindings(reviews.find((r) => r.pr === pr && (!card.repo || !r.repo || r.repo === card.repo)));
  const nothingToDo = openFollowUps.length === 0 && card.tests.state === 'passed' && findingsOpen === 0;
  const tests = testsWords(card.tests.state);
  const diff = card.links.commit ?? (card.links.pr ? `${card.links.pr}/files` : null);
  const headingId = `landed-${card.id || pr || card.sha}-h`;
  const MergeIcon = pr === null ? GitCommitHorizontal : GitMerge;
  const github = card.links.pr ?? (pr === null ? card.links.commit : null);
  const node = landedNode(card);

  return (
    <article data-dev-card="landed" data-repo={card.repo ?? undefined} data-pr={pr ?? undefined} data-commit={pr === null ? card.sha : undefined} data-verified={verified || undefined} aria-labelledby={headingId} className="kw-card relative min-w-0 p-4">
      {onRail && (
        <>
          {pr !== null && <span aria-hidden="true" data-rail-arc className="dv-arc" />}
          <span aria-hidden="true" data-rail-node={node} data-s={node} className="dv-node" />
        </>
      )}
      <div className="flex flex-wrap items-center gap-1.5">
        {card.area && <span className="kw-chip">{card.area}</span>}
        {repoChip && (
          <span data-dev-card-repo className="kw-chip font-mono [overflow-wrap:anywhere]" title={card.repo ?? undefined}>
            <span className="sr-only">Repo </span>
            {repoChip}
          </span>
        )}
        <Sha sha={card.sha} />
        {pr !== null ? (
          card.links.pr ? (
            <a href={card.links.pr} target="_blank" rel="noreferrer noopener" aria-label={`PR #${pr}`} className="kw-focus kw-hit rounded font-mono text-[11.5px] text-zinc-400 hover:text-sky-300">
              #{pr}
            </a>
          ) : (
            <span className="font-mono text-[11.5px] text-zinc-400">
              <span className="sr-only">PR </span>#{pr}
            </span>
          )
        ) : (
          <span className="kw-chip">Direct push</span>
        )}
        {card.risk && <RiskChip level={card.risk.level} />}
        {card.meta && <span className="text-meta text-zinc-400">{card.meta}</span>}
        {verified && (
          <Sticker tone="holo" icon={<ShieldCheck aria-hidden="true" />} className="ml-auto" ariaLabel="Verified: CI is green and someone other than the author reviewed it" title="CI green and reviewed by someone other than the author">
            Verified
          </Sticker>
        )}
      </div>

      <h3 id={headingId} className="mt-2 text-title font-semibold text-zinc-50 [overflow-wrap:anywhere]">
        {card.headline}
      </h3>
      <div className="mt-1 flex flex-wrap items-center gap-x-1 gap-y-0.5 text-meta text-zinc-400">
        <MergeIcon size={13} className="shrink-0 text-ok" aria-hidden="true" />
        <span>
          {pr === null ? 'Committed' : 'Merged'}
          {merged ? ` ${merged}` : ''}
          {card.merged_by && (
            <>
              {' '}by <span className="font-medium text-zinc-100">{card.merged_by}</span>
            </>
          )}
        </span>
        {writer && (
          <>
            <span aria-hidden="true">·</span>
            <span>written by</span>
            <Who name={writer.name} />
            {writer.sub && <span>({writer.sub})</span>}
          </>
        )}
      </div>

      {card.status === 'pending' && <p className="mt-3 text-meta text-zinc-400">Writing this change in plain words…</p>}
      {card.error && (
        <p className="mt-3 text-meta text-zinc-400">
          No plain-words text for this change: <span className="text-zinc-300">{card.error}</span>
        </p>
      )}

      {card.for_you && (
        <div className="mt-3">
          <p className="text-2xs font-semibold uppercase tracking-wider text-zinc-400">What changes for you</p>
          <p className="mt-1 text-body text-zinc-100 [overflow-wrap:anywhere]">{card.for_you}</p>
        </div>
      )}

      {(files.length > 0 || card.under_hood.summary) && (
        <details className="group mt-3 border-t border-hairline pt-3">
          <summary className="kw-focus flex min-h-[44px] cursor-pointer list-none items-center gap-1.5 rounded text-meta text-zinc-400 hover:text-zinc-200 sm:min-h-0 [&::-webkit-details-marker]:hidden">
            <ChevronRight size={14} aria-hidden="true" className="shrink-0 transition-transform duration-[160ms] group-open:rotate-90 motion-reduce:transition-none" />
            <span>Under the hood</span>
            {files.length > 0 && (
              <>
                <span aria-hidden="true">·</span>
                <span className="font-mono text-[11.5px]">
                  {files.length} {files.length === 1 ? 'file' : 'files'}
                </span>
                <DiffStat add={add} del={del} />
              </>
            )}
          </summary>
          {card.under_hood.summary && <p className="mt-2 text-meta text-zinc-300 [overflow-wrap:anywhere]">{card.under_hood.summary}</p>}
          {files.length > 0 && (
            <ul role="list" className="mt-2 space-y-1">
              {files.map((f) => {
                const Icon = isTest(f.path) ? FlaskConical : FileText;
                return (
                  <li key={f.path} className="flex min-w-0 items-start gap-2 font-mono text-[12px] leading-5">
                    <Icon size={12} className="mt-1 shrink-0 text-zinc-500" aria-hidden="true" />
                    <span className="min-w-0 flex-1 text-zinc-200 [overflow-wrap:anywhere]">{f.path}</span>
                    <DiffStat add={f.add} del={f.del} className="mt-[5px] shrink-0" />
                  </li>
                );
              })}
            </ul>
          )}
          {card.risk && (
            <p className="mt-2 text-2xs text-zinc-400">
              Risk reasons: {riskReasons(card.risk).join(' · ')}
            </p>
          )}
          {card.cut && <p className="mt-1 text-2xs text-zinc-400">{card.cut}</p>}
        </details>
      )}

      {openFollowUps.length > 0 && (
        <div role="note" className="mt-3 flex items-start gap-2 rounded-xl border border-amber-300/35 bg-amber-300/10 px-3 py-2 text-meta text-zinc-100">
          <AlertTriangle size={14} className="mt-0.5 shrink-0 text-warn" aria-hidden="true" />
          <p className="min-w-0 [overflow-wrap:anywhere]">
            <span className="font-semibold text-warn">
              {openFollowUps.length} follow-up{openFollowUps.length === 1 ? '' : 's'} open:
            </span>{' '}
            {openFollowUps.map((f, i) => (
              <span key={i}>
                {i > 0 && ' · '}
                {f.text}
                {f.ref && ` (${f.ref})`}
              </span>
            ))}
          </p>
        </div>
      )}

      <div className="mt-3 flex flex-wrap items-center gap-x-3 gap-y-1 border-t border-hairline pt-2.5 text-meta">
        <span data-tests={card.tests.state} className={`inline-flex items-center gap-[7px] ${TESTS_TONE[card.tests.state]}`}>
          <Led s={TESTS_LED[card.tests.state]} />
          {card.tests.state === 'failed' ? (
            <span>
              {tests}
              {card.links.ci && (
                <>
                  {' · '}
                  <a href={card.links.ci} target="_blank" rel="noreferrer noopener" className="kw-focus kw-hit rounded text-sky-300 hover:underline">
                    see CI
                  </a>
                </>
              )}
            </span>
          ) : card.links.ci ? (
            <a href={card.links.ci} target="_blank" rel="noreferrer noopener" className="kw-focus kw-hit rounded hover:underline">
              {tests}
            </a>
          ) : (
            <span>{tests}</span>
          )}
        </span>
        <span className="text-zinc-400">{card.reviewed_by ? `Reviewed by ${card.reviewed_by}` : 'No bot review'}</span>
        {nothingToDo && <span className="text-zinc-400">Nothing to do for you.</span>}
        {findingsOpen > 0 && (
          <span data-findings-open={findingsOpen} className="inline-flex flex-wrap items-center gap-x-1">
            <span className="font-semibold text-amber-300">
              {findingsOpen} {findingsOpen === 1 ? 'finding' : 'findings'} open
            </span>
            <span className="text-zinc-400">in the review</span>
            {onOpenReview && pr !== null && (
              <>
                <span aria-hidden="true" className="text-zinc-400">
                  ·
                </span>
                <button
                  type="button"
                  onClick={() => onOpenReview(pr, card.repo)}
                  className="kw-focus inline-flex min-h-[44px] items-center rounded text-sky-300 hover:text-sky-200 hover:underline sm:min-h-0"
                >
                  Open review
                </button>
              </>
            )}
          </span>
        )}
        <span className="flex items-center gap-3 sm:ml-auto">
          {diff && diff !== github && (
            <a href={diff} target="_blank" rel="noreferrer noopener" className="kw-focus kw-hit inline-flex min-h-[44px] items-center rounded text-sky-300 hover:text-sky-200 sm:min-h-0">
              Diff
            </a>
          )}
          {github && (
            <OutLink href={github} className="min-h-[44px] sm:min-h-0">
              {forgeOpen(forge, card.links.pr ? 'pr' : 'commit')}
            </OutLink>
          )}
        </span>
      </div>
    </article>
  );
};

/** What landed: one git rail down the left (style v2 §5), day labels sitting on it, a node per card. */
export const LandedList: React.FC<LandedListProps> = ({ cards, reviews, now = Date.now(), onOpenReview, forge = null, repoLabel }) => (
  <div data-dev-landed-list className="dv-rail space-y-3">
    {byDay(cards, now).map((day) => (
      <section key={day.label} aria-label={day.label} className="space-y-3">
        <h3 className="-ml-[30px] flex items-center gap-2 pt-2 font-mono text-[10.5px] font-semibold uppercase leading-none tracking-[0.06em] text-zinc-400">
          <span data-rail-day className="relative rounded bg-surface-main py-1 pl-0.5 pr-2">{day.label}</span>
          <span aria-hidden="true" className="h-px flex-1 bg-[var(--kw-hairline)]" />
        </h3>
        {day.cards.map((c) => (
          <LandedCard key={c.id || c.pr || c.sha} card={c} reviews={reviews} onOpenReview={onOpenReview} forge={forge} repoLabel={repoLabel} onRail />
        ))}
      </section>
    ))}
  </div>
);
