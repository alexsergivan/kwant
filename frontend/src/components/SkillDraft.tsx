// A skill drafted from Show it once (design §S, S5): the "Shown by you" badge, the draft in its
// sections, Try it once and the In use line with Delete steps now. The draft's text is shown as
// text; when it doesn't split into sections the card shows the SKILL.md as it is.
import React, { useId, useState } from 'react';
import { Loader2, MousePointer2, Play, TriangleAlert } from 'lucide-react';
import { InlinePart, SkillShown, dayOf, errText, inlineParts, isSkillShown, parseSkillSections, shortDay, showOnceApi, siteList, stepsWord, keepLine, textBlocks, trySites } from '../showOnce';
import type { Skill } from '../skills';
import { MaskChip } from './ShowOnce';

/** The skill's `shown`, when it's well formed; otherwise the card treats it as an ordinary skill. */
export const shownOf = (skill: Skill): SkillShown | null => (skill.source === 'shown' && isSkillShown(skill.shown) ? skill.shown : null);

export const ShownBadge: React.FC = () => (
  <span className="so-badge-you">
    <MousePointer2 size={10} aria-hidden="true" />
    Shown by you
  </span>
);

const Inline: React.FC<{ text: string }> = ({ text }) => (
  <>
    {inlineParts(text).map((p: InlinePart, i) =>
      p.t === 'text' ? <React.Fragment key={i}>{p.v}</React.Fragment> : p.t === 'code' ? <code key={i}>{p.v}</code> : <MaskChip key={i} text={p.text} label={p.label} />,
    )}
  </>
);

/** A text section: "- " or "* " lines as a bulleted list (styled like Steps), other lines as paragraphs. */
const TextSection: React.FC<{ text: string }> = ({ text }) => (
  <>
    {textBlocks(text).map((b, i) =>
      b.t === 'ul' ? (
        <ul key={i}>
          {b.items.map((x, j) => (
            <li key={j}>
              <Inline text={x} />
            </li>
          ))}
        </ul>
      ) : (
        <p key={i}>
          <Inline text={b.text} />
        </p>
      ),
    )}
  </>
);

/** A33: Goal, When to use, Inputs, Steps, Checks, What can go wrong, Sites. */
export const DraftSections: React.FC<{ body: string; shown: SkillShown; label: string }> = ({ body, shown, label }) => {
  const d = parseSkillSections(body);
  const shownLine = `Shown by you on ${shown.shown_on} · from ${stepsWord(shown.step_count)}`;
  if (!d) {
    return (
      <pre className="max-h-64 overflow-auto rounded-lg bg-zinc-950 border border-zinc-800 p-2.5 text-[11px] leading-relaxed text-zinc-300 whitespace-pre-wrap break-words font-mono">
        {body}
      </pre>
    );
  }
  // The examples come from the recording (api.md §4); the SKILL.md's own table is the fallback
  const inputs = shown.inputs.length ? shown.inputs.map((x) => ({ name: x.name, example: x.example })) : d.inputs;
  return (
    <div className="so-draft" role="region" aria-label={label} tabIndex={0}>
      <h6>Goal</h6>
      <p>
        <Inline text={d.goal} />
      </p>
      {d.whenToUse && (
        <>
          <h6>When to use</h6>
          <p>
            <Inline text={d.whenToUse} />
          </p>
        </>
      )}
      {(inputs.length > 0 || d.inputsText) && (
        <>
          <h6>Inputs</h6>
          {inputs.length > 0 ? (
            <table className="so-in">
              <thead>
                <tr>
                  <th scope="col">Name</th>
                  <th scope="col">Example from your run</th>
                </tr>
              </thead>
              <tbody>
                {inputs.map((x) => (
                  <tr key={x.name}>
                    <td>
                      <code>{x.name}</code>
                    </td>
                    <td>{x.example}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          ) : (
            <p>
              <Inline text={d.inputsText!} />
            </p>
          )}
        </>
      )}
      <h6>Steps</h6>
      <ol>
        {d.steps.map((x, i) => (
          <li key={i}>
            <Inline text={x} />
          </li>
        ))}
      </ol>
      {d.checks && (
        <>
          <h6>Checks</h6>
          <TextSection text={d.checks} />
        </>
      )}
      {d.wrong && (
        <>
          <h6>What can go wrong</h6>
          <TextSection text={d.wrong} />
        </>
      )}
      {d.sites.length > 0 && (
        <>
          <h6>Sites</h6>
          <p>
            {d.sites.map((h) => (
              <React.Fragment key={h}>
                <code>{h}</code>{' '}
              </React.Fragment>
            ))}
            <span className="so-mut">Your Browser site rules still apply.</span>
          </p>
        </>
      )}
      <p className="so-shown">{shownLine}</p>
    </div>
  );
};

/** S-S6: run the draft once, for real, with inputs the person can change. */
export const TryItOnce: React.FC<{ skill: Skill; shown: SkillShown; botName: string; onClose: () => void; onStarted?: (kwantId: string) => void }> = ({
  skill,
  shown,
  botName,
  onClose,
  onStarted,
}) => {
  const [values, setValues] = useState<Record<string, string>>(() => Object.fromEntries(shown.inputs.map((x) => [x.name, x.example])));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [started, setStarted] = useState(false);
  const id = useId();
  // A29 (Mira §3): where it acts for real, from the sites kwant recorded
  const sites = siteList(trySites(skill.body));

  const start = async () => {
    setBusy(true);
    setError(null);
    try {
      const r = await showOnceApi.tryOnce(skill.id, values);
      setStarted(true);
      onStarted?.(r.kwant_id);
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(false);
    }
  };

  if (started) {
    return (
      <div className="so-trybox" role="status">
        <b>Trial started in {botName}'s chat.</b>
        <span>It isn't approved yet; it still waits here for you.</span>
      </div>
    );
  }
  return (
    <div className="so-trybox" role="group" aria-labelledby={`${id}-h`}>
      <b id={`${id}-h`}>Try it once with</b>
      {shown.inputs.map((x) => (
        <label key={x.name} className="so-tin">
          <code>{x.name}</code>
          <input
            className="so-input"
            type="text"
            value={values[x.name] ?? ''}
            aria-description={x.description || undefined}
            onChange={(e) => setValues((v) => ({ ...v, [x.name]: e.target.value }))}
          />
        </label>
      ))}
      <p className="so-warn">
        <TriangleAlert size={12} aria-hidden="true" />
        <span>
          It runs for real
          {sites.length > 0 && ' on '}
          {sites.map((p, i) =>
            p.t === 'host' ? (
              <span key={i} className="so-host">
                {p.v}
              </span>
            ) : (
              p.v
            ),
          )}
          . Approvals still ask. You watch it in {botName}'s chat.
        </span>
      </p>
      {error && (
        <p className="so-err" role="alert">
          {error}
        </p>
      )}
      <div className="so-row">
        <button type="button" className="so-btn so-btn-ghost" onClick={onClose}>
          Cancel
        </button>
        <button type="button" className="so-btn so-btn-primary" disabled={busy} onClick={start}>
          {busy ? <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Play size={12} aria-hidden="true" />}
          Start the trial
        </button>
      </div>
    </div>
  );
};

/** Under a draft waiting for review: when its steps go. */
export const KeepLine: React.FC<{ shown: SkillShown }> = ({ shown }) => {
  return <p className="so-keep">{keepLine(dayOf(shown.steps_expire_at, false), shown.steps_deleted)}</p>;
};

/** In use: "Shown by you on Nov 7 · recorded steps kept until Nov 14 · Delete steps now". */
export const ShownLine: React.FC<{ skill: Skill; shown: SkillShown }> = ({ skill, shown }) => {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [gone, setGone] = useState(false);
  const deleted = shown.steps_deleted || gone;
  const until = dayOf(shown.steps_expire_at, false);
  const del = async () => {
    setBusy(true);
    setError(null);
    try {
      await showOnceApi.deleteSkillSteps(skill.id);
      setGone(true);
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="so-shownline">
      <ShownBadge />
      <span>on {shortDay(shown.shown_on)}</span>
      <span aria-hidden="true">·</span>
      {deleted ? (
        <span role={gone ? 'status' : undefined}>recorded steps deleted</span>
      ) : (
        <>
          <span>{until ? `recorded steps kept until ${until}` : 'recorded steps kept'}</span>
          <span aria-hidden="true">·</span>
          <button type="button" className="so-textbtn" disabled={busy} onClick={del} aria-label={`Delete the recorded steps of ${skill.name} now`}>
            Delete steps now
          </button>
        </>
      )}
      {error && (
        <span className="so-err w-full" role="alert">
          {error}
        </span>
      )}
    </div>
  );
};
