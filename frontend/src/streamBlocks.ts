// Streaming markdown, a block at a time (15a).
//
// While a reply streams, its markdown is parsed again every frame. For a long reply that is the whole cost of
// the frame, and it grows with the reply. `splitStreamBlocks` cuts the text into finished blocks plus one open
// block at the end, so the finished ones can render once each (memoized) and only the open one is parsed per
// frame. The final message never goes through here: it renders whole, as before.
//
// A block ends at a blank line outside a code fence, unless the next line carries on the same construct (a
// list, an indented code block or continuation, a table, a block quote). When unsure it does not split: a
// bigger open block is only slower, never wrong. Joining the blocks always gives back the text exactly.

/** An opening code fence: ``` or ~~~, three or more, any indent (a fence inside a list item is indented). */
const FENCE_OPEN = /^([ \t]*)(`{3,}|~{3,})(.*)$/;
/** A closing fence: only the fence characters and trailing spaces. */
const FENCE_CLOSE = /^([ \t]*)(`{3,}|~{3,})[ \t]*$/;
/** A list item: a bullet or a number with . or ), then a space or the line's end. */
const LIST_ITEM = /^ {0,3}(?:[-*+]|\d{1,9}[.)])(?:[ \t]|$)/;
/** A link reference or footnote definition: it can reach back to an earlier block, so such text is not split. */
const DEFINITION = /^ {0,3}\[[^\]\n]+\]:/m;

const indentOf = (s: string) => s.length - s.replace(/^[ \t]+/, '').length;

/**
 * True when the line still being typed already says what it starts: its first word is complete, or it is long.
 * Before that, "2" could still become "2. item" and " " could become an indented code line.
 */
const decidable = (partial: string) => /^[ \t]*\S+[ \t]/.test(partial) || partial.length >= 16;

/** Whether a blank line between `prev` (last non-blank line) and `next` may end a block. */
function mayEnd(prev: string, next: string, inList: boolean): boolean {
  // Indented: an indented code block, or a list item's continuation.
  if (/^(?: {4}|\t)/.test(next)) return false;
  // A list goes on (a loose list, a nested item, an item's later paragraph).
  if (inList && (LIST_ITEM.test(next) || /^[ \t]/.test(next))) return false;
  // A table row or a block quote right after one.
  if (/^ {0,3}\|/.test(next) && prev.includes('|')) return false;
  if (/^ {0,3}>/.test(next) && /^ {0,3}>/.test(prev)) return false;
  return true;
}

/** Cuts a streaming reply into finished blocks and one open block (the last). Joined, they equal `text`. */
export function splitStreamBlocks(text: string): string[] {
  if (!text) return [];
  if (DEFINITION.test(text)) return [text];
  const blocks: string[] = [];
  let start = 0; // where the open block starts
  let fence: { ch: string; len: number; indent: number } | null = null;
  let inList = false; // the open block holds a list item
  let prev = ''; // the open block's last non-blank line
  let blank = false; // a blank line came after `prev`
  let pos = 0;
  while (pos < text.length) {
    const nl = text.indexOf('\n', pos);
    const complete = nl >= 0;
    const line = complete ? text.slice(pos, nl) : text.slice(pos);
    if (fence) {
      const c = complete ? FENCE_CLOSE.exec(line) : null;
      if (c && c[2][0] === fence.ch && c[2].length >= fence.len && indentOf(line) <= fence.indent + 3) fence = null;
      prev = line;
    } else if (line.trim() === '') {
      if (complete) blank = true;
    } else {
      // A blank line, then this line: the block before may be finished.
      if (blank && prev && (complete || decidable(line)) && mayEnd(prev, line, inList)) {
        blocks.push(text.slice(start, pos));
        start = pos;
        inList = false;
      }
      blank = false;
      const f = FENCE_OPEN.exec(line);
      // A backtick fence's info string has no backtick (``` `x` ``` is inline code, not a fence).
      if (f && complete && !(f[2][0] === '`' && f[3].includes('`'))) fence = { ch: f[2][0], len: f[2].length, indent: f[1].length };
      if (LIST_ITEM.test(line)) inList = true;
      prev = line;
    }
    if (!complete) break;
    pos = nl + 1;
  }
  blocks.push(text.slice(start));
  return blocks;
}
