import React, { useCallback, useEffect, useState } from 'react';
import { useUndoableDelete } from '../undoDelete';
import { UndoNotice } from './UndoNotice';
import { Bot, Check, ChevronDown, ChevronRight, Globe2, Loader2, Pencil, Plus, RefreshCw, Trash2, User, X } from 'lucide-react';
import { formatWhen, onContinuity } from '../continuity';
import { playSfx } from '../audio/chiptune';
import {
  SKILL_LIMITS,
  Skill,
  SkillDiff,
  SkillDraft,
  applySkillEvent,
  groupSkills,
  skillDraftError,
  skillsApi,
  slugifySkillName,
} from '../skills';

const smallBtn =
  'inline-flex items-center gap-1 px-2 py-1 rounded-md text-[11px] font-medium border transition-colors disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500';
const quietBtn = `${smallBtn} border-zinc-700/70 bg-zinc-800/70 text-zinc-200 hover:bg-zinc-700`;
const goBtn = `${smallBtn} border-emerald-500/40 bg-emerald-600/80 text-white hover:bg-emerald-500`;
const dangerBtn = `${smallBtn} border-rose-500/30 bg-rose-500/10 text-rose-300 hover:bg-rose-500/20`;
const inputClass =
  'w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors';

const errText = (e: unknown) => (e instanceof Error ? e.message : 'That failed.');

/** Name, description and steps; used to write a new skill and to edit one (before approving). */
const SkillForm: React.FC<{
  idPrefix: string;
  initial: SkillDraft;
  submitLabel: string;
  onSubmit: (d: SkillDraft) => Promise<string | void>;
  onCancel: () => void;
}> = ({ idPrefix, initial, submitLabel, onSubmit, onCancel }) => {
  const [d, setD] = useState<SkillDraft>(initial);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const problem = skillDraftError(d);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (problem) {
      setError(problem);
      return;
    }
    setBusy(true);
    const err = await onSubmit({ name: d.name.trim(), description: d.description.trim(), body: d.body });
    setBusy(false);
    if (err) setError(err);
  };

  return (
    <form onSubmit={submit} className="space-y-2" aria-label={submitLabel}>
      <div>
        <label htmlFor={`${idPrefix}-name`} className="block text-[11px] font-medium text-zinc-400 mb-1">
          Name
        </label>
        <input
          id={`${idPrefix}-name`}
          value={d.name}
          maxLength={SKILL_LIMITS.name}
          onChange={(e) => setD({ ...d, name: e.target.value })}
          onBlur={() => setD((x) => ({ ...x, name: slugifySkillName(x.name) }))}
          placeholder="weekly-report"
          className={`${inputClass} font-mono`}
          autoComplete="off"
        />
      </div>
      <div>
        <label htmlFor={`${idPrefix}-desc`} className="block text-[11px] font-medium text-zinc-400 mb-1">
          When to use it
        </label>
        <input
          id={`${idPrefix}-desc`}
          value={d.description}
          maxLength={SKILL_LIMITS.description}
          onChange={(e) => setD({ ...d, description: e.target.value })}
          placeholder="Building the Friday status report from the repo and tickets"
          className={inputClass}
        />
      </div>
      <div>
        <label htmlFor={`${idPrefix}-body`} className="block text-[11px] font-medium text-zinc-400 mb-1">
          Steps (Markdown)
        </label>
        <textarea
          id={`${idPrefix}-body`}
          value={d.body}
          rows={7}
          onChange={(e) => setD({ ...d, body: e.target.value })}
          className={`${inputClass} font-mono resize-y`}
        />
        <span className={`block text-right text-[10px] font-mono ${d.body.length > SKILL_LIMITS.body ? 'text-rose-400' : 'text-zinc-500'}`}>
          {d.body.length}/{SKILL_LIMITS.body}
        </span>
      </div>
      {error && (
        <p className="text-[11px] text-rose-400" role="alert">
          {error}
        </p>
      )}
      <div className="flex items-center justify-end gap-2">
        <button type="button" onClick={onCancel} className={quietBtn}>
          Cancel
        </button>
        <button type="submit" disabled={busy} className={goBtn}>
          {busy ? <Loader2 size={11} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Check size={11} aria-hidden="true" />}
          {submitLabel}
        </button>
      </div>
    </form>
  );
};

/** What approving changes: the whole text for a new skill, line changes for a revision. */
const SkillChanges: React.FC<{ skill: Skill }> = ({ skill }) => {
  const [diff, setDiff] = useState<SkillDiff | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    setError(null);
    skillsApi.diff(skill.id).then(setDiff, (e) => setError(errText(e)));
  }, [skill.id]);
  useEffect(() => {
    if (skill.replaces_id) load();
  }, [skill.replaces_id, skill.updated_at, load]);

  if (!skill.replaces_id) {
    return (
      <pre className="max-h-64 overflow-auto rounded-lg bg-zinc-950 border border-zinc-800 p-2.5 text-[11px] leading-relaxed text-zinc-300 whitespace-pre-wrap break-words font-mono">
        {skill.body}
      </pre>
    );
  }
  if (error) {
    return (
      <div className="flex items-center justify-between gap-2 text-[11px] text-rose-300" role="alert">
        <span>Couldn't load the changes: {error}</span>
        <button type="button" onClick={load} className={quietBtn}>
          <RefreshCw size={11} aria-hidden="true" /> Retry
        </button>
      </div>
    );
  }
  if (!diff) {
    return (
      <p className="flex items-center gap-2 text-[11px] text-zinc-400" role="status">
        <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Loading changes…
      </p>
    );
  }
  const added = diff.lines.filter((l) => l.op === 'add').length;
  const removed = diff.lines.filter((l) => l.op === 'del').length;
  return (
    <div className="space-y-1">
      <p className="text-[10px] text-zinc-500">
        <span className="text-emerald-300">+{added}</span> <span className="text-rose-300">−{removed}</span> lines
        {diff.description_changed && diff.base && <> · description was “{diff.base.description}”</>}
      </p>
      <div
        className="max-h-64 overflow-auto rounded-lg bg-zinc-950 border border-zinc-800 py-1.5 text-[11px] leading-relaxed font-mono"
        role="region"
        aria-label={`Changes to ${skill.name}`}
        tabIndex={0}
      >
        {diff.lines.map((l, i) => (
          <div
            key={i}
            className={`px-2.5 whitespace-pre-wrap break-words ${
              l.op === 'add' ? 'bg-emerald-500/10 text-emerald-200' : l.op === 'del' ? 'bg-rose-500/10 text-rose-200 line-through decoration-rose-400/40' : 'text-zinc-400'
            }`}
          >
            <span className="select-none inline-block w-3 text-zinc-500" aria-hidden="true">
              {l.op === 'add' ? '+' : l.op === 'del' ? '−' : ' '}
            </span>
            <span className="sr-only">{l.op === 'add' ? 'Added: ' : l.op === 'del' ? 'Removed: ' : ''}</span>
            {l.text || ' '}
          </div>
        ))}
      </div>
    </div>
  );
};

const Badge: React.FC<{ tone: string; children: React.ReactNode }> = ({ tone, children }) => (
  <span className={`inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-medium border ${tone}`}>{children}</span>
);

const SkillMeta: React.FC<{ skill: Skill; replaces?: Skill }> = ({ skill, replaces }) => (
  <div className="flex flex-wrap items-center gap-1.5 text-[10px] text-zinc-500">
    {skill.source === 'agent' ? (
      <Badge tone="bg-violet-500/10 text-violet-300 border-violet-500/20">
        <Bot size={10} aria-hidden="true" /> Proposed
      </Badge>
    ) : (
      <Badge tone="bg-sky-500/10 text-sky-300 border-sky-500/20">
        <User size={10} aria-hidden="true" /> You
      </Badge>
    )}
    {skill.kwant_id === null && (
      <Badge tone="bg-zinc-800 text-zinc-300 border-zinc-700">
        <Globe2 size={10} aria-hidden="true" /> Every bot in this space
      </Badge>
    )}
    {skill.replaces_id && <Badge tone="bg-amber-500/10 text-amber-300 border-amber-500/20">Update to {replaces?.name ?? 'an enabled skill'}</Badge>}
    <span>{formatWhen(skill.status === 'pending' ? skill.created_at : skill.updated_at)}</span>
  </div>
);

export const PendingSkill: React.FC<{ skill: Skill; replaces?: Skill; initialPending?: boolean }> = ({ skill, replaces, initialPending = false }) => {
  const [mode, setMode] = useState<'view' | 'edit'>('view');
  const [open, setOpen] = useState(true);
  const [busy, setBusy] = useState<'approve' | 'reject' | null>(null);
  const [error, setError] = useState<string | null>(null);
  const rejection = useUndoableDelete(undefined, initialPending);

  const approve = async (edits?: SkillDraft) => {
    setBusy('approve');
    setError(null);
    try {
      await skillsApi.approve(skill.id, edits);
      playSfx('coin');
    } catch (e) {
      setBusy(null);
      return errText(e);
    }
  };
  const reject = () => {
    setError(null);
    rejection.start(() => skillsApi.reject(skill.id));
  };

  if (rejection.gone) return null;
  if (rejection.pending) return <UndoNotice as="li" text={`Rejected “${skill.name}”. Nothing was added.`} onUndo={rejection.undo} />;

  return (
    <li className="p-3 rounded-lg border border-amber-500/25 bg-amber-500/[0.04] space-y-2">
      <div className="flex items-start gap-2">
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          aria-expanded={open}
          aria-controls={`skill-${skill.id}-detail`}
          className="mt-0.5 text-zinc-500 hover:text-zinc-200 rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
          aria-label={open ? `Hide ${skill.name}` : `Show ${skill.name}`}
        >
          {open ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
        </button>
        <div className="min-w-0 flex-1 space-y-1">
          <h5 className="text-xs font-semibold font-mono text-zinc-100 break-all">{skill.name}</h5>
          {skill.description && <p className="text-[11px] text-zinc-400 break-words">{skill.description}</p>}
          <SkillMeta skill={skill} replaces={replaces} />
        </div>
      </div>
      <div id={`skill-${skill.id}-detail`} hidden={!open} className="space-y-2">
        {mode === 'edit' ? (
          <SkillForm
            idPrefix={`skill-${skill.id}`}
            initial={{ name: skill.name, description: skill.description, body: skill.body }}
            submitLabel="Save & approve"
            onSubmit={(d) => approve(d)}
            onCancel={() => setMode('view')}
          />
        ) : (
          <>
            <SkillChanges skill={skill} />
            <div className="flex flex-wrap items-center gap-2">
              <button type="button" onClick={() => approve().then((e) => e && setError(e))} disabled={!!busy} className={goBtn}>
                {busy === 'approve' ? <Loader2 size={11} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Check size={11} aria-hidden="true" />}
                Approve
              </button>
              <button type="button" onClick={() => setMode('edit')} disabled={!!busy} className={quietBtn}>
                <Pencil size={11} aria-hidden="true" /> Edit, then approve
              </button>
              <button type="button" onClick={reject} disabled={!!busy} className={dangerBtn}>
                <X size={11} aria-hidden="true" /> Reject
              </button>
            </div>
          </>
        )}
        {(error ?? rejection.error) && (
          <p className="text-[11px] text-rose-400" role="alert">
            {error ?? `Couldn’t reject it: ${rejection.error}`}
          </p>
        )}
      </div>
    </li>
  );
};

export const ActiveSkill: React.FC<{ skill: Skill; initialPending?: boolean }> = ({ skill, initialPending = false }) => {
  const [editing, setEditing] = useState(false);
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const removal = useUndoableDelete(undefined, initialPending);
  const enabled = skill.status === 'enabled';

  const run = async (fn: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(false);
    }
  };
  const remove = () => {
    setError(null);
    removal.start(() => skillsApi.remove(skill.id));
  };

  if (removal.gone) return null;
  if (removal.pending) return <UndoNotice as="li" text={`Deleted “${skill.name}”. Bots stop using it.`} onUndo={removal.undo} />;

  return (
    <li className={`p-2.5 rounded-lg border border-zinc-800 bg-zinc-950/60 space-y-1.5 ${enabled ? '' : 'opacity-70'}`}>
      <div className="flex items-start gap-2">
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          aria-expanded={open}
          className="min-w-0 flex-1 text-left rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
        >
          <span className="flex items-center gap-1.5 text-xs font-mono font-semibold text-zinc-100 break-all">
            {open ? <ChevronDown size={12} className="shrink-0 text-zinc-500" aria-hidden="true" /> : <ChevronRight size={12} className="shrink-0 text-zinc-500" aria-hidden="true" />}
            {skill.name}
            {skill.kwant_id === null && <Globe2 size={11} className="shrink-0 text-zinc-500" aria-label="Every bot in this space" />}
          </span>
          {skill.description && <span className="block mt-0.5 pl-[18px] text-[11px] text-zinc-400 break-words">{skill.description}</span>}
        </button>
        <button
          type="button"
          role="switch"
          aria-checked={enabled}
          aria-label={`${skill.name}: ${enabled ? 'on' : 'off'}`}
          disabled={busy}
          onClick={() => run(() => skillsApi.patch(skill.id, { enabled: !enabled }))}
          className={`relative shrink-0 mt-0.5 w-8 h-[18px] rounded-full border transition-colors disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500 ${
            enabled ? 'bg-emerald-500/70 border-emerald-400/60' : 'bg-zinc-800 border-zinc-700'
          }`}
        >
          <span className={`absolute top-[2px] w-3 h-3 rounded-full bg-white transition-all ${enabled ? 'left-[16px]' : 'left-[2px]'}`} aria-hidden="true" />
        </button>
      </div>
      {open && !editing && (
        <div className="pl-[18px] space-y-1.5">
          <pre className="max-h-56 overflow-auto rounded-lg bg-zinc-950 border border-zinc-800 p-2.5 text-[11px] leading-relaxed text-zinc-300 whitespace-pre-wrap break-words font-mono">
            {skill.body}
          </pre>
          <div className="flex items-center gap-2">
            <SkillMeta skill={skill} />
            <span className="ml-auto flex items-center gap-1">
              <button type="button" onClick={() => setEditing(true)} className={quietBtn}>
                <Pencil size={11} aria-hidden="true" /> Edit
              </button>
              <button type="button" onClick={remove} disabled={busy} className={dangerBtn}>
                <Trash2 size={11} aria-hidden="true" /> Delete
              </button>
            </span>
          </div>
        </div>
      )}
      {editing && (
        <SkillForm
          idPrefix={`skill-${skill.id}`}
          initial={{ name: skill.name, description: skill.description, body: skill.body }}
          submitLabel="Save"
          onSubmit={async (d) => {
            try {
              await skillsApi.patch(skill.id, d);
              setEditing(false);
            } catch (e) {
              return errText(e);
            }
          }}
          onCancel={() => setEditing(false)}
        />
      )}
      {(error ?? removal.error) && (
        <p className="text-[11px] text-rose-400" role="alert">
          {error ?? `Couldn’t delete it: ${removal.error}`}
        </p>
      )}
    </li>
  );
};

/**
 * An agent's skills (its own plus shared ones): proposals to review, the ones in use with an
 * on/off switch, and the rejected/replaced ones for the record. Live through `skill_updated`.
 */
export const SkillsPanel: React.FC<{ kwantId: string; kwantName: string }> = ({ kwantId, kwantName }) => {
  const [skills, setSkills] = useState<Skill[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [writing, setWriting] = useState(false);
  const [showHistory, setShowHistory] = useState(false);

  const reload = useCallback(async () => {
    try {
      setSkills(await skillsApi.list({ kwant_id: kwantId }));
      setLoadError(null);
    } catch (e) {
      setLoadError(errText(e));
    }
  }, [kwantId]);

  useEffect(() => {
    setSkills(null);
    reload();
    return onContinuity('skill_updated', (ev) => setSkills((prev) => (prev ? applySkillEvent(prev, ev, kwantId) : prev)));
  }, [kwantId, reload]);

  const groups = skills ? groupSkills(skills) : null;
  const byId = (id: string | null) => (id ? skills?.find((s) => s.id === id) : undefined);

  return (
    <div className="space-y-3">
      <p className="text-[11px] text-zinc-500 leading-relaxed">
        Written steps {kwantName} follows when a request fits one. {kwantName} can propose new ones; nothing is used until you approve it. Skills
        are instructions only and never give a bot new tools.
      </p>

      {writing ? (
        <div className="p-3 rounded-lg border border-zinc-800 bg-zinc-950/60">
          <SkillForm
            idPrefix="skill-new"
            initial={{ name: '', description: '', body: '' }}
            submitLabel="Add skill"
            onSubmit={async (d) => {
              try {
                await skillsApi.create({ ...d, kwant_id: kwantId });
                playSfx('coin');
                setWriting(false);
              } catch (e) {
                return errText(e);
              }
            }}
            onCancel={() => setWriting(false)}
          />
        </div>
      ) : (
        <button type="button" onClick={() => setWriting(true)} className={quietBtn}>
          <Plus size={11} aria-hidden="true" /> Write a skill
        </button>
      )}

      {loadError && (
        <div className="flex items-center justify-between gap-2 bg-rose-500/10 border border-rose-500/20 text-rose-300 px-3 py-2 rounded-lg text-xs" role="alert">
          <span>Couldn't load skills: {loadError}</span>
          <button type="button" onClick={reload} className={quietBtn}>
            <RefreshCw size={12} aria-hidden="true" /> Retry
          </button>
        </div>
      )}
      {!skills && !loadError && (
        <p className="flex items-center gap-2 text-xs text-zinc-400" role="status">
          <Loader2 size={14} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Loading skills…
        </p>
      )}

      {groups && (
        <>
          {groups.pending.length > 0 && (
            <section aria-labelledby="skills-pending" className="space-y-1.5">
              <h4 id="skills-pending" className="flex items-center gap-1.5 text-xs font-medium text-amber-200">
                To review
                <span className="px-1.5 rounded-full bg-amber-500/20 text-amber-200 text-[10px]">{groups.pending.length}</span>
              </h4>
              <ul className="space-y-2">
                {groups.pending.map((s) => (
                  <PendingSkill key={s.id} skill={s} replaces={byId(s.replaces_id)} />
                ))}
              </ul>
            </section>
          )}

          <section aria-labelledby="skills-active" className="space-y-1.5">
            <h4 id="skills-active" className="text-xs font-medium text-zinc-300">
              In use
            </h4>
            {groups.active.length === 0 ? (
              <p className="text-xs text-zinc-500 border border-dashed border-zinc-800 rounded-lg p-4 text-center">
                No skills yet. When {kwantName} works out a procedure worth repeating, ask it to propose a skill, or write one yourself.
              </p>
            ) : (
              <ul className="space-y-1.5">
                {groups.active.map((s) => (
                  <ActiveSkill key={s.id} skill={s} />
                ))}
              </ul>
            )}
          </section>

          {groups.history.length > 0 && (
            <section className="space-y-1.5">
              <button
                type="button"
                onClick={() => setShowHistory((v) => !v)}
                aria-expanded={showHistory}
                className="flex items-center gap-1 text-[11px] text-zinc-500 hover:text-zinc-300 rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
              >
                {showHistory ? <ChevronDown size={12} aria-hidden="true" /> : <ChevronRight size={12} aria-hidden="true" />}
                Rejected and replaced ({groups.history.length})
              </button>
              {showHistory && (
                <ul className="space-y-1">
                  {groups.history.map((s) => (
                    <li key={s.id} className="flex items-center gap-2 px-2.5 py-1.5 rounded-lg border border-zinc-800/70 text-[11px] text-zinc-500">
                      <span className="font-mono text-zinc-400 break-all">{s.name}</span>
                      <span>{s.status === 'rejected' ? 'rejected' : 'replaced'}</span>
                      <span className="ml-auto shrink-0">{formatWhen(s.reviewed_at ?? s.updated_at)}</span>
                    </li>
                  ))}
                </ul>
              )}
            </section>
          )}
        </>
      )}
    </div>
  );
};
