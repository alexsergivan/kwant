import React, { useEffect, useId, useLayoutEffect, useRef, useState, useSyncExternalStore } from 'react';
import { Check, ChevronDown, ChevronUp, Clock, Minus, Plus, Users } from 'lucide-react';
import type { RecipeBlock, RecipeIngredient, RecipeStep, RecipeTime } from '../../smartResults';
import { inRange, isObj, isText, optText, parseCommon, plainText, uniqueIds } from '../../smartResults';
import { InlineText } from './InlineText';
import { factor, groupIngredients, isScalable, scaleQty, servesRange, shownServes, useRecipeState } from '../../recipe';
import type { RendererProps } from './registry';

/**
 * A recipe (0.7.6): 2–40 ingredients, 2–30 steps, 0–3 times, 0–6 notes, `serves` optional. qty + text is
 * the reply's line (design §11), so the renderer never strips a prefix. Any field off, and the reply
 * shows as text.
 */
export function validRecipe(raw: unknown): RecipeBlock | null {
  const b = parseCommon(raw, 'recipe');
  // Limits as in contract.md §1: the server only writes a recipe as a stretch of the reply.
  if (!b || b.source.kind !== 'markdown_section') return null;
  const serves = b.serves ?? null;
  if (serves !== null && !(typeof serves === 'number' && Number.isInteger(serves) && serves >= 1 && serves <= 24)) return null;

  const rawTimes = b.times ?? [];
  if (!inRange(rawTimes, 0, 3)) return null;
  const times: RecipeTime[] = [];
  for (const t of rawTimes) {
    if (!isObj(t) || !isText(t.label, 40) || !isText(t.text, 60)) return null;
    times.push({ label: t.label, text: t.text });
  }

  if (!inRange(b.ingredients, 2, 40) || !uniqueIds(b.ingredients)) return null;
  const ingredients: RecipeIngredient[] = [];
  for (const it of b.ingredients as Record<string, unknown>[]) {
    const qty = optText(it.qty, 40), group = optText(it.group, 100);
    if (qty === undefined || group === undefined || !isText(it.text, 300) || !/^i\d+$/.test(it.id as string)) return null;
    if (qty !== null && !/^[\d¼½¾⅓⅔⅛]/.test(qty)) return null;
    ingredients.push({ id: it.id as string, text: it.text, qty, group });
  }

  if (!inRange(b.steps, 2, 30) || !uniqueIds(b.steps)) return null;
  const steps: RecipeStep[] = [];
  for (const s of b.steps as Record<string, unknown>[]) {
    if (!isText(s.text, 600) || !/^s\d+$/.test(s.id as string)) return null;
    steps.push({ id: s.id as string, text: s.text });
  }

  const rawNotes = b.notes ?? [];
  if (!inRange(rawNotes, 0, 6) || !rawNotes.every((n) => isText(n, 600))) return null;

  return { id: b.id, type: 'recipe', title: b.title, source: b.source, provenance: b.provenance, serves, times, ingredients, steps, notes: [...(rawNotes as string[])] };
}

const PHONE = '(max-width: 639px)';
const subscribePhone = (cb: () => void) => {
  const mq = typeof window.matchMedia === 'function' ? window.matchMedia(PHONE) : null;
  mq?.addEventListener?.('change', cb);
  return () => mq?.removeEventListener?.('change', cb);
};
/** A viewport under 640 px (design §4): where the ingredients fold. */
export const usePhone = () =>
  useSyncExternalStore(subscribePhone, () => (typeof window.matchMedia === 'function' ? window.matchMedia(PHONE).matches : false), () => false);

/**
 * Two columns need room: the card itself must be at least this wide (design §8, Ada's §11 check). 700
 * puts a 1180 px window with the sidebar at two columns and keeps 1024 at one (F-6).
 */
export const TWO_COLUMNS_MIN = 700;
/** Ingredients fold on a phone when there are more than this many, showing FOLD_SHOW. */
export const FOLD_OVER = 10;
export const FOLD_SHOW = 8;

const openKey = (messageId: string, blockId: string) => `kwnt:recipe-open:${messageId}:${blockId}`;
const readOpen = (key: string) => {
  try {
    return sessionStorage.getItem(key) === '1';
  } catch {
    return false;
  }
};

const chipClass = 'inline-flex h-8 items-center gap-1.5 whitespace-nowrap rounded-full border border-hairline px-3 text-meta text-zinc-200';
const stepBtn =
  'grid h-[30px] w-[30px] max-sm:h-11 max-sm:w-11 place-items-center rounded-full text-zinc-200 hover:bg-white/[0.05] hover:text-zinc-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400 aria-disabled:opacity-40 aria-disabled:cursor-not-allowed';
const boxClass =
  'mt-[3px] h-4 w-4 shrink-0 cursor-pointer rounded accent-sky-500 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400 focus-visible:ring-offset-2 focus-visible:ring-offset-zinc-900';

const SectionHead: React.FC<{ id: string; title: string; children: React.ReactNode }> = ({ id, title, children }) => (
  <div className="mb-1 flex items-baseline justify-between gap-2">
    <h4 id={id} className="text-meta font-semibold text-zinc-200">
      {title}
    </h4>
    {children}
  </div>
);

/** The card: header (name, Serves stepper, times), ingredients and method, notes. Ticks stay on this device. */
export const Recipe: React.FC<RendererProps<RecipeBlock>> = ({ block, messageId }) => {
  const { state, toggleHave, toggleDone, setServes } = useRecipeState(messageId, block);
  const uid = useId();
  const phone = usePhone();
  const card = useRef<HTMLElement>(null);
  const [wide, setWide] = useState(false);
  const [open, setOpen] = useState(() => readOpen(openKey(messageId, block.id)));
  const [popped, setPopped] = useState<string | null>(null);

  // Two columns when the card itself has room, whatever the sidebar takes (no container queries here).
  useLayoutEffect(() => {
    const el = card.current;
    if (!el) return;
    const measure = () => setWide(el.getBoundingClientRect().width >= TWO_COLUMNS_MIN);
    measure();
    if (typeof ResizeObserver !== 'function') return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  useEffect(() => {
    try {
      if (open) sessionStorage.setItem(openKey(messageId, block.id), '1');
      else sessionStorage.removeItem(openKey(messageId, block.id));
    } catch {
      // The open state then lasts for this view only.
    }
  }, [open, messageId, block.id]);

  const scalable = isScalable(block);
  const shown = shownServes(block, state.serves);
  const k = scalable ? factor(block, shown) : 1;
  const scaled = scalable && shown !== block.serves;
  const range = servesRange(block.serves ?? 1);

  const groups = groupIngredients(block);
  const total = block.ingredients.length;
  const folds = phone && total > FOLD_OVER;
  const limit = folds && !open ? FOLD_SHOW : Infinity;
  const have = block.ingredients.filter((i) => state.have.has(i.id)).length;

  const stepsDone = block.steps.filter((s) => state.done.has(s.id)).length;
  const allDone = stepsDone === block.steps.length;
  const next = stepsDone > 0 && !allDone ? block.steps.find((s) => !state.done.has(s.id))?.id : undefined;

  const ingId = `${uid}-ing`;
  const methodId = `${uid}-method`;
  const listId = `${uid}-ings`;
  const hasChips = !!block.serves || block.times.length > 0;

  let seen = 0;
  const title = block.title ?? 'Recipe';

  return (
    <section
      ref={card}
      aria-label={title}
      data-smart-result="recipe"
      data-block-id={block.id}
      data-columns={wide ? 2 : 1}
      className="my-3.5 rounded-xl border border-zinc-700/60 bg-zinc-900/60 p-3.5 shadow-[var(--kws-shadow)] sm:p-5"
    >
      <h3 className="text-title font-semibold text-zinc-100 [overflow-wrap:anywhere]">{title}</h3>
      {hasChips && (
        <div className="mt-2.5 flex flex-wrap items-center gap-2">
          {scalable ? (
            <span
              role="group"
              aria-label="Servings"
              className="inline-flex h-8 items-center rounded-full border border-hairline-strong bg-white/[0.04] pl-3 text-meta text-zinc-400 max-sm:h-11"
            >
              <span className="mr-0.5 inline-flex items-center gap-1.5">
                <Users size={13} aria-hidden="true" />
                Serves
              </span>
              <button
                type="button"
                aria-label="Fewer servings"
                aria-disabled={shown <= range.min}
                onClick={() => shown > range.min && setServes(shown - 1)}
                className={stepBtn}
              >
                <Minus size={14} aria-hidden="true" />
              </button>
              <output aria-live="polite" aria-label={`${shown} ${shown === 1 ? 'serving' : 'servings'}`} className="min-w-[22px] text-center text-[13px] font-semibold tabular-nums text-zinc-100">
                {shown}
              </output>
              <button
                type="button"
                aria-label="More servings"
                aria-disabled={shown >= range.max}
                onClick={() => shown < range.max && setServes(shown + 1)}
                className={stepBtn}
              >
                <Plus size={14} aria-hidden="true" />
              </button>
            </span>
          ) : block.serves ? (
            <span className={chipClass} data-serves-chip>
              <Users size={13} className="text-zinc-400" aria-hidden="true" />
              <span className="text-zinc-400">Serves</span> {block.serves}
            </span>
          ) : null}
          {block.times.map((t, i) => (
            <span key={`${t.label}-${i}`} className={chipClass}>
              {i === 0 && <Clock size={13} className="text-zinc-400" aria-hidden="true" />}
              <span className="text-zinc-400">{t.label}</span> {t.text}
            </span>
          ))}
          {scaled && (
            <button
              type="button"
              onClick={() => block.serves && setServes(block.serves)}
              className="inline-flex min-h-8 items-center rounded px-1 text-meta text-sky-300 underline-offset-[3px] hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400 max-sm:min-h-11"
            >
              Back to {block.serves}
            </button>
          )}
        </div>
      )}

      <div className={`mt-4 grid gap-4 border-t border-hairline pt-3.5 ${wide ? 'grid-cols-[minmax(0,2fr)_minmax(0,3fr)] gap-x-7' : ''}`}>
        <div>
          <SectionHead id={ingId} title="Ingredients">
            <span className="text-meta tabular-nums text-zinc-400">{have ? `${have} of ${total}` : total}</span>
          </SectionHead>
          <div id={listId}>
            {groups.map(({ group, items }, gi) => {
              const visible = items.filter(() => seen++ < limit);
              if (!visible.length) return null;
              const labelId = `${uid}-g${gi}`;
              return (
                <div key={group ?? '_'}>
                  {group && (
                    <div id={labelId} className="mb-0.5 mt-2.5 text-meta font-medium text-zinc-400 [overflow-wrap:anywhere]">
                      {group}
                    </div>
                  )}
                  <ul role="list" aria-labelledby={group ? labelId : ingId}>
                    {visible.map((ing) => {
                      const on = state.have.has(ing.id);
                      const s = ing.qty ? scaleQty(ing.qty, k, plainText(ing.text)) : null;
                      const amountClass = on ? 'font-medium text-zinc-500' : s?.scaled ? 'font-semibold text-sky-200' : 'font-medium text-zinc-100';
                      return (
                        <li key={ing.id}>
                          <label className="-mx-1.5 flex min-h-8 cursor-pointer items-start gap-2.5 rounded-lg px-1.5 py-[5px] hover:bg-white/[0.05] max-sm:min-h-11 max-sm:py-[11px]">
                            <input type="checkbox" checked={on} onChange={() => toggleHave(ing.id)} className={boxClass} />
                            <span
                              className={`min-w-0 text-body [overflow-wrap:anywhere] ${on ? 'text-zinc-500 line-through decoration-zinc-600' : 'text-zinc-300'}`}
                              data-ingredient={ing.id}
                            >
                              {s ? (
                                <>
                                  <span className={`tabular-nums ${amountClass}`} title={s.scaled ? `In the reply: ${ing.qty}` : undefined} data-scaled={s.scaled || undefined}>
                                    {s.qty}
                                  </span>
                                  {/* Colour and the hover title don't reach a screen reader (F-5). */}
                                  {s.scaled && <span className="sr-only"> (was {ing.qty})</span>}
                                  {ing.text.startsWith(',') ? '' : ' '}
                                  <InlineText md={ing.text} />
                                </>
                              ) : (
                                <InlineText md={ing.text} />
                              )}
                            </span>
                          </label>
                        </li>
                      );
                    })}
                  </ul>
                </div>
              );
            })}
          </div>
          {folds && (
            <button
              type="button"
              aria-expanded={open}
              aria-controls={listId}
              onClick={() => setOpen((o) => !o)}
              className="mt-1.5 flex min-h-11 w-full items-center justify-center gap-1.5 rounded-lg border border-hairline text-meta font-medium text-zinc-200 hover:bg-white/[0.05] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400"
            >
              {open ? 'Show fewer' : `Show all ${total}`}
              {open ? <ChevronUp size={14} aria-hidden="true" /> : <ChevronDown size={14} aria-hidden="true" />}
            </button>
          )}
        </div>

        <div className={wide ? 'border-l border-hairline pl-6' : 'border-t border-hairline pt-3.5'}>
          <SectionHead id={methodId} title="Method">
            {allDone ? (
              <span className="inline-flex items-center gap-1 text-meta text-ok">
                <Check size={12} aria-hidden="true" />
                All {block.steps.length} done
              </span>
            ) : (
              <span className="text-meta tabular-nums text-zinc-400">{stepsDone ? `${stepsDone} of ${block.steps.length} done` : `${block.steps.length} steps`}</span>
            )}
          </SectionHead>
          <ol role="list" aria-labelledby={methodId} className="grid gap-0.5">
            {block.steps.map((step, i) => {
              const on = state.done.has(step.id);
              const isNext = step.id === next;
              return (
                <li key={step.id}>
                  <label
                    data-step={step.id}
                    data-state={on ? 'done' : isNext ? 'next' : undefined}
                    className={`relative -mx-2 flex cursor-pointer gap-3 rounded-lg p-2 hover:bg-white/[0.05] has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-sky-400 max-sm:min-h-11 max-sm:py-2.5 ${
                      isNext ? 'bg-sky-500/10 ring-1 ring-inset ring-sky-400/40' : ''
                    }`}
                  >
                    <input
                      type="checkbox"
                      className="sr-only"
                      checked={on}
                      onChange={() => {
                        setPopped(on ? null : step.id);
                        toggleDone(step.id);
                      }}
                    />
                    <span
                      aria-hidden="true"
                      className={`mt-px grid h-[22px] min-w-[22px] shrink-0 place-items-center rounded-full border px-[5px] text-meta font-semibold leading-none tabular-nums ${
                        on
                          ? `border-zinc-700 bg-zinc-800 text-ok ${popped === step.id ? 'motion-safe:animate-[kw-step-done_160ms_var(--kw-ease)_both]' : ''}`
                          : isNext
                            ? 'border-sky-300 bg-sky-300 text-zinc-950'
                            : 'border-zinc-600 text-zinc-300'
                      }`}
                    >
                      {on ? <Check size={12} strokeWidth={3} /> : i + 1}
                    </span>
                    <span className={`min-w-0 text-body [overflow-wrap:anywhere] ${on ? 'text-zinc-500' : isNext ? 'text-zinc-100' : 'text-zinc-300'}`}>
                      <span className="sr-only">Step {i + 1}: </span>
                      <InlineText md={step.text} />
                      {isNext && <span className="sr-only"> (next)</span>}
                    </span>
                  </label>
                </li>
              );
            })}
          </ol>
          <span className="sr-only" aria-live="polite">
            {stepsDone} of {block.steps.length} steps done
          </span>
        </div>
      </div>

      {block.notes.length > 0 && (
        <div className="mt-4 border-t border-hairline pt-3">
          <h4 className="mb-1 text-meta font-semibold text-zinc-400">Notes</h4>
          <ul className="list-disc space-y-1 pl-[18px] text-meta leading-[19px] text-zinc-400 marker:text-zinc-600">
            {block.notes.map((n, i) => (
              <li key={i} className="[overflow-wrap:anywhere]">
                <InlineText md={n} />
              </li>
            ))}
          </ul>
        </div>
      )}
    </section>
  );
};
