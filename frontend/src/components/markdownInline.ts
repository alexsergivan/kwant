// The chat's own look for a link and an inline code span, shared by MarkdownRenderer and the renderers
// that draw a reply's inline markdown themselves (results/InlineText.tsx).
export const linkClass = 'text-sky-400 hover:text-sky-300 underline underline-offset-2 transition-colors';
export const inlineCodeClass =
  'px-1.5 py-0.5 rounded-md font-mono text-[12px] bg-zinc-800/80 text-sky-300 border border-zinc-700/50 [overflow-wrap:anywhere] [box-decoration-break:clone] [-webkit-box-decoration-break:clone]';
