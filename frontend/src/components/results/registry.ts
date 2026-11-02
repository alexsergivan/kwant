// Smart Results registry: block type → { validate, matches?, Component }. A type that isn't here, a block
// its validator rejects, or one that doesn't match the reply's own table or list renders the reply's
// markdown instead. Each renderer lives in its own file in this folder and exports its validator.
//
// Adding a renderer (plan §A.6):
//   1. results/<Type>.tsx exports `valid<Type>(raw: unknown): <Type>Block | null` (build it from the
//      helpers in smartResults.ts: parseCommon, isText, optText, inRange, uniqueIds, isNum, isHttpUrl)
//      and a component taking RendererProps<<Type>Block>.
//   2. Add one line to REGISTRY below.
//   3. Fixtures for every type are in results/fixtures.ts.

import type React from 'react';
import type { BlockOf, BlockType, SmartBlock } from '../../smartResults';
import { matchKey, validComparison, validSteps } from '../../smartResults';
import { Comparison } from './Comparison';
import { Steps } from './Steps';
import { Chart, validChart } from './Chart';
import { Metrics, validMetrics } from './Metrics';
import { Facts, validFacts } from './Facts';
import { Cards, validCards } from './Cards';
import { Findings, validFindings } from './Findings';
import { Verification, validVerification } from './Verification';
import { Files, validFiles } from './Files';

/** One cell or list item of the reply as react-markdown drew it, plus its plain text for matching. */
export interface ReplyPart {
  node: React.ReactNode;
  text: string;
}

/** The reply's own table or list that a block replaces in place. */
export type ReplySource =
  | { kind: 'table'; head: ReplyPart[]; body: ReplyPart[][] }
  | { kind: 'list'; items: ReplyPart[]; start: number; ordered: boolean; task: boolean };

export interface RendererProps<B extends SmartBlock = SmartBlock> {
  block: B;
  messageId: string;
  /** The table or list this block replaces; null for a `slot` block (drawn after the reply). */
  reply: ReplySource | null;
  /** "Ask about this": puts a prefilled question in the composer. It never sends. */
  ask: (text: string) => void;
}

export interface Renderer<B extends SmartBlock> {
  validate: (raw: unknown) => B | null;
  /** Does the block stand for this table or list? Defaults to the same number of rows or items. */
  matches?: (block: B, reply: ReplySource) => boolean;
  Component: React.ComponentType<RendererProps<B>>;
}

export const sameText = (part: ReplyPart | undefined, text: string) => !!part && matchKey(part.text) === matchKey(text);

const comparisonMatches = (block: BlockOf<'comparison'>, reply: ReplySource) =>
  reply.kind === 'table' &&
  reply.head.length === block.columns.length &&
  reply.body.length === block.rows.length &&
  reply.head.every((th, j) => sameText(th, block.columns[j].label)) &&
  reply.body.every((cells, i) => cells.length === block.columns.length && cells.every((td, j) => sameText(td, block.rows[i].cells[j])));

const stepsMatches = (block: BlockOf<'steps'>, reply: ReplySource) => {
  if (reply.kind !== 'list' || reply.items.length !== block.items.length) return false;
  // A task list is a checklist and only a checklist; numbered steps need a numbered list.
  if (reply.task !== (block.variant === 'checklist')) return false;
  if (block.variant === 'steps' && !reply.ordered) return false;
  return reply.items.every((li, i) => {
    const it = block.items[i];
    return sameText(li, it.text) || (!!it.when && sameText(li, `${it.when} ${it.text}`));
  });
};

export const REGISTRY: { [T in BlockType]?: Renderer<BlockOf<T>> } = {
  comparison: { validate: validComparison, matches: comparisonMatches, Component: Comparison },
  steps: { validate: validSteps, matches: stepsMatches, Component: Steps },
  chart: { validate: validChart, Component: Chart },
  metrics: { validate: validMetrics, Component: Metrics },
  facts: { validate: validFacts, Component: Facts },
  cards: { validate: validCards, Component: Cards },
  findings: { validate: validFindings, Component: Findings },
  verification: { validate: validVerification, Component: Verification },
  files: { validate: validFiles, Component: Files },
};

const count = (block: SmartBlock): number | null => {
  const b = block as { rows?: unknown[]; points?: unknown[]; items?: unknown[] };
  return (b.points ?? b.rows ?? b.items)?.length ?? null;
};

/** Same number of rows (table) or items (list) as the block. */
export const defaultMatches = (block: SmartBlock, reply: ReplySource) =>
  reply.kind === 'table' ? reply.body.length === count(block) : reply.items.length === count(block);

/** The checked block and its renderer, or null: then the reply shows its markdown. */
export function resolveBlock(raw: unknown, reply: ReplySource | null): { block: SmartBlock; renderer: Renderer<SmartBlock> } | null {
  const type = (raw as { type?: unknown } | null)?.type;
  if (typeof type !== 'string' || !Object.prototype.hasOwnProperty.call(REGISTRY, type)) return null;
  const renderer = REGISTRY[type as BlockType] as unknown as Renderer<SmartBlock> | undefined;
  const block = renderer?.validate(raw);
  if (!renderer || !block) return null;
  const src = block.source.kind;
  if (!reply) return src === 'slot' ? { block, renderer } : null;
  if ((src === 'markdown_table') !== (reply.kind === 'table') || src === 'slot') return null;
  return (renderer.matches ?? defaultMatches)(block, reply) ? { block, renderer } : null;
}
