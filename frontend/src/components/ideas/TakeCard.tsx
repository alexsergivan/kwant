import React from 'react';
import { Archive, ArrowRight, Check, ChevronRight, Pencil, RefreshCw, ShieldCheck, SquareKanban, TriangleAlert } from 'lucide-react';
import { EFFORT_WORD, type Idea, type IdeaTake, type PickResult, doItHint, readOnlyLine, since } from '../../ideas';
import { OwnerAvatar } from '../MissionBoardView';
import { Sticker } from '../Sticker';
import { BotFace, EditIdea, Effort, IdeaMenu, IdeaText, InfoLine, Spinner, VerdictPill, btn, focusRing } from './parts';

export type PickState = { kind: 'busy' } | { kind: 'done'; result: PickResult } | { kind: 'error' } | null;
export type TakeBusy = 'look' | 'edit' | null;

export interface TakeCardProps {
  idea: Idea & { take: IdeaTake };
  /** The name for `kwant_id`: who wrote the take. */
  authorName: string | null;
  /** The role of `take.bot_id`, when the space knows it. */
  suggestedRole?: string | null;
  botNames: string[];
  now: number;
  phone?: boolean;
  pick?: PickState;
  busy?: TakeBusy;
  notice?: { tone: 'sky' | 'rose'; text: string } | null;
  editing?: boolean;
  editError?: string | null;
  onDoIt?: () => void;
  onLater?: () => void;
  onEdit?: () => void;
  onSaveEdit?: (text: string) => void;
  onCancelEdit?: () => void;
  onLook?: () => void;
  onDelete?: () => void;
  onOpenMission?: (missionId: string) => void;
  /** Focus lands here after Do it. */
  openRef?: React.Ref<HTMLButtonElement>;
  firstRef?: React.Ref<HTMLButtonElement>;
}

const Label: React.FC<{ children: React.ReactNode; id?: string }> = ({ children, id }) => (
  <h4 id={id} className="text-[11px] font-semibold uppercase tracking-[0.06em] text-[#8d87b8]">
    {children}
  </h4>
);

const None = () => <p className="text-[13px] text-[#8d87b8]">None spotted</p>;

const ListBlock: React.FC<{ title: string; items: string[]; icon: 'risk' | 'question'; phone: boolean }> = ({ title, items, icon, phone }) => {
  const list = items.length ? (
    <ul className="flex flex-col gap-1.5">
      {items.map((t, i) => (
        <li key={i} className="flex gap-2 text-[13px] leading-5 text-[#ecebfa] min-w-0">
          {icon === 'risk' ? (
            <TriangleAlert size={14} aria-hidden="true" className="mt-[3px] shrink-0 text-amber-300" />
          ) : (
            <span aria-hidden="true" className="mt-[1px] shrink-0 w-[14px] text-center font-semibold text-[#b8adff]">
              ?
            </span>
          )}
          <span className="min-w-0 break-words">{t}</span>
        </li>
      ))}
    </ul>
  ) : (
    <None />
  );
  const head = `${title} · ${items.length}`;
  if (phone)
    return (
      <details className="rounded-lg border border-white/[0.08] bg-white/[0.02]" data-fold={icon}>
        <summary className={`min-h-[44px] flex items-center gap-2 px-3 cursor-pointer text-[13px] font-medium text-[#ecebfa] ${focusRing}`}>
          <ChevronRight size={14} aria-hidden="true" className="kw-details-chevron shrink-0 text-[#8d87b8]" />
          {head}
        </summary>
        <div className="px-3 pb-3">{list}</div>
      </details>
    );
  return (
    <section className="flex flex-col gap-2" data-block={icon}>
      <Label>{head}</Label>
      {list}
    </section>
  );
};

/** The done panel after Do it: never a "started" state (spec §4.2). */
const PickedPanel: React.FC<{ result: PickResult; bot: string | null; onOpen: () => void; openRef?: React.Ref<HTMLButtonElement> }> = ({ result, bot, onOpen, openRef }) => (
  <div className="flex flex-col gap-2 px-4 py-3 border-t border-white/[0.08] bg-[rgb(10_7_25/.35)]" data-picked-panel>
    <div className="flex flex-wrap items-center gap-2">
      <Sticker tone="holo" size="sm" icon={<Check aria-hidden="true" />}>
        Picked
      </Sticker>
      <span className="text-[13px] text-[#ecebfa] min-w-0">
        {bot ? `Draft mission made for ${bot}` : 'Draft mission made'}
        {result.tasks !== null ? ` · ${result.tasks} ${result.tasks === 1 ? 'task' : 'tasks'}.` : '.'}
      </span>
      {result.mission_status === 'draft' && (
        <span className="kw-chip kw-chip-warn" data-waiting-chip>
          Waiting for your OK
        </span>
      )}
      <button ref={openRef} type="button" onClick={onOpen} className={`${btn.secondary} sm:ml-auto`} data-action="open-board">
        Open on board
        <ArrowRight size={14} aria-hidden="true" />
      </button>
    </div>
    <p className="text-[12px] text-[#a39dcc]">Approve it on the board to start. Until then nothing runs.</p>
  </div>
);

/** A ready take, in full (spec §4). Desktop shows it inline; phone shows it on its own screen. */
export const TakeCard: React.FC<TakeCardProps> = (p) => {
  const { idea, authorName, suggestedRole, botNames, now, phone = false, pick = null, busy = null, notice, editing = false } = p;
  const take = idea.take;
  const picking = pick?.kind === 'busy';
  const done = pick?.kind === 'done' ? pick.result : null;
  const locked = picking || busy !== null || !!done;
  const ago = since(idea.updated_at, now);
  const titleId = `take-${idea.id}-title`;
  const steps = take.plan;
  return (
    <article aria-labelledby={titleId} className="rounded-2xl border border-[rgb(157_143_255/.28)] bg-[#1a1538] min-w-0 overflow-hidden kw-jar-arrive" data-take-card={idea.id} data-status={idea.status}>
      <div className="flex flex-col gap-3 px-4 pt-4 pb-3">
        <div className="flex items-start gap-2">
          <div className="min-w-0 flex-1 flex flex-col gap-1">
            <span id={titleId} className="text-[11px] font-semibold uppercase tracking-[0.06em] text-[#8d87b8]">
              Your idea
            </span>
            {!editing && <IdeaText text={idea.text} botNames={botNames} className="text-[15px] leading-[22px] font-medium text-[#f7f6ff]" />}
          </div>
          {!editing && !done && (
            <IdeaMenu
              label="More for this idea"
              disabled={locked}
              items={[
                { key: 'edit', label: 'Edit', onSelect: () => p.onEdit?.() },
                { key: 'delete', label: 'Delete', onSelect: () => p.onDelete?.(), danger: true },
              ]}
            />
          )}
        </div>
        {editing && <EditIdea initial={idea.text} hasTake busy={busy === 'edit'} error={p.editError ?? null} onSave={(t) => p.onSaveEdit?.(t)} onCancel={() => p.onCancelEdit?.()} />}
        <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
          <VerdictPill verdict={take.verdict} />
          <Effort effort={take.effort} />
          <span className="flex items-center gap-1.5 text-[12px] text-[#8d87b8] sm:ml-auto min-w-0">
            {idea.kwant_id && <OwnerAvatar kwantId={idea.kwant_id} name={authorName ?? 'Bot'} size={20} />}
            <span className="min-w-0 truncate">
              {idea.kwant_id && authorName ? `${authorName}’s take` : 'The take'}
              {ago ? ` · ${ago === 'just now' ? 'just now' : `${ago} ago`}` : ''}
            </span>
          </span>
        </div>
        {take.title && <h3 className="text-[14px] leading-5 font-semibold text-[#ecebfa] break-words">{take.title}</h3>}
      </div>
      <div className={`grid gap-4 px-4 pb-4 ${phone ? 'grid-cols-1' : 'grid-cols-1 md:grid-cols-[1.25fr_1fr] md:gap-0'}`}>
        <div className={`flex flex-col gap-4 min-w-0 ${phone ? '' : 'md:pr-5'}`}>
          <section className="flex flex-col gap-1.5">
            <Label>Why it matters</Label>
            {take.why ? <p className="text-[13px] leading-5 text-[#ecebfa] break-words whitespace-pre-line">{take.why}</p> : <None />}
          </section>
          <section className="flex flex-col gap-2" data-plan>
            <Label>The plan · {steps.length} {steps.length === 1 ? 'step' : 'steps'}</Label>
            {steps.length ? (
              <ol className="flex flex-col gap-2.5">
                {steps.map((s, i) => (
                  <li key={i} className="flex gap-2.5 min-w-0">
                    <span aria-hidden="true" className="shrink-0 grid place-items-center w-5 h-5 rounded bg-[#2c2552] font-mono text-[11px] text-[#b8adff]">
                      {i + 1}
                    </span>
                    <div className="min-w-0 flex flex-col gap-0.5">
                      <span className="text-[13px] leading-5 text-[#f7f6ff] break-words">{s.title}</span>
                      {s.done_when && (
                        <span className="text-[12px] leading-4 text-[#a39dcc] break-words" data-done-when>
                          Done when: {s.done_when}
                        </span>
                      )}
                    </div>
                  </li>
                ))}
              </ol>
            ) : (
              <None />
            )}
          </section>
        </div>
        <div className={`flex flex-col gap-4 min-w-0 ${phone ? '' : 'md:pl-5 md:border-l md:border-white/[0.08]'}`}>
          <ListBlock title="Risks" items={take.risks} icon="risk" phone={phone} />
          <ListBlock title="Open questions" items={take.questions} icon="question" phone={phone} />
          <section className="flex flex-col gap-2" data-who>
            <Label>Who should do it</Label>
            {take.bot_id || take.bot ? (
              <div className="flex items-center gap-2.5 rounded-lg border border-white/[0.08] bg-white/[0.03] px-3 py-2 min-w-0">
                <OwnerAvatar kwantId={take.bot_id} name={take.bot ?? 'Bot'} size={32} />
                <div className="min-w-0 flex flex-col">
                  <span className="text-[13px] font-medium text-[#f7f6ff] truncate">{take.bot ?? 'A bot'}</span>
                  {suggestedRole && <span className="text-[12px] text-[#a39dcc] truncate">{suggestedRole}</span>}
                </div>
              </div>
            ) : (
              <p className="text-[13px] text-[#8d87b8]">No bot suggested. You’ll pick one on the board.</p>
            )}
          </section>
        </div>
      </div>
      {notice && (
        <div className="px-4 pb-3">
          <InfoLine tone={notice.tone} className="!text-[12px]">
            {notice.text}
          </InfoLine>
        </div>
      )}
      {done ? (
        <PickedPanel result={done} bot={take.bot} onOpen={() => p.onOpenMission?.(done.mission_id)} openRef={p.openRef} />
      ) : (
        <div className={`flex flex-col gap-2 px-4 py-3 border-t border-white/[0.08] bg-[rgb(10_7_25/.35)] ${phone ? 'sticky bottom-0 z-10 bg-[#130f2a]' : ''}`} data-action-bar>
          {pick?.kind === 'error' && idea.status === 'ready' && <InfoLine tone="rose">Couldn’t make the draft. Nothing was created. Try again.</InfoLine>}
          <div className={phone ? 'flex flex-col gap-2' : 'flex flex-wrap items-center gap-2'}>
            <button ref={p.firstRef} type="button" onClick={p.onDoIt} disabled={locked} className={`${btn.primary} ${phone ? '!min-h-[48px] w-full text-[14px]' : ''}`} data-action="do-it">
              {picking ? <Spinner /> : null}
              {picking ? 'Making the draft…' : phone ? 'Do it · make a draft' : 'Do it'}
              {!picking && <ArrowRight size={14} aria-hidden="true" />}
            </button>
            {phone ? (
              <p className="text-center text-[12px] text-[#a39dcc]">Goes to the Mission board for your OK. Nothing starts yet.</p>
            ) : (
              <p className="flex items-center gap-1.5 text-[12px] text-[#a39dcc] min-w-0 flex-1" data-do-it-hint>
                <SquareKanban size={13} aria-hidden="true" className="shrink-0" />
                <span className="min-w-0">
                  Makes a <strong className="font-semibold text-[#ecebfa]">draft</strong> mission{doItHint(take.bot).startsWith('.') ? '' : ' '}
                  {doItHint(take.bot)}
                </span>
              </p>
            )}
            <div className={phone ? 'grid grid-cols-3 gap-2' : 'flex items-center gap-1.5'}>
              <button type="button" onClick={p.onLater} disabled={locked} className={btn.secondary} data-action="later">
                <Archive size={14} aria-hidden="true" />
                Later
              </button>
              <button type="button" onClick={p.onEdit} disabled={locked || editing} className={btn.secondary} data-action="edit">
                <Pencil size={14} aria-hidden="true" />
                Edit
              </button>
              <button type="button" onClick={p.onLook} disabled={locked} className={btn.quiet} data-action="look">
                {busy === 'look' ? <Spinner /> : <RefreshCw size={14} aria-hidden="true" />}
                {busy === 'look' ? 'Asking…' : 'Look again'}
              </button>
            </div>
          </div>
        </div>
      )}
      <p className="flex items-center gap-1.5 px-4 py-2 border-t border-white/[0.06] text-[11px] text-[#8d87b8]" data-read-only>
        <ShieldCheck size={12} aria-hidden="true" className="shrink-0" />
        {readOnlyLine(authorName)}
      </p>
    </article>
  );
};

/** A ready take, folded to one row (desktop: the takes after the first; phone: every take). */
export const TakeRow: React.FC<{ idea: Idea & { take: IdeaTake }; isNew: boolean; botNames: string[]; authorName: string | null; onOpen: () => void; buttonRef?: React.Ref<HTMLButtonElement> }> = ({
  idea,
  isNew,
  authorName,
  onOpen,
  buttonRef,
}) => (
  <div className="kw-slip flex items-center gap-2.5 rounded-xl border border-[rgb(157_143_255/.28)] bg-[#1d1840] pl-3 pr-2 py-2 min-w-0 kw-jar-arrive" data-take-row={idea.id}>
    <VerdictPill verdict={idea.take.verdict} small />
    <span className="min-w-0 flex-1 flex flex-col">
      <span className="text-[14px] leading-5 text-[#f7f6ff] truncate">{idea.take.title || idea.text}</span>
      <span className="text-[12px] leading-4 text-[#8d87b8] truncate">
        <span className="inline-flex items-center gap-1.5 align-middle">
          {idea.kwant_id && <BotFace kwantId={idea.kwant_id} name={authorName ?? 'Bot'} size={14} />}
          {authorName ? `${authorName}’s take` : 'A take'}
        </span>
        <span className="sr-only">, effort {EFFORT_WORD[idea.take.effort]}</span>
      </span>
    </span>
    {isNew && (
      <span className="kw-chip !text-[11px] text-[#d6d0ff] border-[rgb(157_143_255/.45)] bg-[rgb(157_143_255/.14)]" data-new-chip>
        New
      </span>
    )}
    <span className="hidden sm:inline-flex">
      <Effort effort={idea.take.effort} label={false} word={false} />
    </span>
    <button ref={buttonRef} type="button" onClick={onOpen} className={btn.secondary} data-action="open" aria-label={`Open the take on: ${idea.take.title || idea.text}`}>
      Open
      <ChevronRight size={14} aria-hidden="true" />
    </button>
  </div>
);
