import React, { useEffect, useRef, useState } from 'react';
import { Check, Copy, ShoppingBasket } from 'lucide-react';
import type { RecipeBlock } from '../../smartResults';
import { recipeMarkdown, shoppingItems, shoppingListText, shownServes, useRecipeState } from '../../recipe';
import { playSfx } from '../../audio/chiptune';
import { actionClass } from './SmartActions';

// Kept apart from Recipe.tsx so the registry (and its tests) never load the sound module.

/** A copy chip that says "Copied" (emerald check) for 2 s, with its own words for screen readers. */
function useCopied() {
  const [said, setSaid] = useState<{ ok: boolean; shown: string; sr: string } | null>(null);
  const timer = useRef<number>();
  useEffect(() => () => window.clearTimeout(timer.current), []);
  const copy = async (text: string, shown: string, sr: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setSaid({ ok: true, shown, sr });
      playSfx('click');
    } catch {
      setSaid({ ok: false, shown: 'Copy failed', sr: 'Could not copy' });
    }
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setSaid(null), 2000);
  };
  return { said, copy };
}

/**
 * The recipe's chips in the message's action row (design §6): Copy recipe and Copy shopping list, in
 * place of Copy data. The shopping list is the unticked ingredients at the shown servings; its count
 * follows the ticks live.
 */
export const RecipeActions: React.FC<{ block: RecipeBlock; messageId: string }> = ({ block, messageId }) => {
  const { state } = useRecipeState(messageId, block);
  const shown = shownServes(block, state.serves);
  const left = shoppingItems(block, state.have).length;
  const recipe = useCopied();
  const list = useCopied();
  const forServes = block.serves ? ` for ${shown} ${shown === 1 ? 'serving' : 'servings'}` : '';
  const items = (n: number) => `${n} ${n === 1 ? 'item' : 'items'}`;
  const none = left === 0;
  return (
    <>
      <button
        type="button"
        data-recipe-copy="recipe"
        onClick={() => recipe.copy(recipeMarkdown(block, shown), 'Copied', `Copied the recipe${forServes}`)}
        className={actionClass}
      >
        {recipe.said?.ok ? <Check size={14} className="text-emerald-400" aria-hidden="true" /> : <Copy size={14} aria-hidden="true" />}
        <span className={recipe.said?.ok ? 'font-medium text-emerald-400' : recipe.said ? 'text-rose-300' : ''}>{recipe.said?.shown ?? 'Copy recipe'}</span>
      </button>
      <button
        type="button"
        data-recipe-copy="shopping"
        aria-disabled={none || undefined}
        aria-label={list.said ? undefined : `Copy shopping list, ${items(left)}`}
        title={none ? 'Everything is ticked.' : undefined}
        onClick={() => !none && list.copy(shoppingListText(block, state.have, shown), `Copied ${items(left)}`, `Copied ${items(left)}${forServes}`)}
        className={`${actionClass} ${none ? 'cursor-not-allowed opacity-45' : ''}`}
      >
        {list.said?.ok ? <Check size={14} className="text-emerald-400" aria-hidden="true" /> : <ShoppingBasket size={14} aria-hidden="true" />}
        {list.said ? (
          <span className={list.said.ok ? 'font-medium text-emerald-400' : 'text-rose-300'}>{list.said.shown}</span>
        ) : (
          <>
            <span className="max-sm:hidden">Copy shopping list</span>
            <span className="sm:hidden">Shopping list</span>
            <span className="min-w-[18px] rounded-full bg-zinc-800 px-[5px] text-center text-2xs leading-[18px] tabular-nums text-zinc-200">{left}</span>
          </>
        )}
      </button>
      <span className="sr-only" aria-live="polite">
        {recipe.said?.sr ?? list.said?.sr ?? ''}
      </span>
    </>
  );
};
