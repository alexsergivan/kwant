// The guest's outbox (design.md "For Iris" 3): every message gets a client_id and is drawn dimmed until the box's
// copy with the same client_id comes back. One that couldn't reach the box stays on this phone (localStorage, per
// pocket and seat) and is sent again on Retry, on `online`, or when the page opens again.
import type { Msg } from './view';

export interface Store {
  getItem(k: string): string | null;
  setItem(k: string, v: string): void;
  removeItem?(k: string): void;
}

export interface Out {
  client_id: string;
  text: string;
  /** When the guest pressed Send (ISO). */
  at: string;
  state: 'sending' | 'failed';
}

const KEEP = 20;

/** A client_id unique enough for one guest's messages (impl.md cuts it at 64 characters). */
export function clientId(now: number, rnd: () => number = Math.random): string {
  return `c${now.toString(36)}${Math.floor(rnd() * 36 ** 6).toString(36).padStart(6, '0')}`;
}

export class Outbox {
  items: Out[] = [];
  private key = '';

  constructor(private store: Store | null | undefined) {}

  /** Which seat the outbox belongs to. Messages kept from an earlier visit come back as failed. */
  open(code: string, me: string): void {
    const key = `kp-desk-out:${code}:${me}`;
    if (key === this.key) return;
    this.key = key;
    this.items = [];
    try {
      const raw = this.store?.getItem(key);
      const list: unknown = raw ? JSON.parse(raw) : [];
      if (Array.isArray(list))
        for (const x of list) {
          const o = x as Partial<Out>;
          if (typeof o?.client_id === 'string' && typeof o.text === 'string' && o.text && typeof o.at === 'string')
            this.items.push({ client_id: o.client_id, text: o.text, at: o.at, state: 'failed' });
        }
    } catch {
      /* unreadable: start empty */
    }
  }

  add(text: string, id: string, at: string): Out {
    const o: Out = { client_id: id, text, at, state: 'sending' };
    this.items.push(o);
    this.save();
    return o;
  }

  get(id: string): Out | undefined {
    return this.items.find((o) => o.client_id === id);
  }

  mark(id: string, state: Out['state']): void {
    const o = this.get(id);
    if (o) o.state = state;
    this.save();
  }

  drop(id: string): void {
    this.items = this.items.filter((o) => o.client_id !== id);
    this.save();
  }

  /** The box's thread has these messages now: their dimmed copies go (the swap without a flicker). */
  settle(messages: Msg[]): boolean {
    const have = new Set(messages.map((m) => m.client_id).filter(Boolean));
    const before = this.items.length;
    this.items = this.items.filter((o) => !have.has(o.client_id));
    if (this.items.length !== before) this.save();
    return this.items.length !== before;
  }

  failed(): Out[] {
    return this.items.filter((o) => o.state === 'failed');
  }

  private save(): void {
    if (!this.key) return;
    try {
      const keep = this.items.slice(-KEEP).map(({ client_id, text, at }) => ({ client_id, text, at }));
      if (keep.length) this.store?.setItem(this.key, JSON.stringify(keep));
      else if (this.store?.removeItem) this.store.removeItem(this.key);
      else this.store?.setItem(this.key, '[]');
    } catch {
      /* private mode or full: the message stays on the page until reload */
    }
  }
}
