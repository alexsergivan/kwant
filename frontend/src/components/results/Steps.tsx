import React, { useId, useState } from 'react';
import { StepsBlock, plainText, readDoneSteps, saveDoneSteps } from '../../smartResults';
import type { RendererProps } from './registry';

const boxClass =
  'mt-[5px] h-3.5 w-3.5 shrink-0 cursor-pointer rounded-sm accent-sky-500 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-500/70 focus-visible:ring-offset-1 focus-visible:ring-offset-zinc-900';

/**
 * A reply's steps (numbered, tickable), checklist (ticks start from the reply's own [x]) or timeline (a
 * rail in the list's marker gutter). Each keeps the streamed list's size. Ticks stay on this device.
 */
export const Steps: React.FC<RendererProps<StepsBlock>> = ({ block, messageId, reply }) => {
  const list = reply?.kind === 'list' ? reply : null;
  // A slot block draws its items as plain text.
  const items = block.items.map((it, i) => list?.items[i]?.node ?? plainText(it.text));
  const start = list?.start ?? 1;
  const [done, setDone] = useState(() => readDoneSteps(messageId, block));
  const uid = useId();

  const toggle = (id: string) => {
    const next = new Set(done);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setDone(next);
    saveDoneSteps(messageId, block, next);
  };

  if (block.variant === 'timeline') {
    return (
      <ol role="list" className="my-2.5 space-y-1 pl-5 text-zinc-300" data-smart-result="steps" data-variant="timeline" data-block-id={block.id}>
        {block.items.map((it, i) => {
          const last = i === block.items.length - 1;
          return (
            <li key={it.id} className="relative leading-relaxed pl-0.5">
              {/* The rail sits where the list's bullets were, so nothing moves. */}
              <span aria-hidden="true" className="absolute -left-[13px] top-[7px] h-2 w-2 rounded-full border border-sky-400/70 bg-zinc-950" />
              {!last && <span aria-hidden="true" className="absolute -left-[10px] top-[17px] -bottom-[11px] w-px bg-zinc-700" />}
              {list ? (
                items[i]
              ) : (
                <>
                  <span className="font-medium text-zinc-200">{it.when}</span>
                  {': '}
                  {items[i]}
                  {it.owner && <span className="text-zinc-500"> (owner: {it.owner})</span>}
                </>
              )}
            </li>
          );
        })}
      </ol>
    );
  }

  const numbered = block.variant === 'steps';
  const List = list && !list.ordered ? 'ul' : 'ol';
  return (
    <div className="my-2.5" data-smart-result="steps" data-variant={block.variant} data-block-id={block.id}>
      <List role="list" className={`space-y-1 text-zinc-300 ${numbered ? '' : 'pl-1'}`}>
        {block.items.map((item, i) => {
          const id = `${uid}-${item.id}`;
          const on = done.has(item.id);
          return (
            <li key={item.id} className="flex items-start gap-2 leading-relaxed">
              <input id={id} type="checkbox" checked={on} onChange={() => toggle(item.id)} className={boxClass} />
              <label
                htmlFor={id}
                className={`min-w-0 cursor-pointer [overflow-wrap:anywhere] motion-safe:transition-colors ${on ? 'text-zinc-500 line-through decoration-zinc-600' : ''}`}
              >
                {numbered && <span className="mr-1 tabular-nums text-zinc-500">{start + i}.</span>}
                {items[i]}
              </label>
            </li>
          );
        })}
      </List>
      <span className="sr-only" aria-live="polite">
        {done.size} of {block.items.length} done
      </span>
    </div>
  );
};
