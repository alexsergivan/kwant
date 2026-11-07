// 0.7.6 "Matches the ticket" on the Review card (design.md §6, api-decisions 17). When a PR maps to a ticket's
// brief, the review's own `matches {verdict, items}` block replaces "Did it do what was asked?": one row per
// done-when line, met / not met / unclear, with the reviewer's evidence and the line's source chips from the
// brief. A row gets Fix it only when the review has a finding `k{n}` for it, sent through the same fix_task
// as any finding. Counts are the rows' own states, not a new figure.

import React from 'react';
import { Check, ExternalLink, HelpCircle, Loader2, Target, X } from 'lucide-react';
import type { DevFinding, FindingAction } from '../../dev/types';
import type { Brief, MatchItem, ReviewMatches } from '../../dev/tickets';
import { doneWhenFor, extraPaths, matchCount, onSite } from '../../dev/tickets';
import { CodeText, InferredChip, SiteOrigins, SourceChip } from './TicketBrief';

/** The finding a row's Fix it sends: the review's `k{n}` finding for done-when line n, whatever its source. */
export const rowFinding = (findings: DevFinding[], i: number): DevFinding | null => findings.find((f) => f.id === `k${i + 1}`) ?? null;

/** The `k{n}` findings that live on the Matches rows, so Findings doesn't list them twice. */
export const matchFindingIds = (m: ReviewMatches | null): Set<string> =>
  new Set(m ? m.items.map((it, i) => (it.state === 'met' ? null : `k${i + 1}`)).filter((x): x is string => !!x) : []);

const STATE: Record<MatchItem['state'], { word: string; chip: string | null; cls: string }> = {
  met: { word: 'Met', chip: null, cls: 'ok' },
  not_met: { word: 'Not met', chip: 'Not met', cls: 'bad' },
  unclear: { word: 'Unclear', chip: 'Unclear', cls: 'warn' },
};

const MRow: React.FC<{
  r: MatchItem;
  i: number;
  brief: Brief | null;
  f: DevFinding | null;
  busy: boolean;
  act?: (f: DevFinding, a: FindingAction) => void;
}> = ({ r, i, brief, f, busy, act }) => {
  const st = STATE[r.state];
  const sent = f?.state === 'sent';
  const line = r.sources.length > 0 || r.inferred ? null : doneWhenFor(brief, r, i);
  const sources = r.sources.length > 0 ? r.sources : line?.sources ?? [];
  const inferred = r.inferred || !!line?.inferred;
  const paths = extraPaths(r.evidence, r.paths);
  return (
    <li className="tk-mrow" data-m={st.cls} data-row={i + 1}>
      <span className="st" aria-hidden="true">
        {st.cls === 'ok' ? <Check size={12} /> : st.cls === 'bad' ? <X size={12} /> : <HelpCircle size={13} />}
      </span>
      <div className="body">
        <p className="tx">
          <span className="sr-only">
            Line {i + 1}, {st.word}:{' '}
          </span>
          <CodeText text={r.criterion} />
          {(inferred || sources.length > 0) && (
            <span className="srcs">
              {inferred && <InferredChip />}
              {sources.map((s, j) => (
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
          {sent ? (
            <span className="kw-chip kw-chip-ok" data-sent>
              Sent{f?.sent_to ? ` to ${f.sent_to}` : ''}
            </span>
          ) : (
            f &&
            act &&
            f.state === 'open' && (
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
  findings: DevFinding[];
  /** The ticket's brief, for each line's source chips when the review didn't copy them. */
  brief?: Brief | null;
  /** The saved Jira site's origins: the key links to Jira only on that site (api-decisions 18). */
  origins?: readonly string[];
  /** The brief's author for "Ann's brief"; falls back to "the brief". */
  lead?: string | null;
  busy?: boolean;
  act?: (f: DevFinding, a: FindingAction) => void;
}

export const TicketMatches: React.FC<TicketMatchesProps> = ({ matches, findings, brief = null, origins = [], lead = null, busy = false, act }) => {
  const { met, total } = matchCount(matches.items);
  const all = matches.verdict === 'met' && total > 0 && met === total;
  const url = onSite(matches.url, origins);
  return (
    <section className="tk-matches mt-4" aria-label="Matches the ticket" data-matches={`${met}/${total}`} data-verdict={matches.verdict}>
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
              <MRow key={i} r={r} i={i} brief={brief} f={r.state === 'met' ? null : rowFinding(findings, i)} busy={busy} act={act} />
            ))}
          </ul>
        </SiteOrigins.Provider>
      )}
    </section>
  );
};
