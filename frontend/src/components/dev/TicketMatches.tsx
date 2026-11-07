// 0.7.6 "Matches the ticket" on the Review card (design.md §6, api-decisions 17, 20, 21; api.md 2.11). When a PR
// maps to a ticket's brief, the review's own `matches` block replaces "Did it do what was asked?": one row per
// done-when line, met / not met / unclear, with the reviewer's evidence and the line's source chips as the
// server copied them. Fix it, Undo and Dismiss go to the item's own `k{n}` id on the findings route; its state
// is the item's `fix`, read as sent. Whether the block holds back LGTM is the server's `blocks_lgtm`, never
// worked out here. Counts are the rows' own states, not a new figure.

import React from 'react';
import { Check, ExternalLink, HelpCircle, Loader2, Target, X } from 'lucide-react';
import type { DevFinding, FindingAction } from '../../dev/types';
import type { MatchItem, ReviewMatches } from '../../dev/tickets';
import { extraPaths, matchCount, onSite } from '../../dev/tickets';
import { undoLeft } from '../../dev/logic';
import { CodeText, InferredChip, SiteOrigins, SourceChip } from './TicketBrief';

/**
 * A matches item as the findings route sees it: the item's server id and its `fix` state, so Fix it, Undo and
 * the send error go through the review card's one finding path. Null on a met item or one with no id.
 */
export function itemFinding(r: MatchItem): DevFinding | null {
  if (!r.id || !r.fix || r.state === 'met') return null;
  return {
    id: r.id,
    severity: r.state === 'not_met' ? 'high' : 'medium',
    title: r.criterion,
    file: null,
    line: null,
    detail: r.evidence,
    fix: null,
    source: 'review',
    state: r.fix.state,
    sent_at: r.fix.sent_at,
    sent_to: r.fix.sent_to,
    task_id: r.fix.task_id,
    send_error: r.fix.send_error,
    dismissed: r.fix.dismissed as DevFinding['dismissed'],
  };
}

/** The item ids that live on the Matches rows, so Findings never lists one of them twice. */
export const matchFindingIds = (m: ReviewMatches | null): Set<string> =>
  new Set(m ? m.items.map((it) => it.id).filter((x): x is string => !!x) : []);

const STATE: Record<MatchItem['state'], { word: string; chip: string | null; cls: string }> = {
  met: { word: 'Met', chip: null, cls: 'ok' },
  not_met: { word: 'Not met', chip: 'Not met', cls: 'bad' },
  unclear: { word: 'Unclear', chip: 'Unclear', cls: 'warn' },
};

const MRow: React.FC<{
  r: MatchItem;
  i: number;
  now: number;
  busy: boolean;
  act?: (f: DevFinding, a: FindingAction) => void;
}> = ({ r, i, now, busy, act }) => {
  const st = STATE[r.state];
  const f = itemFinding(r);
  const paths = extraPaths(r.evidence, r.paths);
  const left = f ? undoLeft(f, now) : 0;
  return (
    <li className="tk-mrow" data-m={st.cls} data-row={i + 1} data-item={r.id ?? undefined}>
      <span className="st" aria-hidden="true">
        {st.cls === 'ok' ? <Check size={12} /> : st.cls === 'bad' ? <X size={12} /> : <HelpCircle size={13} />}
      </span>
      <div className="body">
        <p className="tx">
          <span className="sr-only">
            Line {i + 1}, {st.word}:{' '}
          </span>
          <CodeText text={r.criterion} />
          {(r.inferred || r.sources.length > 0) && (
            <span className="srcs">
              {r.inferred && <InferredChip />}
              {r.sources.map((s, j) => (
                <SourceChip key={j} s={s} />
              ))}
            </span>
          )}
        </p>
        {(r.evidence || paths.length > 0) && (
          <p className="ev">
            {r.evidence && <CodeText text={r.evidence} />}
            {paths.length > 0 && (
              <>
                {r.evidence ? <span aria-hidden="true"> · </span> : null}
                <span className="sr-only">Files: </span>
                {paths.map((p, j) => (
                  <React.Fragment key={p}>
                    {j > 0 ? ', ' : ''}
                    <code>{p}</code>
                  </React.Fragment>
                ))}
              </>
            )}
          </p>
        )}
        {f?.send_error && f.state === 'open' && (
          <p className="tk-err" role="alert">
            Couldn’t send it: {f.send_error}
          </p>
        )}
      </div>
      {st.chip && (
        <span className="r">
          <span className={`kw-chip ${st.cls === 'bad' ? 'kw-chip-bad' : 'kw-chip-warn'}`} aria-hidden="true">
            {st.chip}
          </span>
          {f?.state === 'sent' ? (
            <>
              <span className="kw-chip kw-chip-ok" data-sent>
                Sent{f.sent_to ? ` to ${f.sent_to}` : ''}
              </span>
              {left > 0 && act && (
                <button type="button" className="tk-btn-sm" disabled={busy} onClick={() => act(f, { action: 'undo_fix' })} aria-label={`Undo Fix it: line ${i + 1}`} data-undo={f.id}>
                  Undo ({Math.ceil(left / 1000)} s)
                </button>
              )}
            </>
          ) : f?.state === 'dismissed' ? (
            <span className="kw-chip" data-dismissed>
              Dismissed
            </span>
          ) : (
            f &&
            act && (
              <button type="button" className="tk-btn-sm" disabled={busy} onClick={() => act(f, { action: 'fix' })} aria-label={`Fix it: line ${i + 1}`} data-fix={f.id}>
                {busy ? <Loader2 size={12} className="tk-spin" aria-hidden="true" /> : null}
                {f.send_error ? 'Try again' : 'Fix it'}
              </button>
            )
          )}
        </span>
      )}
    </li>
  );
};

export interface TicketMatchesProps {
  matches: ReviewMatches;
  /** The clock the review card ticks, for the Undo countdown. */
  now?: number;
  /** The saved Jira site's origins: the key links to Jira only on that site (api-decisions 18). */
  origins?: readonly string[];
  /** The brief's author for "Ann's brief"; falls back to "the brief". */
  lead?: string | null;
  busy?: boolean;
  act?: (f: DevFinding, a: FindingAction) => void;
}

export const TicketMatches: React.FC<TicketMatchesProps> = ({ matches, now = Date.now(), origins = [], lead = null, busy = false, act }) => {
  const { met, total } = matchCount(matches.items);
  const all = matches.verdict === 'met' && total > 0 && met === total;
  const url = onSite(matches.url, origins);
  return (
    <section className="tk-matches mt-4" aria-label="Matches the ticket" data-matches={`${met}/${total}`} data-verdict={matches.verdict} data-blocks-lgtm={matches.blocks_lgtm ? 'true' : 'false'}>
      <div className="tk-sech">
        <Target size={12} aria-hidden="true" />
        <h4>Matches the ticket</h4>
        {matches.key &&
          (url ? (
            <a className="tk-key" href={url} target="_blank" rel="noreferrer noopener">
              {matches.key}
              <ExternalLink size={10} aria-hidden="true" />
              <span className="sr-only"> (opens in Jira in a new tab)</span>
            </a>
          ) : (
            <span className="tk-key">{matches.key}</span>
          ))}
        {total > 0 && (
          <span className={all ? 'sum tk-ok' : 'sum tk-warn'}>
            {met} of {total} met
          </span>
        )}
      </div>
      <p className="tk-asked">
        Checked against the <b>Done when</b> lines in {lead ? `${lead}’s brief` : 'the brief'}, not against the PR’s own description.
      </p>
      {total === 0 ? (
        <p className="tk-asked">The brief had no done-when lines to check.</p>
      ) : (
        <SiteOrigins.Provider value={origins}>
          <ul className="tk-mrows" role="list">
            {matches.items.map((r, i) => (
              <MRow key={r.id ?? i} r={r} i={i} now={now} busy={busy} act={act} />
            ))}
          </ul>
        </SiteOrigins.Provider>
      )}
    </section>
  );
};
