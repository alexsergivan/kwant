import React from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { inlineCodeClass, linkClass } from '../markdownInline';

// Only what can sit inside one line of a reply: the paragraph wrapper is dropped, anything else (a
// heading, a list, raw HTML) keeps just its text. Raw HTML is never drawn (react-markdown skips it).
const INLINE = ['p', 'a', 'code', 'strong', 'em', 'del', 'br'];

/**
 * One line of a reply as the chat draws it (F-1): links stay links (the chat's own new-tab link), code
 * stays code, bold and italics stay (in the line's own colour, so a done step dims all of it). A leading "1." "-" "#" or ">" is the reply's text, not a new block.
 */
export const InlineText: React.FC<{ md: string }> = ({ md }) => (
  <ReactMarkdown
    remarkPlugins={[remarkGfm]}
    allowedElements={INLINE}
    unwrapDisallowed
    components={{
      p: ({ children }) => <>{children}</>,
      a: ({ href, children }) => (
        <a href={href} target="_blank" rel="noopener noreferrer" className={linkClass}>
          {children}
        </a>
      ),
      code: ({ children }) => <code className={inlineCodeClass}>{children}</code>,
      strong: ({ children }) => <strong className="font-semibold">{children}</strong>,
      em: ({ children }) => <em className="italic">{children}</em>,
    }}
  >
    {escapeBlockStart(md)}
  </ReactMarkdown>
);

/** "1. Mix" or "- salt" as one line of text: escape the marker that would start a list, heading or quote. */
export const escapeBlockStart = (md: string) => md.replace(/^(\s*)(#{1,6}|>|[-+*]|\d{1,9}[.)])(?=\s|$)/, (_all, sp: string, mark: string) => sp + mark.replace(/^(\d+)([.)])$/, '$1\\$2').replace(/^([#>+*-])/, '\\$1'));
