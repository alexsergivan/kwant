// 0.7.6 "Matches the ticket" on the Review card (design.md §6). When a PR maps to a ticket's brief it
// replaces "Did it do what was asked?": one row per done-when line, met / not met / unclear, with the
// evidence the reviewer gave. Not met and unclear rows get Fix it, which sends the matching `k{n}` finding
// through the same fix_task as any finding. Counts are the rows' own states, not a new figure.

import React from 'react';
import { Check, ExternalLink, HelpCircle, Loader2, Target, X } from 'lucide-react';
import type { DevFinding, FindingAction, ReviewCard } from '../../dev/types';
import { matchCount, safeHref } from '../../dev/tickets';
import { CodeText } from './TicketBrief';

type Intent = NonNullable<ReviewCard['intent']>;
type Row = Intent['rows'][number];

/** The finding a row's Fix it sends: Vega's ticket findings are `k{done_when_index + 1}` with source ticket. */
export const rowFinding = (findings: DevFinding[], i: number): DevFinding | null =>
  findings.find((f) => f.source === 'ticket' && f.id === `k${i + 1}`) ?? null;

/** True when the card should show Matches the ticket instead of the generic intent section. */
export const isTicketIntent = (intent: ReviewCard['intent']): intent is Intent => !!intent && intent.source === 'ticket' && !intent.none;

const STATE: Record<string, { word: string; chip: string | null; cls: string }> = {
  met: { word: 'Met', chip: null, cls: 'ok' },
  not_met: { word: 'Not met', chip: 'Not met', cls: 'bad' },
  unclear: { word: 'Unclear', chip: 'Unclear', cls: 'warn' },
};

const MRow: React.FC<{ r: Row; i: number; f: DevFinding | null; busy: boolean; act?: (f: DevFinding, a: FindingAction) => void }> = ({ r, i, f, busy, act }) => {
  const st = STATE[r.state] ?? STATE.unclear!;
  const sent = f?.state === 'sent';
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
          <CodeText text={r.text} />
        </p>
        {(r.evidence_text || (r.evidence_paths && r.evidence_paths.length > 0)) && (
          <p className="ev">
            {r.evidence_text && <CodeText text={r.evidence_text} />}
            {r.evidence_paths && r.evidence_paths.length > 0 && (
              <>
                {r.evidence_text ? <span aria-hidden="true"> · </span> : null}
                <span className="sr-only">Files: </span>
                {r.evidence_paths.map((p, j) => (
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
  intent: Intent;
  findings: DevFinding[];
  /** The brief's author for "Ann's brief"; falls back to "the brief". */
  lead?: string | null;
  busy?: boolean;
  act?: (f: DevFinding, a: FindingAction) => void;
}

export const TicketMatches: React.FC<TicketMatchesProps> = ({ intent, findings, lead = null, busy = false, act }) => {
  const { met, total } = matchCount(intent.rows);
  const all = total > 0 && met === total;
  const url = intent.url ? safeHref(intent.url) : null;
  return (
    <section className="tk-matches mt-4" aria-label="Matches the ticket" data-matches={`${met}/${total}`}>
      <div className="tk-sech">
        <Target size={12} aria-hidden="true" />
        <h4>Matches the ticket</h4>
        {intent.key &&
          (url ? (
            <a className="tk-key" href={url} target="_blank" rel="noreferrer noopener">
              {intent.key}
              <ExternalLink size={10} aria-hidden="true" />
              <span className="sr-only"> (opens in Jira in a new tab)</span>
            </a>
          ) : (
            <span className="tk-key">{intent.key}</span>
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
        <ul className="tk-mrows" role="list">
          {intent.rows.map((r, i) => (
            <MRow key={i} r={r} i={i} f={r.state === 'met' ? null : rowFinding(findings, i)} busy={busy} act={act} />
          ))}
        </ul>
      )}
    </section>
  );
};
