// secret-ballot-open-phase: the box keys a vote by a token only this phone knows, so it never shows a ballot back.
// This phone keeps the token and the order it sent, per pocket and seat, so "your order" and "Change my vote" come
// from here. The token is never shown; a guest on another phone sees that their vote is in and secret.

export interface Store {
  getItem(k: string): string | null;
  setItem(k: string, v: string): void;
}

/** What this phone keeps for one seat: its token, and the order it sent in which round ("vote again" adds one). */
export interface Copy {
  token: string;
  revotes: number;
  ranking?: number[];
}

const TOKEN = /^[0-9a-f]{32}$/;

/** 16 random bytes as 32 lowercase hex characters (what the box accepts). */
export function newToken(random: (b: Uint8Array) => Uint8Array): string {
  return Array.from(random(new Uint8Array(16)), (x) => x.toString(16).padStart(2, '0')).join('');
}

const whole = (x: unknown): x is number => typeof x === 'number' && Number.isInteger(x) && x >= 0;

function parse(raw: string | null): Copy | undefined {
  if (!raw) return undefined;
  try {
    const o = JSON.parse(raw) as Record<string, unknown> | null;
    if (!o || typeof o.token !== 'string' || !TOKEN.test(o.token)) return undefined;
    const ranking = Array.isArray(o.ranking) && o.ranking.length && o.ranking.every(whole) ? (o.ranking as number[]) : undefined;
    return { token: o.token, revotes: whole(o.revotes) ? o.revotes : 0, ranking };
  } catch {
    return undefined;
  }
}

/** This phone's ballots: localStorage when the browser allows it, else this page load only. */
export class Ballots {
  private mem = new Map<string, string>();

  constructor(
    private store: Store | null | undefined,
    private code: string,
    private random: (b: Uint8Array) => Uint8Array,
  ) {}

  private key(seat: string): string {
    return `kp-ballot:${this.code}:${seat}`;
  }

  private get(seat: string): Copy | undefined {
    let raw: string | null = null;
    try {
      raw = this.store?.getItem(this.key(seat)) ?? null;
    } catch {
      raw = null;
    }
    return parse(raw ?? this.mem.get(this.key(seat)) ?? null);
  }

  private put(seat: string, c: Copy): void {
    const raw = JSON.stringify(c);
    this.mem.set(this.key(seat), raw);
    try {
      this.store?.setItem(this.key(seat), raw);
    } catch {
      // Private mode or a full store: this page load still has it.
    }
  }

  /** The token this phone votes with for this seat: the kept one, or a new one kept from now on. */
  token(seat: string): string {
    const c = this.get(seat);
    if (c) return c.token;
    const token = newToken(this.random);
    this.put(seat, { token, revotes: 0 });
    return token;
  }

  /** After the box took the vote: the order this phone sent, in this round. */
  sent(seat: string, revotes: number, ranking: number[]): void {
    this.put(seat, { token: this.token(seat), revotes, ranking: ranking.slice() });
  }

  /** The order this phone sent in this round, if it did ("vote again" makes an earlier one old). */
  mine(seat: string, revotes: number): number[] | undefined {
    const c = this.get(seat);
    return c?.ranking && c.revotes === revotes ? c.ranking.slice() : undefined;
  }
}
