import type { CSSProperties } from 'react';
/** bubble-not-alert: a bot reply's creature thread takes the avatar's body colour (design in autonomy/bubble-not-alert/design.md). */
export function replyThreadStyle(body: string | undefined): CSSProperties | undefined {
  return body && /^#[0-9a-f]{6}$/i.test(body) ? ({ '--kw-thread': body } as CSSProperties) : undefined;
}
