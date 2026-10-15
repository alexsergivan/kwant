import React, { useState } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { Copy, Check, Terminal } from 'lucide-react';
import { playSfx } from '../audio/chiptune';

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
}

export const MarkdownRenderer: React.FC<MarkdownRendererProps> = ({
  content,
  className = '',
  isStreaming = false,
}) => {
  return (
    <div className={`markdown-content text-sm leading-relaxed text-zinc-200 ${className}`}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          // Pre element pass-through to let CodeBlock handle layout
          pre: ({ children }) => <>{children}</>,

          // Code rendering: CodeBlock for blocks, styled pill for inline
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
                className="px-1.5 py-0.5 rounded-md font-mono text-[12px] bg-zinc-800/80 text-sky-300 border border-zinc-700/50"
                {...props}
              >
                {children}
              </code>
            );
          },

          // Tables
          table: ({ children }) => (
            <div className="my-3.5 overflow-x-auto rounded-xl border border-zinc-800 bg-zinc-950/70 shadow-sm">
              <table className="w-full text-left border-collapse text-xs font-sans">
                {children}
              </table>
            </div>
          ),
          thead: ({ children }) => (
            <thead className="bg-zinc-900/90 text-zinc-200 border-b border-zinc-800 font-medium">
              {children}
            </thead>
          ),
          tbody: ({ children }) => (
            <tbody className="divide-y divide-zinc-800/60 font-normal text-zinc-300">
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

          // Lists
          ul: ({ children }) => (
            <ul className="list-disc list-outside pl-5 my-2.5 space-y-1 text-zinc-300">
              {children}
            </ul>
          ),
          ol: ({ children }) => (
            <ol className="list-decimal list-outside pl-5 my-2.5 space-y-1 text-zinc-300">
              {children}
            </ol>
          ),
          li: ({ children }) => (
            <li className="leading-relaxed pl-0.5">
              {React.Children.map(children, formatTextMentions)}
            </li>
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

      {isStreaming && (
        <span className="inline-block w-1.5 h-4 ml-0.5 align-middle bg-sky-400 animate-pulse rounded-sm" />
      )}
    </div>
  );
};
