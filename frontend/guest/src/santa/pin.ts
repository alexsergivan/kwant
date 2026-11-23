// The 4-digit PIN (G1, G3, G4): one real input (numeric keyboard, nothing remembered) under four 60 px boxes that
// show dots, never digits. Screen readers and keyboards use the input; the boxes are only a picture of it.
import { h } from '../dom';
import type { Fail } from '../api';
import { clock } from '../app';

export class PinInput {
  readonly el: HTMLElement;
  readonly input: HTMLInputElement;
  private boxes: HTMLElement[];

  constructor(id: string, label: string, private onChange: (pin: string) => void, value = '', warn = false, describedBy?: string) {
    this.input = h('input', {
      id,
      'data-k': id,
      class: 'pin-in',
      type: 'text',
      inputmode: 'numeric',
      pattern: '[0-9]*',
      maxlength: 4,
      autocomplete: 'off',
      autocorrect: 'off',
      spellcheck: 'false',
      enterkeyhint: 'go',
      'aria-label': label,
      'aria-describedby': describedBy ?? null,
      'aria-invalid': warn ? 'true' : null,
    }) as HTMLInputElement;
    this.boxes = [0, 1, 2, 3].map(() => h('div', { class: `pin${warn ? ' warn' : ''}` }));
    this.el = h('div', { class: 'pinwrap' }, h('div', { class: 'pins', 'aria-hidden': 'true' }, this.boxes), this.input);
    this.input.value = clean(value);
    this.input.addEventListener('input', () => {
      const v = clean(this.input.value);
      if (v !== this.input.value) this.input.value = v;
      this.paint();
      this.onChange(v);
    });
    this.input.addEventListener('focus', () => this.paint());
    this.input.addEventListener('blur', () => this.paint());
    this.paint();
  }

  get value(): string {
    return this.input.value;
  }

  private paint(): void {
    const n = this.input.value.length;
    const focused = document.activeElement === this.input;
    this.boxes.forEach((b, i) => {
      b.classList.toggle('on', focused && i === Math.min(n, 3) && n < 4);
      b.replaceChildren(i < n ? '•' : focused && i === n ? h('span', { class: 'caret' }) : '');
    });
  }
}

/** Digits only, at most 4. */
export const clean = (s: string): string => s.replace(/\D/g, '').slice(0, 4);

export type Problem = { title: string; sub?: string; field?: 'name' | 'pin' | 'code' };

/**
 * Plain words for a refused join or rejoin. The wrong-PIN tries come from the server's `tries_left`;
 * other refusals show pa1's own `message` when it sent one.
 */
export function pinCopy(r: Fail, host: string, joining: boolean): Problem {
  const e = r.error;
  if (e === 'wrong_pin') {
    const n = r.tries_left;
    const left = n === undefined ? '' : n > 0 ? `${n} more ${n === 1 ? 'try' : 'tries'}, then a 10-minute wait. ` : 'Next comes a 10-minute wait. ';
    return { title: "That name and PIN don't match.", sub: `${left}Check the name is spelled the way you joined.`, field: 'pin' };
  }
  if (e === 'locked') {
    const t = r.until ? clock(r.until) : '';
    return { title: 'Too many tries for now.', sub: `${t ? `Try again after ${t}` : 'Try again in 10 minutes'}, or ask ${host} for a reset code.`, field: 'pin' };
  }
  if (e === 'hard_locked') return { title: 'This name is locked.', sub: `Ask ${host} for a reset code to get back in.`, field: 'pin' };
  if (e === 'pin_required') return { title: 'Pick a 4-digit PIN.', field: 'pin' };
  if (e === 'name_taken')
    return joining
      ? { title: 'Someone already joined with that name.', sub: 'If it\'s you, tap "I\'ve joined before". Otherwise add a letter to your name.', field: 'name' }
      : { title: 'That name is taken.', field: 'name' };
  if (e === 'name_empty') return { title: 'Type your name first.', field: 'name' };
  if (e === 'name_too_long') return { title: 'Names are 1-20 letters.', field: 'name' };
  if (e === 'name_not_allowed') return { title: "That name can't be used here. Try another one.", field: 'name' };
  if (e === 'reset_pending') return { title: `${cap(host)} made you a reset code.`, sub: 'Tap "I have a reset code" and type it with a new PIN.', field: 'code' };
  if (e === 'no_reset' || e === 'reset_expired') return { title: r.message ?? "That reset code doesn't work any more.", sub: `Ask ${host} for a new one.`, field: 'code' };
  if (e === 'slow_down' || r.status === 429) return { title: 'Too many tries just now.', sub: 'Wait a minute, then try again.' };
  if (r.status === 0 || r.status >= 500) return { title: `Couldn't reach ${host}'s computer.`, sub: 'Check your signal and try again. What you typed is kept.' };
  if (r.message) return { title: r.message };
  return { title: "That didn't work. Try again." };
}

export const cap = (s: string): string => s.charAt(0).toUpperCase() + s.slice(1);
