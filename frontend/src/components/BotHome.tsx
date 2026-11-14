import React from 'react';
import { ArrowRight, AtSign, Clock, Lightbulb, ListChecks, MessageSquareText, Plug, Sparkles, Wand2 } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import { useKwantMood } from './AvatarMood';
import { NewTag } from './NewTag';
import { markNewSeen } from '../whatsNew';
import { ChatHome, ago, goodAt, starterCards, teamHomeLine } from '../botHome';
import { ACCOUNT_WAYS } from '../firstScreen';

const ICONS = [Sparkles, ListChecks, Lightbulb, Wand2];

interface BotHomeProps {
  home: ChatHome;
  spaceName?: string;
  /** Every bot in the space, for #general's row of faces. */
  team?: Array<{ id: string; name: string; title: string; avatar_sprite: string }>;
  prompts: string[];
  /** false while no account can run a turn: the cards say so and stay still. */
  canRun?: boolean | null;
  onPick: (prompt: string) => void;
  /** Shows "Connect an AI account to start" in place of the asks (first-screen E); the caller decides when. */
  accountCard?: boolean;
  /** Space settings › Accounts. */
  onOpenAccounts?: () => void;
  /** A team's home: who answers, in the composer pill's own words (answerTarget(...).detail). */
  answerLine?: string;
}

/**
 * "Connect an AI account to start" (first-screen E): the first step, not an error, so violet and calm. Shown in
 * an empty chat while nothing can run and Get started isn't on screen.
 */
export const AccountCard: React.FC<{ onOpenAccounts?: () => void }> = ({ onOpenAccounts }) => (
  <div className="mt-8 w-full text-left" data-account-card>
    <div role="region" aria-label="Connect an AI account" className="grid grid-cols-[32px_1fr] gap-x-3 rounded-2xl border border-sky-400/35 bg-sky-400/[0.07] p-4">
      <span className="flex h-8 w-8 items-center justify-center rounded-xl bg-accent-soft text-sky-200" aria-hidden="true">
        <Plug size={16} />
      </span>
      <div className="min-w-0">
        <h3 className="text-[16px] font-semibold leading-[22px] text-zinc-50">Connect an AI account to start</h3>
        <p className="mt-1 text-[14px] leading-[21px] text-zinc-300">Your bots think with an AI account you already have. Any one of these works:</p>
        <ul className="mt-2.5 flex flex-wrap gap-1.5" aria-label="Accounts that work">
          {ACCOUNT_WAYS.map((w) => (
            <li key={w.name} className="rounded-full border border-white/[0.14] px-2.5 py-0.5 text-[12px] text-zinc-100">
              {w.name}
              {w.plan && <span className="text-zinc-400"> {w.plan}</span>}
            </li>
          ))}
        </ul>
      </div>
      <div className="col-span-2 mt-3 sm:col-span-1 sm:col-start-2">
        <button
          type="button"
          onClick={onOpenAccounts}
          data-connect-account
          className="inline-flex h-11 w-full items-center justify-center rounded-xl bg-gradient-to-b from-[#7a62f0] to-[#5b47d6] px-4 text-body font-semibold text-white hover:brightness-110 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-300 focus-visible:ring-offset-2 focus-visible:ring-offset-[#120e26] sm:h-9 sm:w-auto"
        >
          Connect an account
        </button>
      </div>
    </div>
    <p className="mt-2 text-center text-meta text-zinc-400">Nothing runs and nothing is made until one is connected.</p>
  </div>
);

/**
 * The empty chat. A bot's own chat opens on a big living avatar, what it's good at and four things to
 * try; #general opens on "What do you want done?" with the team's faces and three example asks. Everything comes from what the client
 * already has. It waits 220ms before showing (.kw-home) so a chat still loading its messages never flashes it.
 */
export const BotHome: React.FC<BotHomeProps> = ({ home, team = [], prompts, canRun, onPick, accountCard = false, onOpenAccounts, answerLine }) => {
  const bot = home.kind === 'bot' ? home.bot : undefined;
  // A team's chat: its name and purpose over the same faces, chips and asks as #general (13d).
  const crew = home.kind === 'team' ? home.team : undefined;
  const cards = starterCards(prompts, crew ? 3 : 4);
  const chips = bot ? goodAt(bot) : [];
  const shown = team.slice(0, 8);
  const extra = team.length - shown.length;
  // Breathing is the working signal: only a working bot's glow moves (violet, like its dot).
  const working = useKwantMood(bot?.id) === 'working';
  // Nothing can run yet: the faces sleep (eyes shut) and the glow dims to 20 %.
  const asleep = accountCard && !working;
  const general = home.kind === 'general';

  return (
    <section className="kw-home mx-auto flex w-full max-w-3xl flex-col items-center px-1 pb-6 pt-4 text-center sm:pt-10" aria-label={bot ? `${bot.name}, home` : crew ? `${crew.name}, team home` : 'Start a conversation'} data-bot-home={home.kind}>
      {/* The hero: a soft glow behind the floating avatar. Static at .35; it breathes in violet only while the bot works. */}
      <div className="relative mb-4 flex h-36 w-36 items-center justify-center sm:h-44 sm:w-44">
        <span
          className={`absolute inset-0 rounded-full blur-2xl ${working ? 'kw-glow bg-[#9d8fff] opacity-60 motion-reduce:opacity-50' : asleep ? 'bg-accent-gradient opacity-20' : 'bg-accent-gradient opacity-35'}`}
          data-glow={working ? 'working' : asleep ? 'asleep' : 'still'}
          aria-hidden="true"
        />
        {bot ? (
          // 96 px with 4 px mood cells (spec.md §2); the mood badge is cut out of the main surface
          <span className="kw-float relative" style={{ '--kw-avatar-cut': 'var(--kw-surface-main, #120e26)' } as React.CSSProperties}>
            <ModernAvatar role={bot.avatar_sprite} name={bot.name} size={96} kwantId={bot.id} asleep={asleep} className="rounded-2xl shadow-raised" />
          </span>
        ) : (
          <span className="relative flex -space-x-5" aria-hidden="true">
            {shown.slice(0, 3).map((k, i) => (
              <span key={k.id} className={`kw-float rounded-2xl ring-4 ring-[var(--kw-surface-main)] ${i === 1 ? 'z-10 scale-110' : ''}`} style={{ animationDelay: `${i * 300}ms` }}>
                <ModernAvatar role={k.avatar_sprite} name={k.name} size={72} kwantId={k.id} asleep={asleep} />
              </span>
            ))}
            {shown.length === 0 && <Sparkles size={56} className="text-sky-200" />}
          </span>
        )}
      </div>

      <h2 className="text-display font-bold text-zinc-50" data-team-home-title={crew ? '' : undefined}>
        {bot ? bot.name : crew ? crew.name : 'What do you want done?'}
      </h2>
      <p className="mt-1 max-w-md text-body text-zinc-300" data-team-home-line={crew ? '' : undefined}>
        {bot ? bot.title : crew ? teamHomeLine(crew) : 'Say it in your own words. The bot that fits best picks it up, or type @ and a name to choose one.'}
      </p>

      {bot ? (
        chips.length > 0 && (
        <div className="mt-4 flex max-w-xl flex-wrap items-center justify-center gap-1.5" role="list" aria-label={`${bot.name} is good at`}>
          <span className="mr-0.5 text-meta font-medium text-zinc-400">Good at</span>
          {chips.map((c) => (
            <span key={c} role="listitem" className="kw-chip kw-chip-accent">
              {c}
            </span>
          ))}
        </div>
        )
      ) : (
        shown.length > 0 && (
          <div className="mt-4 flex flex-wrap items-center justify-center gap-1.5" role="list" aria-label={crew ? `Who is in ${crew.name}` : 'Bots that can answer here'}>
            {shown.map((k) => {
              const leads = !!crew?.lead && k.name === crew.lead;
              return (
                <span key={k.id} role="listitem" className="kw-chip pl-1" title={leads ? `${k.title}, leads the team` : k.title} data-lead={leads ? '' : undefined}>
                  <ModernAvatar role={k.avatar_sprite} name={k.name} size={18} />
                  {k.name}
                  {leads && <span className="text-zinc-400">· lead</span>}
                </span>
              );
            })}
            {extra > 0 && <span role="listitem" className="kw-chip">+{extra} more</span>}
          </div>
        )
      )}

      {accountCard ? (
        <AccountCard onOpenAccounts={onOpenAccounts} />
      ) : cards.length > 0 && (
        <div className="mt-8 w-full text-left">
          <p className="mb-2 flex items-center gap-1.5 text-meta font-semibold uppercase tracking-wider text-zinc-400">
            <MessageSquareText size={13} aria-hidden="true" /> {general || crew ? 'For example' : 'Try asking'} <NewTag id="starters" className="ml-1.5 normal-case tracking-normal" />
          </p>
          <div className={`kw-stagger grid grid-cols-1 gap-2.5 ${cards.length === 3 ? 'sm:grid-cols-3' : 'sm:grid-cols-2'}`} role="group" aria-label="Suggested prompts">
            {cards.map((prompt, i) => {
              const Icon = ICONS[i % ICONS.length];
              return (
                <button
                  key={prompt}
                  type="button"
                  style={{ '--i': i } as React.CSSProperties}
                  onClick={() => {
                    markNewSeen('starters');
                    onPick(prompt);
                  }}
                  disabled={canRun === false}
                  title={canRun === false ? 'Add an account first' : 'Fills the message box so you can edit it before sending'}
                  className="kw-card-interactive group flex min-h-14 items-start gap-3 p-3.5 hover:border-white/[0.14] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400 focus-visible:ring-offset-2 focus-visible:ring-offset-[#120e26] sm:min-h-[72px]"
                >
                  <span className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-xl bg-accent-soft text-sky-200">
                    <Icon size={16} aria-hidden="true" />
                  </span>
                  <span className="min-w-0 flex-1 text-body font-medium leading-snug text-zinc-100">{prompt}</span>
                  <ArrowRight size={16} className="mt-1 shrink-0 text-zinc-500 transition-transform duration-150 group-hover:translate-x-0.5 group-hover:text-sky-300 motion-reduce:transition-none" aria-hidden="true" />
                </button>
              );
            })}
          </div>
        </div>
      )}

      {bot && home.recent && home.recent.length > 0 && (
        <div className="mt-6 w-full text-left">
          <p className="mb-2 flex items-center gap-1.5 text-meta font-semibold uppercase tracking-wider text-zinc-400">
            <Clock size={13} aria-hidden="true" /> Pick up where you left off
          </p>
          <ul className="grid grid-cols-1 gap-2 sm:grid-cols-2">
            {home.recent.map((r) => (
              <li key={r.id}>
                <button type="button" onClick={() => home.onOpenRecent?.(r.id)} className="kw-card-interactive flex w-full items-center justify-between gap-3 px-3.5 py-2.5">
                  <span className="min-w-0 truncate text-body text-zinc-100">{r.title}</span>
                  <span className="shrink-0 text-meta text-zinc-400">{ago(r.updated_at)}</span>
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}

      {!accountCard && (
        crew ? (
          // Can wrap on a phone, so the @ sits inline with the words instead of beside the block.
          <p className="mt-8 max-w-md text-balance text-meta text-zinc-400" data-answer-line="">
            <AtSign size={13} aria-hidden="true" className="mr-1.5 inline-block align-[-2px]" />
            {answerLine ?? `${crew.name} answers. Type @ and a name to ask one member.`}
          </p>
        ) : (
        <p className="mt-8 flex items-center gap-1.5 text-meta text-zinc-400">
          <AtSign size={13} aria-hidden="true" />
          {general ? 'Type @ to ask one bot yourself.' : 'Type @ to bring another bot into the conversation.'}
        </p>
        )
      )}
    </section>
  );
};
