// Dev mode visibility (design.md §2): the slim "this space has code" strip above #general, in the
// getting-started banner's slot and style. Turn on opens Settings › General at the Dev mode card (it never
// switches Dev mode on by itself: that changes approvals and the production floor); Not now hides it for
// this space for good. An offer, not news: an <aside> labelled by its title, never announced on load.

import React from 'react';
import { Code } from 'lucide-react';
import type { DevModeState } from '../devMode';
import { foundRepo } from '../devSuggest';
import { useMedia } from '../useMedia';
import { PHONE_QUERY } from '../layout';
import { Btn } from './GetStarted';

export const SUGGEST_SUB = 'Dev mode lets your bots review pull requests, show what landed and guard deploys.';
export const SUGGEST_SUB_PHONE = 'Dev mode lets your bots review PRs and guard deploys.';
/** Said once, out of sight, after Not now. */
export const SUGGEST_HIDDEN = 'Hidden. Dev mode stays in Space settings › General.';

export const DevModeSuggestion: React.FC<{ spaceName: string; state: DevModeState; onOpen: () => void; onDismiss: () => void }> = ({ spaceName, state, onOpen, onDismiss }) => {
  const phone = useMedia(PHONE_QUERY);
  const r = foundRepo(state);
  if (!r) return null;
  const more = state.repos.length - 1;
  const title = `${spaceName} has code: ${r.repo}${more > 0 ? ` and ${more} more` : ''}`;
  return (
    <aside
      aria-labelledby="dv-sug-title"
      data-dev-suggestion
      className={`kw-fade-in border-b border-sky-500/20 bg-sky-500/[.06] ${phone ? 'px-3 py-2.5' : 'flex items-center gap-3 px-6 py-2.5'}`}
    >
      <div className="flex items-center gap-3 min-w-0 flex-1">
        <span className="w-7 h-7 shrink-0 rounded-md bg-sky-500/15 text-sky-300 grid place-items-center">
          <Code size={15} aria-hidden="true" />
        </span>
        <div className="min-w-0 flex-1">
          <p id="dv-sug-title" className="text-xs font-medium text-zinc-100 truncate" title={title}>
            {spaceName} has code: <span className="font-mono">{r.repo}</span>
            {more > 0 ? ` and ${more} more` : ''}
          </p>
          <p className="text-[11.5px] text-zinc-400 truncate">{phone ? SUGGEST_SUB_PHONE : SUGGEST_SUB}</p>
        </div>
      </div>
      <div className={`flex items-center gap-2 ${phone ? 'mt-2 justify-end' : ''}`}>
        <Btn phone={phone} kind="secondary" className={phone ? '' : '!h-8'} onClick={onDismiss} data-dev-suggestion-dismiss>
          Not now
        </Btn>
        <Btn phone={phone} kind="primary" className={phone ? '' : '!h-8'} onClick={onOpen} data-dev-suggestion-open>
          Turn on Dev mode
        </Btn>
      </div>
    </aside>
  );
};
