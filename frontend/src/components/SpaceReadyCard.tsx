// "Get {space} ready": the card on an empty space (no bots yet). Step 2 is "Add your bots": say what
// they should do and get a team suggested (team-from-words brief, "Entry points"), or start from a
// ready-made team, or add one bot. On a server without the team routes, step 2 is the plain New bot.
import React from 'react';
import { Bot, Check, KeyRound } from 'lucide-react';
import { useReadyTeams } from './ReadyTeams';
import { TeamNeedBox } from './TeamNeedBox';

const FOCUS = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-300';

export interface SpaceReadyCardProps {
  spaceName: string;
  /** The space's tile, drawn by the caller. */
  tile: React.ReactNode;
  readyAccounts: number;
  botCount: number;
  onAddAccount: () => void;
  onNewBot: () => void;
}

interface Step {
  done: boolean;
  keepAction?: boolean;
  title: string;
  detail?: string;
  action?: { label: string; icon: React.ReactNode; onClick: () => void };
  body?: React.ReactNode;
}

export const SpaceReadyCard: React.FC<SpaceReadyCardProps> = ({ spaceName, tile, readyAccounts, botCount, onAddAccount, onNewBot }) => {
  const rt = useReadyTeams();
  const words = !!rt?.available;
  const picks = rt?.templates ?? [];
  const quiet = `inline rounded text-sky-300 hover:text-sky-200 underline-offset-2 hover:underline ${FOCUS}`;

  const botsStep: Step = words
    ? {
        done: botCount > 0,
        title: 'Add your bots',
        body: (
          <div className="mt-2 space-y-2.5">
            <TeamNeedBox label="What should your bots do?" register />
            {/* flex-wrap: on a touch phone each link is a 44 px target, so wrapped rows stay even. */}
            <p className="flex flex-wrap items-center gap-x-1 text-[11.5px] leading-[18px] text-zinc-400" data-ready-made-line>
              <span className="basis-full">Or start from a ready-made team:</span>
              {picks.map((t) => (
                <React.Fragment key={t.id}>
                  <button type="button" data-template={t.id} onClick={() => rt!.open({ kind: 'template', templateId: t.id })} className={quiet}>
                    {t.label}
                  </button>
                  <span aria-hidden="true"> · </span>
                </React.Fragment>
              ))}
              <span>or </span>
              <button type="button" data-new-bot onClick={onNewBot} className={quiet}>
                add one bot
              </button>
            </p>
          </div>
        ),
      }
    : {
        done: botCount > 0,
        title: 'Add your first bot',
        detail: `${spaceName}'s #general opens with it.`,
        action: { label: 'New bot', icon: <Bot size={13} />, onClick: onNewBot },
      };

  const steps: Step[] = [
    readyAccounts > 0
      ? {
          done: true,
          // Still offered once This machine is ready: more accounts and API keys go here too.
          keepAction: true,
          title: readyAccounts === 1 ? '1 account ready' : `${readyAccounts} accounts ready`,
          detail: 'Add another Claude, Codex or Grok sign-in, or an API key.',
          action: { label: 'Add an account', icon: <KeyRound size={13} />, onClick: onAddAccount },
        }
      : {
          done: false,
          title: 'Add an account',
          detail: 'Sign in with Claude, Codex or Grok, add an API key, or use this machine.',
          action: { label: 'Add an account', icon: <KeyRound size={13} />, onClick: onAddAccount },
        },
    botsStep,
    { done: false, title: 'Message #general', detail: `The best fit answers, only from ${spaceName}'s bots.` },
  ];

  return (
    <main className="flex-1 min-w-0 overflow-y-auto flex items-start sm:items-center justify-center p-4 sm:p-8" data-space-checklist>
      <div className="w-full max-w-md rounded-2xl border border-zinc-800 bg-zinc-900/60 p-5 sm:p-6 space-y-4">
        <div className="flex items-center gap-3">
          {tile}
          <div className="min-w-0">
            <h1 className="text-base font-semibold text-zinc-100 truncate">Get {spaceName} ready</h1>
            <p className="text-xs text-zinc-400">Bots, teams and accounts here stay in {spaceName}.</p>
          </div>
        </div>
        <ol className="space-y-2">
          {steps.map((step, i) => (
            <li key={step.title} className="flex items-start gap-3 rounded-xl border border-zinc-800 bg-zinc-950/50 p-3" data-step={i + 1}>
              <span
                className={`mt-0.5 w-5 h-5 shrink-0 rounded-full flex items-center justify-center text-[11px] font-semibold ${
                  step.done ? 'bg-emerald-500 text-zinc-950' : 'bg-zinc-800 text-zinc-300'
                }`}
                aria-hidden="true"
              >
                {step.done ? <Check size={12} /> : i + 1}
              </span>
              <span className="flex-1 min-w-0">
                <span className={`block text-xs font-medium ${step.done ? (step.keepAction ? 'text-zinc-300' : 'text-zinc-400 line-through') : 'text-zinc-100'}`}>
                  {step.title}
                  {step.done && <span className="sr-only"> (done)</span>}
                </span>
                {step.detail && <span className="block text-[11px] text-zinc-500">{step.detail}</span>}
                {step.body}
              </span>
              {step.action && (!step.done || step.keepAction) && (
                <button
                  type="button"
                  onClick={step.action.onClick}
                  className={`shrink-0 flex items-center gap-1 px-2.5 py-1 rounded-lg text-[11px] font-medium ${FOCUS} ${
                    step.done ? 'text-zinc-200 border border-zinc-700 hover:border-sky-500/50 hover:text-sky-200' : 'text-white bg-sky-600 hover:bg-sky-500'
                  }`}
                >
                  {step.action.icon}
                  {step.action.label}
                </button>
              )}
            </li>
          ))}
        </ol>
      </div>
    </main>
  );
};
