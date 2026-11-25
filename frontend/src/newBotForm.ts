// The New bot form asks for a name and a job first (36g new-bot-name-first). These two helpers pick the
// avatar from the job until the person picks one, and say when editing a bot should open "More options".
import type { AgentFormData } from './components/NewAgentModal';

/**
 * Words in a job that point at one of AVATAR_STYLES, from ModernAvatar's own name rules plus a few plain
 * job words. A word matches at the start of a word ("test" matches "testing", not "latest").
 */
export const JOB_AVATAR_WORDS: Array<[string, string[]]> = [
  ['quizmaster', ['quiz', 'trivia']],
  ['desk', ['support', 'customer', 'front desk', 'help desk', 'reception', 'inbox', 'email', 'booking', 'appointment']],
  ['security', ['security', 'secure', 'vulnerab', 'pentest', 'threat']],
  ['devops', ['devops', 'deploy', 'server', 'infra', 'terminal', 'shell']],
  ['tech_lead', ['tech lead', 'code', 'coding', 'engineer', 'programm']],
  ['architect', ['architect', 'system design']],
  ['fast', ['speed', 'quick', 'fast']],
  ['designer', ['design', 'ux', 'ui']],
  ['data', ['data', 'analy', 'spreadsheet', 'sql', 'chart', 'metric']],
  ['qa', ['qa', 'test', 'bug']],
  ['product', ['product', 'roadmap']],
  ['writer', ['writ', 'docs', 'blog', 'article', 'newsletter', 'proofread']],
  ['research', ['research', 'scien', 'study']],
  ['teacher', ['teach', 'tutor', 'lesson', 'homework']],
  ['marketing', ['marketing', 'seo', 'social media', 'campaign', 'advert']],
  ['legal', ['legal', 'lawyer', 'contract']],
  ['coach', ['coach', 'mentor', 'career']],
  ['traveler', ['travel', 'trip', 'flight', 'hotel']],
  ['chef', ['chef', 'cook', 'recipe', 'meal']],
  ['fitness', ['fitness', 'workout', 'trainer', 'gym']],
  ['party', ['party', 'event', 'birthday', 'wedding']],
  ['builder', ['diy', 'builder', 'handy', 'repair', 'renovat']],
  ['zen', ['wellbeing', 'mindful', 'zen', 'meditat', 'stress']],
  ['home', ['home', 'house', 'chore', 'garden']],
  ['santa', ['santa', 'christmas']],
  ['snap', ['photo']],
  ['tally', ['sign-up', 'signup', 'rsvp']],
];

const escape = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
const RULES = JOB_AVATAR_WORDS.map(([id, words]) => [id, new RegExp(`\\b(?:${words.map(escape).join('|')})`, 'i')] as const);

/** The avatar style for a job (then the name): the style whose word comes first, else 'default'. */
export function avatarForJob(job: string, name = ''): string {
  for (const text of [job, name]) {
    let best: { id: string; at: number } | null = null;
    for (const [id, re] of RULES) {
      const m = re.exec(text);
      if (m && (!best || m.index < best.at)) best = { id, at: m.index };
    }
    if (best) return best.id;
  }
  return 'default';
}

/** Whether a bot being edited has something set that "More options" holds, so it opens expanded. */
export function hasCustomMoreOptions(bot: AgentFormData | null | undefined): boolean {
  if (!bot) return false;
  const title = (bot.title ?? '').trim();
  return (
    (title !== '' && title !== 'AI Specialist') ||
    (!!bot.preferred_model && bot.preferred_model !== 'auto') ||
    !!(bot.soul ?? '').trim() ||
    !!bot.profile_id ||
    bot.profile_failover === false ||
    !!bot.covers?.length ||
    !!bot.avoids?.length ||
    !!bot.browser_allowed_sites?.length ||
    !!bot.browser_blocked_sites?.length ||
    !!bot.browser_block_private_network
  );
}
