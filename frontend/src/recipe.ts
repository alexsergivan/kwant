// The recipe Smart Results card (0.7.6, next-076/recipe-result/design.md): scaling a leading amount,
// the ticks and servings kept per message on this device, and the two things the card copies.
// Every number on the card is the reply's, or the reply's amount times shown / serves (§3). Nothing else
// is computed.

import { useCallback, useEffect, useState } from 'react';
import type { RecipeBlock, RecipeIngredient } from './smartResults';
import { plainText } from './smartResults';

// ---------------------------------------------------------------------------------------------------
// Scaling (design §3; the reference is scaleQty() in design/recipe.html)

const GLYPH: Record<string, number> = { '¼': 1 / 4, '½': 1 / 2, '¾': 3 / 4, '⅓': 1 / 3, '⅔': 2 / 3, '⅛': 1 / 8 };
const NUM = '(?:\\d{1,3}(?:,\\d{3})+(?![\\d.,])|\\d+\\s+\\d+\\/\\d+|\\d+\\/\\d+|\\d+(?:[.,]\\d+)?(?:\\s*[¼½¾⅓⅔⅛])?|[¼½¾⅓⅔⅛])';
const LEAD = new RegExp(`^(${NUM})(?:\\s*[–-]\\s*(${NUM}))?`);

/** The value of one amount as written: "2", "1.5", "1,5", "1/2", "1 1/2", "½", "1½". */
export function amountValue(s: string): number {
  const t = s.trim();
  let m: RegExpMatchArray | null;
  // "1,000": a comma before exactly three digits is a thousands separator (contract §1); "1,5" is 1.5.
  if (/^\d{1,3}(?:,\d{3})+$/.test(t)) return +t.replace(/,/g, '');
  if ((m = t.match(/^(\d+)\s+(\d+)\/(\d+)$/))) return +m[1] + +m[2] / +m[3];
  if ((m = t.match(/^(\d+)\/(\d+)$/))) return +m[1] / +m[2];
  if ((m = t.match(/^(\d+(?:[.,]\d+)?)\s*([¼½¾⅓⅔⅛])$/))) return +m[1].replace(',', '.') + GLYPH[m[2]];
  if (GLYPH[t] !== undefined) return GLYPH[t];
  return +t.replace(',', '.');
}

/**
 * A scaled amount written for a cook: 10 or more is a whole number; under 10 it snaps to ¼ ⅓ ½ ⅔ ¾ (and ⅛
 * below 1) when within 0.07, written as a whole number plus a glyph; otherwise one decimal with a dot.
 */
export function formatAmount(v: number): string {
  if (v >= 10) return String(Math.round(v));
  const w = Math.floor(v);
  const f = v - w;
  const C: [number, string][] = [[0, ''], ...(w === 0 ? ([[1 / 8, '⅛']] as [number, string][]) : []), [1 / 4, '¼'], [1 / 3, '⅓'], [1 / 2, '½'], [2 / 3, '⅔'], [3 / 4, '¾'], [1, '']];
  let best = C[0];
  for (const c of C) if (Math.abs(f - c[0]) < Math.abs(f - best[0])) best = c;
  if (Math.abs(f - best[0]) > 0.07) return (Math.round(v * 10) / 10).toString();
  const whole = best[0] === 1 ? w + 1 : w;
  return (whole ? String(whole) : '') + best[1] || '0';
}

const PLURAL: Record<string, string> = {
  cup: 'cups', clove: 'cloves', can: 'cans', onion: 'onions', lemon: 'lemons', egg: 'eggs', tablespoon: 'tablespoons',
  teaspoon: 'teaspoons', slice: 'slices', carrot: 'carrots', potato: 'potatoes', tomato: 'tomatoes', stick: 'sticks', sprig: 'sprigs',
};
const SINGULAR: Record<string, string> = Object.fromEntries(Object.entries(PLURAL).map(([a, b]) => [b, a]));

/**
 * An ingredient's leading amount at factor k. Only a leading number, fraction or range changes (both
 * ends of a range); the word right after it flips singular/plural from a fixed list; everything else
 * stays exactly as written. `scaled` is false when nothing changed hands (k = 1, or nothing to scale).
 */
export function scaleQty(qty: string, k: number): { qty: string; scaled: boolean } {
  const m = qty.match(LEAD);
  if (!m || k === 1) return { qty, scaled: false };
  const a = amountValue(m[1]) * k;
  const b = m[2] ? amountValue(m[2]) * k : null;
  if (!Number.isFinite(a) || (b !== null && !Number.isFinite(b))) return { qty, scaled: false };
  const top = b ?? a;
  const rest = qty.slice(m[0].length).replace(/^(\s+)([A-Za-z]+)/, (all, sp: string, w: string) => {
    // Keep the reply's case: "Cups" stays capitalised.
    const lw = w.toLowerCase();
    const to = top > 1 ? PLURAL[lw] : SINGULAR[lw];
    if (!to) return all;
    return sp + (w[0] === w[0].toUpperCase() ? to[0].toUpperCase() + to.slice(1) : to);
  });
  return { qty: formatAmount(a) + (b !== null ? '–' + formatAmount(b) : '') + rest, scaled: true };
}

/** Can this amount scale at all? (It starts with a number, fraction or glyph.) */
export const canScale = (qty: string | null) => !!qty && LEAD.test(qty);

/** qty + text is the reply's full line (§11, contract §1): "1 onion" + ", diced"; "2 cups" + "flour". */
export const joinLine = (qty: string, text: string) => (text.startsWith(',') ? qty + text : `${qty} ${text}`);

export const servesRange = (serves: number) => ({ min: 1, max: Math.max(24, serves) });

/** Does the Serves chip step? Only with `serves` and at least one amount that can scale (design §2). */
export const isScalable = (block: RecipeBlock) => !!block.serves && block.ingredients.some((i) => canScale(i.qty));

/** The servings the card shows: the chosen number while it can scale, else the recipe's own. */
export const shownServes = (block: RecipeBlock, chosen: number | null) => (isScalable(block) ? chosen ?? block.serves ?? 1 : block.serves ?? 1);

/** The ingredient line at the shown servings, as plain text. */
export function ingredientLine(ing: RecipeIngredient, k: number): string {
  const text = plainText(ing.text);
  if (!ing.qty) return text;
  return joinLine(scaleQty(ing.qty, k).qty, text);
}

/** Ungrouped first (no label), then each group in the order the reply first used it. */
export function groupIngredients(block: RecipeBlock): { group: string | null; items: RecipeIngredient[] }[] {
  const groups: { group: string | null; items: RecipeIngredient[] }[] = [];
  const loose = block.ingredients.filter((i) => !i.group);
  if (loose.length) groups.push({ group: null, items: loose });
  for (const ing of block.ingredients) {
    if (!ing.group) continue;
    let g = groups.find((x) => x.group === ing.group);
    if (!g) groups.push((g = { group: ing.group, items: [] }));
    g.items.push(ing);
  }
  return groups;
}

/** The scale factor shown / serves: 1 without `serves`. */
export const factor = (block: RecipeBlock, shown: number) => (block.serves ? shown / block.serves : 1);

/** "Prep 15 min · Cook 35 min" with "Serves 6" first when the recipe says how many it serves. */
export function headerLine(block: RecipeBlock, shown: number): string {
  const parts = [...(block.serves ? [`Serves ${shown}`] : []), ...block.times.map((t) => `${t.label} ${t.text}`)];
  return parts.join(' · ');
}

/** Copy recipe (§6): markdown at the shown servings, no ticks. */
export function recipeMarkdown(block: RecipeBlock, shown: number): string {
  const k = factor(block, shown);
  const out: string[] = [`## ${block.title ?? 'Recipe'}`];
  const head = headerLine(block, shown);
  if (head) out.push(head);
  out.push('', '### Ingredients');
  groupIngredients(block).forEach(({ group, items }, gi) => {
    if (group) out.push(...(gi > 0 ? [''] : []), `**${group}**`);
    for (const ing of items) out.push(`- ${ingredientLine(ing, k)}`);
  });
  out.push('', '### Method');
  block.steps.forEach((s, i) => out.push(`${i + 1}. ${plainText(s.text)}`));
  if (block.notes.length) {
    out.push('', '### Notes');
    for (const n of block.notes) out.push(`- ${plainText(n)}`);
  }
  return out.join('\n') + '\n';
}

/** The ingredients still to get: unticked, in the card's order. */
export function shoppingItems(block: RecipeBlock, have: Set<string>): RecipeIngredient[] {
  return groupIngredients(block).flatMap((g) => g.items).filter((i) => !have.has(i.id));
}

/** Copy shopping list (§6): the unticked ingredients, scaled, no groups. "(serves 6)" only with serves. */
export function shoppingListText(block: RecipeBlock, have: Set<string>, shown: number): string {
  const k = factor(block, shown);
  const head = `Shopping list: ${block.title ?? 'Recipe'}${block.serves ? ` (serves ${shown})` : ''}`;
  return [head, ...shoppingItems(block, have).map((i) => `- ${ingredientLine(i, k)}`)].join('\n') + '\n';
}

// ---------------------------------------------------------------------------------------------------
// State: one key per block, kept on this device (§7)

export interface RecipeState {
  /** The servings shown; the recipe's own number when nothing was chosen. */
  serves: number | null;
  have: Set<string>;
  done: Set<string>;
}

export const recipeStorageKey = (messageId: string, blockId: string) => `kwnt:recipe:${messageId}:${blockId}`;
export const RECIPE_CHANGE_EVENT = 'kwnt:recipe-change';

/** What this device kept for the block, ids checked against the block; the reply's state when nothing is kept. */
export function readRecipeState(messageId: string, block: RecipeBlock): RecipeState {
  const fresh: RecipeState = { serves: block.serves, have: new Set(), done: new Set() };
  try {
    const stored = localStorage.getItem(recipeStorageKey(messageId, block.id));
    if (stored === null) return fresh;
    const raw = JSON.parse(stored) as { serves?: unknown; have?: unknown; done?: unknown };
    const ings = new Set(block.ingredients.map((i) => i.id));
    const steps = new Set(block.steps.map((s) => s.id));
    const ids = (v: unknown, ok: Set<string>) => new Set(Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string' && ok.has(x)) : []);
    const range = block.serves ? servesRange(block.serves) : null;
    const serves =
      range && typeof raw.serves === 'number' && Number.isInteger(raw.serves) && raw.serves >= range.min && raw.serves <= range.max ? raw.serves : block.serves;
    return { serves, have: ids(raw.have, ings), done: ids(raw.done, steps) };
  } catch {
    return fresh;
  }
}

export function saveRecipeState(messageId: string, block: RecipeBlock, s: RecipeState) {
  const key = recipeStorageKey(messageId, block.id);
  try {
    if (s.have.size === 0 && s.done.size === 0 && s.serves === block.serves) localStorage.removeItem(key);
    else {
      const body: { serves?: number; have: string[]; done: string[] } = { have: [...s.have], done: [...s.done] };
      if (block.serves && s.serves !== null && s.serves !== block.serves) body.serves = s.serves;
      localStorage.setItem(key, JSON.stringify(body));
    }
  } catch {
    // Private mode or a full store: the ticks just don't outlast the page.
  }
  // The card and its footer (in the action row) share this state.
  window.dispatchEvent(new CustomEvent(RECIPE_CHANGE_EVENT, { detail: { key } }));
}

/**
 * The block's ticks and servings, shared by the card and the footer's shopping-list count. Backed by the
 * one storage key plus a window event, so both update together (and so do two views of one message).
 */
export function useRecipeState(messageId: string, block: RecipeBlock) {
  const [state, setState] = useState<RecipeState>(() => readRecipeState(messageId, block));
  const key = recipeStorageKey(messageId, block.id);
  useEffect(() => {
    setState(readRecipeState(messageId, block));
    const on = (e: Event) => {
      if ((e as CustomEvent<{ key?: string }>).detail?.key === key) setState(readRecipeState(messageId, block));
    };
    window.addEventListener(RECIPE_CHANGE_EVENT, on);
    return () => window.removeEventListener(RECIPE_CHANGE_EVENT, on);
    // The block is checked again each render; its id and the message decide the key.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);

  const update = useCallback(
    (fn: (s: RecipeState) => RecipeState) => {
      const next = fn(readRecipeState(messageId, block));
      setState(next);
      saveRecipeState(messageId, block, next);
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [key],
  );
  const toggle = (which: 'have' | 'done', id: string) =>
    update((s) => {
      const set = new Set(s[which]);
      if (set.has(id)) set.delete(id);
      else set.add(id);
      return { ...s, [which]: set };
    });
  return {
    state,
    toggleHave: (id: string) => toggle('have', id),
    toggleDone: (id: string) => toggle('done', id),
    setServes: (n: number) => update((s) => ({ ...s, serves: n })),
  };
}
