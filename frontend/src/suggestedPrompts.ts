// The "Suggested:" prompts above an empty chat, made from who is in it: a kitchen team gets
// "@HeadChef plan this week's dinners", not developer prompts. No model call, so they show at once.

export interface PromptBot {
  name: string;
  title?: string;
  persona_prompt?: string;
}

export type PromptContext =
  /** A team's chat, or any channel with a fixed set of members. `lead` is a member's name. */
  | { kind: 'team'; members: PromptBot[]; lead?: string; purpose?: string }
  /** A chat with one bot. */
  | { kind: 'bot'; bot: PromptBot }
  /** #general: Laya picks who answers, so prompts don't name anyone. */
  | { kind: 'general'; bots: PromptBot[] };

/** Chips in a 1:1 chat and #general; a team gets one more so a third member has an ask too. */
export const MAX_PROMPTS = 3;
export const MAX_TEAM_PROMPTS = 4;

interface Role {
  match: RegExp;
  /** Lower-case asks that read after "@Name ", the first one being the most typical. */
  asks: [string, string];
  /** A catch-all head word ("engineer"): any more specific role in the same title wins. */
  generic?: true;
}

// Within a pattern, phrases come before single words: a regex takes the first alternative that
// matches at a position, and the longer phrase is the one that says what the job is.
// Patterns are whole words or phrases, never a bare qualifier: "product", "release", "platform",
// "designer" and "coach" say little on their own ("Product Photographer", "Press Release Writer",
// "Interior Designer", "Life Coach"), so only the phrases that pin a field down count.
const ROLES: Role[] = [
  { match: /\bsous( chef)?\b|\bline cook\b|\bprep cook\b/i, asks: ['write a prep list for tonight', 'suggest a quick weeknight dinner'] },
  { match: /\bpastry( chef| cook)?\b|\bbak(er|ing)\b|\bdessert/i, asks: ['suggest a dessert for this weekend', 'bake something with what I have'] },
  { match: /\bnutrition(ist|al)?( coach)?\b|\bdiet(itian)?\b|\bmeal plan(ner|ning)?\b/i, asks: ["check this week's meals are balanced", 'make a high-protein meal plan'] },
  { match: /\bchef\b|\bcook\b|\bculinary\b|\bkitchen\b|\bmenu\b|\brecipes?\b/i, asks: ["plan this week's dinners", 'turn my fridge into a recipe'] },
  { match: /\bgrocer(y|ies)?( planner)?\b|\bshopping( planner)?\b|\bpantry\b/i, asks: ["make this week's shopping list", 'keep my groceries under budget'] },
  { match: /\bsommelier\b|\bwine\b|\bbartender\b|\bmixolog/i, asks: ['pair a drink with dinner', 'suggest a cocktail for Friday'] },
  {
    match: /\b(running|strength|gym|yoga|pilates|fitness) (coach|trainer|instructor)\b|\bpersonal trainer\b|\bfitness\b|\bworkouts?\b/i,
    asks: ['plan my workouts for the week', 'make a 20-minute home routine'],
  },
  { match: /\b(life|career|executive|mindset) coach\b|\bcoach\b|\bmentor\b/i, asks: ['help me set goals for this month', "talk through a decision I'm stuck on"] },
  {
    match: /\bdevops\b|\bsre\b|\b(cyber)?security (engineer|analyst|lead|specialist)\b|\bcybersecurity\b|\binfosec\b|\binfra(structure)?\b|\bsysadmin\b|\bplatform engineer(ing)?\b/i,
    asks: ["check my server's security", 'set up backups for my server'],
  },
  {
    match: /\bqa\b|\btest(er|ing)?\b|\bquality assurance\b|\brelease (engineer|manager)\b/i,
    asks: ['write a test plan for the next release', 'find the riskiest part of this change'],
  },
  {
    match: /\bux\b|\bui\b|\b(product|web|app|interaction|visual) designer\b|\bfrontend\b|\bfront-end\b/i,
    asks: ['review the sign-up screen', 'make this page work on phones'],
  },
  {
    match: /\b(kitchen|interior|home) designer\b|\binterior\b|\bdecorator\b|\bhome decor\b/i,
    asks: ['suggest a layout for my living room', 'pick a colour palette for a room'],
  },
  { match: /\bbackend\b|\bback-end\b|\bdatabase\b|\bapi\b|\bdata engineer/i, asks: ['design the API for a new feature', 'speed up a slow database query'] },
  { match: /\b(software|systems|solutions?) architect\b|\barchitect\b/i, asks: ['sketch the architecture for a new feature', 'weigh two ways to build this'] },
  {
    match: /\bengineer\b|\bdeveloper\b|\bdev\b|\bprogrammer\b|\bcoder\b|\bprototyp/i,
    asks: ['build a quick prototype', 'review my latest changes'],
    generic: true,
  },
  { match: /\bseo\b|\bkeywords?\b/i, asks: ['find keywords I could rank for', 'audit my homepage for SEO'] },
  { match: /\bmarketing\b|\bmarketer\b|\bgrowth\b|\bcampaigns?\b|\bsocial media\b|\bbrand\b/i, asks: ['draft a launch post', 'plan a two-week campaign'] },
  {
    match: /\bstor(y|ies|yteller|ytelling)\b|\bnovelist\b|\bpoet\b|\bfiction\b/i,
    asks: ['write a bedtime story', 'outline a short story'],
  },
  {
    match: /\bwriter\b|\bcopy(writer|writing)?\b|\bcontent\b|\beditor\b|\bauthor\b|\bblog/i,
    asks: ['outline a blog post', 'tighten this paragraph'],
  },
  { match: /\bresearch|\banalyst\b|\bscout\b|\binvestigat/i, asks: ['research my top 3 competitors', 'summarise what people say about this'] },
  { match: /\bsales\b|\bbusiness development\b|\blead gen/i, asks: ['draft a follow-up email to a lead', 'write a short sales pitch'] },
  { match: /\b(customer )?support\b|\bcustomer (success|service)\b|\bcommunity\b/i, asks: ['draft a reply to an upset customer', 'write an FAQ answer'] },
  {
    match: /\bfinancial (planner|advisor)\b|\bfinanc(e|ial)\b|\baccount(ant|ing)\b|\bbookkeep|\bbudget\b|\bcfo\b/i,
    asks: ["summarise this month's spending", 'make a simple budget'],
  },
  { match: /\blegal\b|\blawyer\b|\bcounsel\b|\bcompliance\b/i, asks: ['check this contract for risks', 'explain this clause simply'] },
  { match: /\brecruit|\bhr\b|\bhiring\b|\bpeople ops\b/i, asks: ['write a job post', 'draft interview questions'] },
  { match: /\bteacher\b|\btutor\b|\beducat|\binstructor\b/i, asks: ['explain this topic simply', 'quiz me on this'] },
  { match: /\b(travel|trip)( planner| agent)?\b/i, asks: ['plan a weekend trip', 'make a packing list'] },
  { match: /\bproduct (manager|owner|lead)\b|\bpm\b/i, asks: ['turn this idea into a short spec', 'prioritise these features'] },
  { match: /\bassistant\b|\bsecretary\b|\bplanner\b|\bscheduler\b|\borgani[sz]er\b/i, asks: ['plan my week', 'turn these notes into a to-do list'] },
  { match: /\bvideo( editor)?\b|\bfilm\b|\byoutube\b|\btiktok\b/i, asks: ['script a 30-second video', 'suggest video ideas for this week'] },
  { match: /\bphotograph|\billustrat|\bartist\b|\bgraphic design(er)?\b|\bcover design(er)?\b/i, asks: ['describe a cover image idea', 'critique this image'] },
];

/**
 * The role a piece of text names. A title is a noun phrase whose last word is the job
 * ("Product Photographer" is a photographer), so the match ending latest wins, a longer phrase
 * breaks a tie ("Fitness Coach" over "Coach"), and a catch-all like "engineer" only counts when
 * nothing else does. In prose (a persona or a team's purpose) the first role mentioned wins.
 */
function findRole(text: string | undefined, kind: 'title' | 'prose'): Role | null {
  if (typeof text !== 'string' || !text.trim()) return null;
  let best: { role: Role; start: number; end: number } | null = null;
  for (const role of ROLES) {
    const m = role.match.exec(text);
    if (!m) continue;
    const hit = { role, start: m.index, end: m.index + m[0].length };
    if (!best) {
      best = hit;
      continue;
    }
    if (!!best.role.generic !== !!role.generic) {
      if (best.role.generic) best = hit;
      continue;
    }
    const better =
      kind === 'title'
        ? hit.end > best.end || (hit.end === best.end && hit.start < best.start)
        : hit.start < best.start || (hit.start === best.start && hit.end > best.end);
    if (better) best = hit;
  }
  return best?.role ?? null;
}

const capitalize = (s: string) => (s ? s[0].toUpperCase() + s.slice(1) : s);

/** The role a bot's title says it has, else its persona's opening, else null. */
export function roleOf(bot: PromptBot): Role | null {
  const persona = typeof bot.persona_prompt === 'string' ? bot.persona_prompt.slice(0, 240) : undefined;
  return findRole(bot.title, 'title') ?? findRole(persona, 'prose');
}

const named = (bot: PromptBot | null | undefined): bot is PromptBot => typeof bot?.name === 'string' && !!bot.name.trim();

/** A name that works after "@": the composer's mentions are single words. */
const mentionable = (bot: PromptBot) => /^[A-Za-z0-9_-]+$/.test(bot.name);

/** Asks for different roles, in the order given, one per role: [bot, ask] pairs. */
function distinctAsks(bots: PromptBot[], limit: number): Array<[PromptBot, string]> {
  const seen = new Set<Role>();
  const out: Array<[PromptBot, string]> = [];
  for (const bot of bots) {
    if (out.length >= limit) break;
    const role = roleOf(bot);
    if (!role || seen.has(role)) continue;
    seen.add(role);
    out.push([bot, role.asks[0]]);
  }
  return out;
}

export function suggestedPrompts(ctx: PromptContext): string[] {
  if (ctx.kind === 'bot') {
    const role = ctx.bot ? roleOf(ctx.bot) : null;
    if (!role) return ['What can you help me with?'];
    return [capitalize(role.asks[0]), capitalize(role.asks[1]), 'What can you help me with?'];
  }

  if (ctx.kind === 'general') {
    const bots = (ctx.bots ?? []).filter(named);
    if (!bots.length) return [];
    const asks = distinctAsks(bots, MAX_PROMPTS - 1).map(([, ask]) => capitalize(ask));
    return [...asks, 'Who here can help me with what?'];
  }

  const members = (ctx.members ?? []).filter(named);
  if (!members.length) return [];
  const lead = members.find((m) => m.name === ctx.lead);
  const ordered = lead ? [lead, ...members.filter((m) => m !== lead)] : members;
  const asks = distinctAsks(ordered.filter(mentionable), MAX_TEAM_PROMPTS - 1).map(([bot, ask]) => `@${bot.name} ${ask}`);
  // Nobody's title says what they do: the team's purpose may, else a neutral opener. Either way
  // it goes to the lead (or the first member who can be @mentioned), never a guess at the wrong field.
  const first = ordered.find(mentionable);
  if (!asks.length && first) {
    const role = findRole(ctx.purpose, 'prose');
    asks.push(`@${first.name} ${role ? role.asks[0] : 'what should we tackle first?'}`);
  }
  return [...asks, members.length > 1 ? 'What can each of you help me with?' : 'What can you help me with?'];
}
