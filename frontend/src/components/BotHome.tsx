import React from 'react';
import { ArrowRight, AtSign, Clock, Lightbulb, ListChecks, MessageSquareText, Sparkles, Wand2 } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import { NewTag } from './NewTag';
import { markNewSeen } from '../whatsNew';
import { ChatHome, ago, goodAt, starterCards } from '../botHome';

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
}

/**
 * The empty chat. A bot's own chat opens on a big living avatar, what it's good at and four things to
 * try; #general opens on "Laya picks" with the team's faces. Everything comes from what the client
 * already has. It waits 220ms before showing (.kw-home) so a chat still loading its messages never flashes it.
 */
export const BotHome: React.FC<BotHomeProps> = ({ home, spaceName, team = [], prompts, canRun, onPick }) => {
  const bot = home.kind === 'bot' ? home.bot : undefined;
  const cards = starterCards(prompts);
  const chips = bot ? goodAt(bot) : [];
  const shown = team.slice(0, 8);
  const extra = team.length - shown.length;

  return (
    <section className="kw-home mx-auto flex w-full max-w-3xl flex-col items-center px-1 pb-6 pt-4 text-center sm:pt-10" aria-label={bot ? `${bot.name}, home` : 'Start a conversation'} data-bot-home={home.kind}>
      {/* The hero: a soft accent glow that breathes behind the avatar, which floats */}
      <div className="relative mb-4 flex h-36 w-36 items-center justify-center sm:h-44 sm:w-44">
        <span className="kw-glow absolute inset-0 rounded-full bg-accent-gradient opacity-60 blur-2xl" aria-hidden="true" />
        {bot ? (
          // 96 px with 4 px mood cells (spec.md §2); the mood badge is cut out of the main surface
          <span className="kw-float relative" style={{ '--kw-avatar-cut': 'var(--kw-surface-main, #120e26)' } as React.CSSProperties}>
            <ModernAvatar role={bot.avatar_sprite} name={bot.name} size={96} kwantId={bot.id} className="rounded-2xl shadow-raised" />
          </span>
        ) : (
          <span className="relative flex -space-x-5" aria-hidden="true">
            {shown.slice(0, 3).map((k, i) => (
              <span key={k.id} className={`kw-float rounded-2xl ring-4 ring-[var(--kw-surface-main)] ${i === 1 ? 'z-10 scale-110' : ''}`} style={{ animationDelay: `${i * 300}ms` }}>
                <ModernAvatar role={k.avatar_sprite} name={k.name} size={72} kwantId={k.id} />
              </span>
            ))}
            {shown.length === 0 && <Sparkles size={56} className="text-sky-200" />}
          </span>
        )}
      </div>

      <h2 className="text-display font-bold text-zinc-50">{bot ? bot.name : spaceName ? `Ask anything in ${spaceName}` : 'Ask anything'}</h2>
      <p className="mt-1 max-w-md text-body text-zinc-300">
        {bot ? bot.title : 'Laya picks who answers. Or type @ and a name to choose a bot yourself.'}
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
          <div className="mt-4 flex flex-wrap items-center justify-center gap-1.5" role="list" aria-label="Bots Laya can pick from">
            {shown.map((k) => (
              <span key={k.id} role="listitem" className="kw-chip pl-1" title={k.title}>
                <ModernAvatar role={k.avatar_sprite} name={k.name} size={18} />
                {k.name}
              </span>
            ))}
            {extra > 0 && <span role="listitem" className="kw-chip">+{extra} more</span>}
          </div>
        )
      )}

      {cards.length > 0 && (
        <div className="mt-8 w-full text-left">
          <p className="mb-2 flex items-center gap-1.5 text-meta font-semibold uppercase tracking-wider text-zinc-400">
            <MessageSquareText size={13} aria-hidden="true" /> Try asking <NewTag id="starters" className="ml-1.5 normal-case tracking-normal" />
          </p>
          <div className="kw-stagger grid grid-cols-1 gap-2.5 sm:grid-cols-2" role="group" aria-label="Suggested prompts">
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
                  className="kw-card-interactive group flex min-h-[72px] items-start gap-3 p-3.5"
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

      <p className="mt-8 flex items-center gap-1.5 text-meta text-zinc-400">
        <AtSign size={13} aria-hidden="true" />
        {home.kind === 'general' ? 'Type @ to ask a specific bot instead of letting Laya pick.' : 'Type @ to bring another bot into the conversation.'}
      </p>
    </section>
  );
};
