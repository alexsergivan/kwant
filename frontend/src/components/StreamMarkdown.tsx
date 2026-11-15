import React from 'react';
import { MarkdownRenderer } from './MarkdownRenderer';
import { splitStreamBlocks } from '../streamBlocks';

// A paragraph that ends a finished block keeps its usual gap below (MarkdownRenderer gives the last paragraph
// `last:mb-0`), so the blocks read exactly as one text would. Other margins collapse across the blocks.
const BLOCKS = '[&>.markdown-content:not(:last-child)>p:last-child]:mb-3';

/**
 * The live reply's markdown (15a). Finished blocks render once each, memoized: their content never changes
 * again, so React skips them. Only the open block at the end is parsed again each frame, and only it carries
 * the typing cursor. The final message
 * renders whole through MarkdownRenderer, so any difference in how the text was cut is gone when it lands.
 */
export const StreamMarkdown = React.memo(function StreamMarkdown({ content }: { content: string }) {
  const blocks = splitStreamBlocks(content);
  return (
    <div className={BLOCKS} data-stream-blocks={blocks.length}>
      {blocks.map((block, i) => (
        <MarkdownRenderer key={i} content={block} isStreaming={true} cursor={i === blocks.length - 1} />
      ))}
    </div>
  );
});
