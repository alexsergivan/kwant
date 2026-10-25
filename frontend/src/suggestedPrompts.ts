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

export const MAX_PROMPTS = 3;

interface Role {
  match: RegExp;
  /** Lower-case asks that read after "@Name ", the first one being the most typical. */
  asks: [string, string];
}

// Most specific first: "Sous Chef" is a sous chef before it is a chef, "DevOps" before "dev".
const ROLES: Role[] = [
  { match: /\bsous\b|\bline cook\b|\bprep\b/i, asks: ['write a prep list for tonight', 'suggest a quick weeknight dinner'] },
  { match: /\bpastry\b|\bbak(er|ing)\b|\bdessert/i, asks: ['suggest a dessert for this weekend', 'bake something with what I have'] },
  { match: /\bnutrition|\bdiet|\bdietitian\b/i, asks: ["check this week's meals are balanced", 'make a high-protein meal plan'] },
  { match: /\bchef\b|\bcook\b|\bculinary\b|\bkitchen\b|\bmenu\b|\brecipe/i, asks: ["plan this week's dinners", 'turn my fridge into a recipe'] },
  { match: /\bgrocer|\bshopping\b|\bpantry\b/i, asks: ["make this week's shopping list", 'keep my groceries under budget'] },
  { match: /\bsommelier\b|\bwine\b|\bbartender\b|\bmixolog/i, asks: ['pair a drink with dinner', 'suggest a cocktail for Friday'] },
  { match: /\bfitness\b|\btrainer\b|\bcoach\b|\bworkout\b/i, asks: ['plan my workouts for the week', 'make a 20-minute home routine'] },
  { match: /\bdevops\b|\bsre\b|\bsecurity\b|\binfra(structure)?\b|\bsysadmin\b|\bplatform\b/i, asks: ["check my server's security", 'set up backups for my server'] },
  { match: /\bqa\b|\btest(er|ing)?\b|\bquality\b|\brelease\b/i, asks: ['write a test plan for the next release', 'find the riskiest part of this change'] },
  { match: /\bdesign(er)?\b|\bux\b|\bui\b|\bfrontend\b|\bfront-end\b/i, asks: ['review the sign-up screen', 'make this page work on phones'] },
  { match: /\bbackend\b|\bback-end\b|\bdatabase\b|\bapi\b|\bdata engineer/i, asks: ['design the API for a new feature', 'speed up a slow database query'] },
  { match: /\barchitect\b/i, asks: ['sketch the architecture for a new feature', 'weigh two ways to build this'] },
  { match: /\bengineer\b|\bdeveloper\b|\bdev\b|\bprogrammer\b|\bcoder\b|\bprototyp/i, asks: ['build a quick prototype', 'review my latest changes'] },
  { match: /\bseo\b|\bkeyword/i, asks: ['find keywords I could rank for', 'audit my homepage for SEO'] },
  { match: /\bmarketing\b|\bmarketer\b|\bgrowth\b|\bcampaign\b|\bsocial media\b|\bbrand\b/i, asks: ['draft a launch post', 'plan a two-week campaign'] },
  { match: /\bwriter\b|\bcopy(writer|writing)?\b|\bcontent\b|\beditor\b|\bauthor\b|\bblog/i, asks: ['outline a blog post', 'tighten this paragraph'] },
  { match: /\bresearch|\banalyst\b|\bscout\b|\binvestigat/i, asks: ['research my top 3 competitors', 'summarise what people say about this'] },
  { match: /\bsales\b|\bbusiness development\b|\blead gen/i, asks: ['draft a follow-up email to a lead', 'write a short sales pitch'] },
  { match: /\bsupport\b|\bcustomer\b|\bcommunity\b/i, asks: ['draft a reply to an upset customer', 'write an FAQ answer'] },
  { match: /\bfinance\b|\baccount(ant|ing)\b|\bbookkeep|\bbudget\b|\bcfo\b/i, asks: ["summarise this month's spending", 'make a simple budget'] },
  { match: /\blegal\b|\blawyer\b|\bcounsel\b|\bcompliance\b/i, asks: ['check this contract for risks', 'explain this clause simply'] },
  { match: /\brecruit|\bhr\b|\bhiring\b|\bpeople ops\b/i, asks: ['write a job post', 'draft interview questions'] },
  { match: /\bteacher\b|\btutor\b|\beducat|\binstructor\b/i, asks: ['explain this topic simply', 'quiz me on this'] },
  { match: /\btravel\b|\btrip\b/i, asks: ['plan a weekend trip', 'make a packing list'] },
  { match: /\bproduct manager\b|\bproduct owner\b|\bpm\b|\bproduct\b/i, asks: ['turn this idea into a short spec', 'prioritise these features'] },
  { match: /\bassistant\b|\bsecretary\b|\bplanner\b|\bscheduler\b|\borgani[sz]er\b/i, asks: ['plan my week', 'turn these notes into a to-do list'] },
  { match: /\bvideo\b|\bfilm\b|\byoutube\b|\btiktok\b/i, asks: ['script a 30-second video', 'suggest video ideas for this week'] },
  { match: /\bphotograph|\billustrat|\bartist\b/i, asks: ['describe a cover image idea', 'critique this image'] },
];

const capitalize = (s: string) => (s ? s[0].toUpperCase() + s.slice(1) : s);

/** The role a bot's title says it has, else its persona's opening, else null. */
export function roleOf(bot: PromptBot): Role | null {
  const byText = (text: string | undefined) => (text ? ROLES.find((r) => r.match.test(text)) ?? null : null);
  return byText(bot.title) ?? byText(bot.persona_prompt?.slice(0, 240));
}

/** A name that works after "@": the composer's mentions are single words. */
const mentionable = (name: string) => /^[A-Za-z0-9_-]+$/.test(name);

/** Asks for different roles, in the order given, one per role: [bot, ask] pairs. */
function distinctAsks(bots: PromptBot[], limit: number): Array<[PromptBot, string]> {
  const seen = new Set<Role>();
  const out: Array<[PromptBot, string]> = [];
  for (const bot of bots) {
    const role = roleOf(bot);
    if (!role || seen.has(role)) continue;
    seen.add(role);
    out.push([bot, role.asks[0]]);
    if (out.length >= limit) break;
  }
  return out;
}

export function suggestedPrompts(ctx: PromptContext): string[] {
  if (ctx.kind === 'bot') {
    const role = roleOf(ctx.bot);
    if (!role) return ['What can you help me with?'];
    return [capitalize(role.asks[0]), capitalize(role.asks[1]), 'What can you help me with?'];
  }

  if (ctx.kind === 'general') {
    if (!ctx.bots.length) return [];
    const asks = distinctAsks(ctx.bots, MAX_PROMPTS - 1).map(([, ask]) => capitalize(ask));
    return [...asks, 'Who here can help me with what?'];
  }

  const members = ctx.members.filter((m) => m.name.trim());
  if (!members.length) return [];
  const lead = members.find((m) => m.name === ctx.lead);
  const ordered = lead ? [lead, ...members.filter((m) => m !== lead)] : members;
  const asks = distinctAsks(ordered.filter((m) => mentionable(m.name)), MAX_PROMPTS - 1).map(([bot, ask]) => `@${bot.name} ${ask}`);
  // Nobody's title says what they do: the team's purpose may, and the lead takes it.
  if (!asks.length && lead && mentionable(lead.name) && ctx.purpose) {
    const role = ROLES.find((r) => r.match.test(ctx.purpose!));
    if (role) asks.push(`@${lead.name} ${role.asks[0]}`);
  }
  return [...asks, members.length > 1 ? 'What can each of you help me with?' : 'What can you help me with?'];
}
