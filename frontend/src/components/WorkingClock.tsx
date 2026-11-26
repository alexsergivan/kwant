import React, { useEffect, useRef, useState } from 'react';
import type { LiveActivity } from '../activity';
import { formatElapsed, turnStart } from '../workingStrip';

// 36d: the working scene's two timed pieces of text (working-scene/design.md §4-5). Neither moves anything.

/** 36d §5: a changed sentence is spoken at most once every 3 s; the last one always lands. */
export function useThrottledText(text: string, gapMs = 3000): string {
  const [shown, setShown] = useState(text);
  const last = useRef(0);
  useEffect(() => {
    if (text === shown) return;
    const wait = last.current + gapMs - Date.now();
    if (wait <= 0) {
      last.current = Date.now();
      setShown(text);
      return;
    }
    const id = window.setTimeout(() => {
      last.current = Date.now();
      setShown(text);
    }, wait);
    return () => window.clearTimeout(id);
  }, [text, shown, gapMs]);
  useEffect(() => {
    last.current = Date.now();
  }, []);
  return shown;
}

/** "Working · 0:09": its own component, so the second tick redraws only the clock. Text, not motion: it counts under reduced motion too. */
export const Clock: React.FC<{ live?: LiveActivity | null }> = ({ live }) => {
  const mountedAt = useRef(Date.now());
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, []);
  return (
    <span className="tabular-nums" aria-hidden="true" data-working-elapsed="">
      Working · {formatElapsed(now - turnStart(live, mountedAt.current))}
    </span>
  );
};

