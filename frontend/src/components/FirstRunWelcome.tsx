import React from 'react';
import { KeyRound } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';

export interface WelcomeBot {
  id: string;
  name: string;
  title?: string;
  avatar_sprite?: string;
}

interface Props {
  bots: WelcomeBot[];
  /** "Sign in with Claude" when Claude is installed but signed out; otherwise "Add an account". */
  actionLabel: string;
  onAction: () => void;
}

/** Shown in an empty chat until an account works: what kwant is, who the bots are, one next step. */
export const FirstRunWelcome: React.FC<Props> = ({ bots, actionLabel, onAction }) => (
  <section aria-labelledby="kw-welcome-title" data-first-run-welcome="" className="mx-auto w-full max-w-lg pt-6 sm:pt-16 text-center">
    <div className="flex justify-center -space-x-2 mb-4" aria-hidden="true">
      {bots.slice(0, 5).map((b) => (
        <span key={b.id} className="rounded-full ring-4 ring-zinc-950">
          <ModernAvatar role={b.avatar_sprite} name={b.name} size={44} />
        </span>
      ))}
    </div>
    <h2 id="kw-welcome-title" className="text-lg font-semibold text-zinc-100">
      Welcome to kwant bots
    </h2>
    <p className="mt-1.5 text-sm text-zinc-400">
      These are your bots. Ask anything here and Laya picks who answers.
    </p>
    {bots.length > 0 && (
      <ul className="mt-4 flex flex-wrap justify-center gap-x-4 gap-y-1 text-xs text-zinc-400">
        {bots.slice(0, 5).map((b) => (
          <li key={b.id}>
            <span className="font-medium text-zinc-200">{b.name}</span>
            {b.title && <span className="text-zinc-500"> · {b.title}</span>}
          </li>
        ))}
      </ul>
    )}
    <button
      type="button"
      onClick={onAction}
      className="mt-6 inline-flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-medium text-white bg-violet-600 hover:bg-violet-500 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-violet-300"
    >
      <KeyRound size={15} aria-hidden="true" />
      {actionLabel}
    </button>
    <p className="mt-2 text-[11px] text-zinc-500">Sign in with Claude or ChatGPT, or paste an API key. Your bots can answer once one works.</p>
  </section>
);
