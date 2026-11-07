import React, { useState } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { Copy, Check, Terminal } from 'lucide-react';
import { playSfx } from '../audio/chiptune';
import { HastNode, blockAt, hastText, rehypeSmartIndex, rehypeSmartSection, resultBlocks, sectionBlock } from '../smartResults';
import type { ReplyPart, ReplySource } from './results/registry';
import { SmartResult } from './results/SmartResult';
import { ReplyText } from './results/ReplyText';

interface CodeBlockProps {
  language?: string;
  value: string;
}

const CodeBlock: React.FC<CodeBlockProps> = ({ language, value }) => {
  const [copied, setCopied] = useState(false);

  const handleCopy = () => {
    navigator.clipboard.writeText(value);
    setCopied(true);
    playSfx('click');
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <div className="my-3.5 rounded-xl border border-zinc-800 bg-zinc-950 overflow-hidden shadow-md">
      <div className="flex items-center justify-between px-3.5 py-1.5 bg-zinc-900/90 border-b border-zinc-800 text-[11px] font-mono text-zinc-400 select-none">
        <div className="flex items-center gap-1.5">
          <Terminal size={12} className="text-zinc-500" />
          <span>{language || 'text'}</span>
        </div>
        <button
          type="button"
          onClick={handleCopy}
          className="flex items-center gap-1 px-2 py-0.5 rounded text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800 transition-colors cursor-pointer"
          title="Copy code"
        >
          {copied ? (
            <>
              <Check size={12} className="text-emerald-400" />
              <span className="text-emerald-400">Copied</span>
            </>
          ) : (
            <>
              <Copy size={12} />
              <span>Copy</span>
            </>
          )}
        </button>
      </div>
      <pre className="p-3.5 overflow-x-auto text-xs font-mono text-zinc-200 leading-relaxed">
        <code>{value}</code>
      </pre>
    </div>
  );
};

// Formats mentions inside string text nodes
const formatTextMentions = (content: React.ReactNode): React.ReactNode => {
  if (typeof content !== 'string') {
    return content;
  }
  const parts = content.split(/(@[a-zA-Z0-9_-]+)/g);
  if (parts.length === 1) {
    return content;
  }
  return parts.map((part, idx) => {
    if (part.startsWith('@') && /^@[a-zA-Z0-9_-]+$/.test(part)) {
      return (
        <span
          key={idx}
          className="inline-flex items-center px-1.5 py-0.5 mx-0.5 rounded text-sky-400 bg-sky-500/10 font-medium font-mono text-[11px] border border-sky-500/20"
        >
          {part}
        </span>
      );
    }
    return part;
  });
};

interface MarkdownRendererProps {
  content: string;
  className?: string;
  isStreaming?: boolean;
  /** The message's `results` (Smart Results). Only for a final bot reply. */
  results?: unknown;
  messageId?: string;
}

// Smart Results: hand the reply's own rendered cells or items to the block, which checks them against
// its data (results/registry.ts). Any mismatch and the table or list renders as plain markdown.
type MdElement = React.ReactElement<{ node?: HastNode; children?: React.ReactNode }>;
const elementsOf = (children: React.ReactNode) => React.Children.toArray(children).filter(React.isValidElement) as MdElement[];
const tagOf = (el: MdElement) => el.props.node?.tagName ?? (typeof el.type === 'string' ? el.type : '');
const shown = (el: MdElement) => React.Children.map(el.props.children, formatTextMentions);
const part = (el: MdElement, node: React.ReactNode = shown(el)): ReplyPart => ({ node, text: hastText(el.props.node) });
const hasClass = (n: HastNode | undefined, c: string) => {
  const cls = n?.properties?.className;
  return Array.isArray(cls) ? cls.includes(c) : cls === c;
};
const isTaskItem = (n: HastNode | undefined) => hasClass(n, 'task-list-item');

const checkClass = 'mt-[5px] h-3.5 w-3.5 shrink-0 rounded-sm accent-sky-500';

/** A task item's content without its checkbox or the space after it. */
function taskContent(children: React.ReactNode): React.ReactNode[] {
  const out = React.Children.toArray(children).filter((c) => !(React.isValidElement(c) && tagOf(c as MdElement) === 'input'));
  if (typeof out[0] === 'string') out[0] = out[0].replace(/^\s+/, '');
  return out;
}

function tableSource(children: React.ReactNode): ReplySource | null {
  let head: ReplyPart[] | null = null;
  const body: ReplyPart[][] = [];
  for (const section of elementsOf(children)) {
    const tag = tagOf(section);
    if (tag !== 'thead' && tag !== 'tbody') return null;
    for (const tr of elementsOf(section.props.children)) {
      if (tagOf(tr) !== 'tr') return null;
      const cells = elementsOf(tr.props.children).map((el) => part(el));
      if (tag === 'thead') {
        if (head) return null;
        head = cells;
      } else body.push(cells);
    }
  }
  return head ? { kind: 'table', head, body } : null;
}

function listSource(node: HastNode, children: React.ReactNode): ReplySource {
  const lis = elementsOf(children).filter((el) => tagOf(el) === 'li');
  const task = lis.length > 0 && lis.every((li) => isTaskItem(li.props.node));
  // A loose list wraps each item in <p>; unwrap it so the item reads as one line of text.
  const inline = (li: MdElement): React.ReactNode => {
    const kids = elementsOf(li.props.children);
    const content = task ? taskContent(shown(li)) : shown(li);
    // A tight task item's only element is its checkbox: that is not a loose item.
    if (!kids.some((el) => tagOf(el) === 'p') || kids.some((el) => tagOf(el) !== 'p' && tagOf(el) !== 'input')) return content;
    const ps = kids.filter((el) => tagOf(el) === 'p');
    return ps.flatMap((p, i) => {
      const c = task && i === 0 ? taskContent(shown(p)) : shown(p);
      return i ? [' ', c] : [c];
    });
  };
  const start = Number(node.properties?.start);
  return {
    kind: 'list',
    items: lis.map((li) => part(li, inline(li))),
    start: Number.isInteger(start) && start >= 0 ? start : 1,
    ordered: node.tagName === 'ol',
    task,
  };
}

export const MarkdownRenderer: React.FC<MarkdownRendererProps> = ({
  content,
  className = '',
  isStreaming = false,
  results,
  messageId,
}) => {
  // Never while streaming: the block appears when the final message arrives.
  const smart = !isStreaming && !!messageId && resultBlocks(results).length > 0;
  const slot = smart ? blockAt(results, { kind: 'slot' }) : null;
  // A recipe stands for a stretch of the reply: rehypeSmartSection cuts it out and leaves a placeholder.
  const section = smart ? sectionBlock(results) : null;

  const listView = (node: HastNode | undefined, children: React.ReactNode, plain: React.ReactElement) => {
    if (!smart || !node || node.data?.smartTop !== true || typeof node.data.smartIndex !== 'number') return plain;
    const raw = blockAt(results, { kind: 'list', index: node.data.smartIndex, ordered: node.tagName === 'ol' });
    if (!raw) return plain;
    return <SmartResult raw={raw} reply={listSource(node, children)} messageId={messageId as string} fallback={plain} />;
  };

  return (
    <ReplyText.Provider value={content}>
    <div className={`markdown-content text-sm leading-relaxed text-zinc-200 ${className}`}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={smart ? [rehypeSmartIndex as never, ...(section ? [rehypeSmartSection(content, results) as never] : [])] : undefined}
        components={{
          // Pre element pass-through to let CodeBlock handle layout
          pre: ({ children }) => <>{children}</>,

          // Code rendering: CodeBlock for blocks, styled pill for inline. A long inline run (a 46-character
          // identifier, `--test-threads=1`) may break anywhere rather than run past the bubble, and each
          // wrapped line keeps its own padding and radius (clone) instead of one box cut in two.
          code: ({ className: codeClassName, children, ...props }) => {
            const match = /language-(\w+)/.exec(codeClassName || '');
            const strVal = String(children).replace(/\n$/, '');
            const isMultiLine = strVal.includes('\n');

            if (match || isMultiLine) {
              return (
                <CodeBlock
                  language={match ? match[1] : ''}
                  value={strVal}
                />
              );
            }

            return (
              <code
                className="px-1.5 py-0.5 rounded-md font-mono text-[12px] bg-zinc-800/80 text-sky-300 border border-zinc-700/50 [overflow-wrap:anywhere] [box-decoration-break:clone] [-webkit-box-decoration-break:clone]"
                {...props}
              >
                {children}
              </code>
            );
          },

          // A stretch of the reply that one block stands for (a recipe). Its markdown is the fallback.
          div: ({ node, children, ...props }) => {
            const data = (node as HastNode | undefined)?.data;
            if (!section || data?.smartSection !== true || typeof data.markdown !== 'string') return <div {...props}>{children}</div>;
            const md = data.markdown;
            return (
              <SmartResult raw={section} reply={{ kind: 'section', markdown: md }} messageId={messageId as string} fallback={<MarkdownRenderer content={md} />} />
            );
          },

          // Tables
          table: ({ node, children }) => {
            const plain = (
              <div className="my-3.5 overflow-x-auto rounded-xl border border-zinc-700/60 bg-zinc-900/60 shadow-[var(--kws-shadow)]">
                <table className="w-full text-left border-collapse text-xs font-sans">
                  {children}
                </table>
              </div>
            );
            const index = (node as HastNode | undefined)?.data?.smartIndex;
            const raw = smart && typeof index === 'number' ? blockAt(results, { kind: 'table', index }) : null;
            const reply = raw ? tableSource(children) : null;
            return raw && reply ? <SmartResult raw={raw} reply={reply} messageId={messageId as string} fallback={plain} /> : plain;
          },
          thead: ({ children }) => (
            <thead className="bg-zinc-800/70 text-zinc-100 border-b border-zinc-700/60 font-medium">
              {children}
            </thead>
          ),
          tbody: ({ children }) => (
            <tbody className="divide-y divide-zinc-700/40 font-normal text-zinc-300">
              {children}
            </tbody>
          ),
          tr: ({ children }) => (
            <tr className="hover:bg-zinc-900/50 transition-colors">
              {children}
            </tr>
          ),
          th: ({ children }) => (
            <th className="px-3.5 py-2.5 text-xs font-semibold text-zinc-200 whitespace-nowrap">
              {React.Children.map(children, formatTextMentions)}
            </th>
          ),
          td: ({ children }) => (
            <td className="px-3.5 py-2.5 text-xs text-zinc-300 leading-relaxed align-top">
              {React.Children.map(children, formatTextMentions)}
            </td>
          ),

          // Headings
          h1: ({ children }) => (
            <h1 className="text-lg font-bold text-zinc-100 mt-5 mb-2.5 pb-1.5 border-b border-zinc-800 flex items-center gap-2">
              {React.Children.map(children, formatTextMentions)}
            </h1>
          ),
          h2: ({ children }) => (
            <h2 className="text-base font-semibold text-zinc-100 mt-4 mb-2 pb-1 border-b border-zinc-800/60">
              {React.Children.map(children, formatTextMentions)}
            </h2>
          ),
          h3: ({ children }) => (
            <h3 className="text-sm font-semibold text-zinc-200 mt-3 mb-1.5">
              {React.Children.map(children, formatTextMentions)}
            </h3>
          ),
          h4: ({ children }) => (
            <h4 className="text-xs font-semibold text-zinc-300 mt-2 mb-1 uppercase tracking-wider">
              {React.Children.map(children, formatTextMentions)}
            </h4>
          ),

          // Paragraphs & Text
          p: ({ children }) => (
            <p className="mb-3 last:mb-0 leading-relaxed">
              {React.Children.map(children, formatTextMentions)}
            </p>
          ),
          strong: ({ children }) => (
            <strong className="font-semibold text-zinc-100">
              {React.Children.map(children, formatTextMentions)}
            </strong>
          ),
          em: ({ children }) => (
            <em className="italic text-zinc-300">
              {children}
            </em>
          ),

          // Lists. A GFM task list keeps its boxes in the bullet's place, read-only until it becomes a
          // checklist block (results/Steps.tsx draws the same box).
          ul: ({ node, children }) => {
            const task = hasClass(node as HastNode | undefined, 'contains-task-list');
            const plain = (
              <ul className={`${task ? 'list-none pl-1' : 'list-disc list-outside pl-5'} my-2.5 space-y-1 text-zinc-300`}>
                {children}
              </ul>
            );
            return listView(node as HastNode | undefined, children, plain);
          },
          ol: ({ node, children }) => {
            const task = hasClass(node as HastNode | undefined, 'contains-task-list');
            const plain = (
              <ol className={`${task ? 'list-none pl-1' : 'list-decimal list-outside pl-5'} my-2.5 space-y-1 text-zinc-300`}>
                {children}
              </ol>
            );
            return listView(node as HastNode | undefined, children, plain);
          },
          li: ({ node, children }) =>
            isTaskItem(node as HastNode | undefined) ? (
              <li className="flex items-start gap-2 leading-relaxed">
                {React.Children.toArray(children).filter((c) => React.isValidElement(c) && tagOf(c as MdElement) === 'input')}
                <span className="min-w-0 [overflow-wrap:anywhere]">{React.Children.map(taskContent(children), formatTextMentions)}</span>
              </li>
            ) : (
              <li className="leading-relaxed pl-0.5">
                {React.Children.map(children, formatTextMentions)}
              </li>
            ),
          input: ({ node: _node, ...props }) =>
            props.type === 'checkbox' ? (
              <input type="checkbox" checked={!!props.checked} disabled readOnly className={checkClass} />
            ) : (
              <input {...props} />
            ),

          // Quotes & Dividers
          blockquote: ({ children }) => (
            <blockquote className="border-l-2 border-sky-500/70 pl-3.5 my-3 text-zinc-400 italic bg-sky-950/15 py-1.5 rounded-r-lg">
              {children}
            </blockquote>
          ),
          hr: () => <hr className="my-4 border-zinc-800" />,

          // Links
          a: ({ href, children }) => (
            <a
              href={href}
              target="_blank"
              rel="noopener noreferrer"
              className="text-sky-400 hover:text-sky-300 underline underline-offset-2 transition-colors"
            >
              {children}
            </a>
          ),
        }}
      >
        {content}
      </ReactMarkdown>
      {slot && (
        <SmartResult
          raw={slot}
          reply={null}
          messageId={messageId as string}
          fallback={null}
          renderText={(md) => <MarkdownRenderer content={md} />}
        />
      )}

      {isStreaming && (
        <span className="inline-block w-1.5 h-4 ml-0.5 align-middle bg-sky-400 animate-pulse rounded-sm" />
      )}
    </div>
    </ReplyText.Provider>
  );
};
