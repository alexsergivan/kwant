// The guest view the page reads (pa1 "Pocket API v1", per-viewer: this guest's own view only).
// Every number shown comes from here; the page never works out a score, a rank or a percentage.

export type Phase = 'join' | 'lobby' | 'question' | 'reveal' | 'between' | 'paused' | 'ended' | 'full' | 'removed';

export interface Team {
  id: string;
  name: string;
  /** Team colour: index into the 8 avatar palettes in join order (cyan, coral, mint, yellow, blue, green, red, violet). */
  color: number;
  count: number;
  max: number;
  full?: boolean;
}

export interface Retention {
  kind: 'after_end' | 'after_date' | 'at_end';
  days?: number;
  /** YYYY-MM-DD */
  date?: string;
}

export interface Me {
  name: string;
  avatar: string;
  team?: string;
  team_name?: string;
  team_color?: number;
  team_count?: number;
  score: number;
}

export type Answer = { choice: string } | { text: string } | { number: number };

export interface Question {
  id: string;
  round: number;
  round_name?: string;
  rounds?: number;
  n: number;
  of: number;
  kind: 'choice' | 'text' | 'number' | 'picture';
  text: string;
  options?: string[];
  picture?: { url: string; alt?: string; caption?: string; pixel?: boolean };
  number?: { start?: number; min?: number; max?: number };
  hint?: string;
  secs_left: number;
  total_secs: number;
  /** Set once the server has this guest's answer (locked in). */
  answer?: Answer;
  /** Players still to answer, shown on the locked-in screen. */
  waiting?: number;
}

export interface Reveal {
  outcome: 'right' | 'near' | 'wrong' | 'none' | 'overturned';
  points: number;
  /** Right answers: base + speed bonus, as the kit scored them. */
  base?: number;
  speed?: number;
  /** Wrong number answers that still score: "for being close". */
  close?: boolean;
  /** This guest's answer as they gave it. */
  yours?: string;
  /** The Quizmaster's reason for a near miss ("One letter off, so it counts."). */
  why?: string;
  answer: string;
  answer_letter?: string;
  pct?: number;
  source?: string;
  team?: { rank: number; name: string; color: number; score: number };
}

export interface BoardRow {
  rank: number;
  name: string;
  color: number;
  score: number;
  move?: number;
  me?: boolean;
}

export interface Between {
  after_round: number;
  rounds: number;
  next_in_secs?: number;
  line?: string;
  board: BoardRow[];
}

export interface GuestView {
  v: 1;
  kit: string;
  /** What to call it in copy: "quiz" for the pub quiz. */
  noun?: string;
  title: string;
  host?: string;
  /** ISO time the lobby or the game starts. */
  starts_at?: string;
  phase: Phase;
  me?: Me;
  teams?: Team[] | null;
  team_max?: number;
  rounds?: number;
  max_players?: number;
  player_count?: number;
  team_count?: number;
  players?: { name: string; avatar: string }[];
  retention?: Retention;
  question?: Question;
  reveal?: Reveal & { question: Question };
  between?: Between;
  ended?: { winner?: string; points?: number };
  /** YYYY-MM-DD the guest data is deleted, once the date is known. */
  deletes_on?: string;
  /** Same-origin pictures for later rounds, preloaded in the lobby one at a time. */
  preload?: string[];
  line?: string;
}
