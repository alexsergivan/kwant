// Copy data (Markdown, CSV, JSON) for any block. Comparison and steps have their own shapes; every other
// type exports its item list (or points, or checks) with one column per plain field. A renderer can
// override this with `markdown` / `csv` in its registry entry.

import type { SmartBlock } from '../../smartResults';
import { joinLine, recipeMarkdown } from '../../recipe';
import { comparisonCsv, csvCell, plainText, previewNote, stepsMarkdown } from '../../smartResults';

type Row = Record<string, unknown>;

/** The block's rows as flat records: the thing Copy data copies. */
export function blockRows(block: SmartBlock): Row[] {
  switch (block.type) {
    case 'comparison':
      return block.rows.map((r) => Object.fromEntries(block.columns.map((c, j) => [plainText(c.label), plainText(r.cells[j])])));
    case 'chart':
      return block.points.map((p) => ({
        [plainText(block.x.label)]: plainText(p.x),
        ...Object.fromEntries(block.series.map((s, k) => [plainText(s.label), plainText(p.cells[k] ?? '')])),
      }));
    case 'verification':
      return block.checks.map(({ label, kind, status, excerpt, checked_at }) => ({ check: label, kind, status, excerpt, checked_at }));
    case 'findings':
      return block.items.map(({ severity_text, title, detail, refs }) => ({
        severity: severity_text,
        title,
        detail,
        refs: refs.map((r) => (r.line == null ? r.path : `${r.path}:${r.line}`)).join(' '),
      }));
    case 'cards':
      return block.items.map(({ title, url, description, meta }) => ({ title, url, description, meta: meta.join(' · ') }));
    case 'files':
      // A cut preview says so in its own row, so pasted data never passes a partial preview off as the file.
      return block.items.map(({ name, mime, size_bytes, preview }) => {
        const note = previewNote(preview);
        return { name, type: mime, bytes: size_bytes, ...(note ? { preview: `${note}; Copy data copies the preview only` } : {}) };
      });
    case 'steps':
      return block.items.map(({ text, done, when, owner }) => ({ ...(when != null ? { when } : {}), step: text, ...(owner != null ? { owner } : {}), ...(done != null ? { done } : {}) }));
    case 'pocket':
      // A live thing, not a data set: Show as text and the render-error fallback get its facts in words.
      return pocketRows(block);
    case 'recipe':
      return block.ingredients.map((i) => ({ ...(i.group ? { group: i.group } : {}), ingredient: i.qty ? joinLine(i.qty, i.text) : i.text }));
    default:
      return block.items.map(({ id: _id, ...rest }) => rest as Row);
  }
}

const POCKET_STATE_TEXT = { writing: 'Writing', checking: 'Checking', dry_run_stopped: 'Dry run stopped', ready: 'Ready', live: 'Live', ended: 'Ended' } as const;

function pocketRows(block: Extract<SmartBlock, { type: 'pocket' }>): Row[] {
  const rows: Row[] = [{ what: 'Pocket app', value: block.name }, { what: 'State', value: `${POCKET_STATE_TEXT[block.state]}${block.paused ? ' (paused)' : ''}` }];
  if (block.when) rows.push({ what: 'When', value: block.when });
  if (block.progress) rows.push({ what: 'Progress', value: `${block.progress.done} of ${block.progress.total}` });
  if (block.checks) rows.push({ what: 'Answers checked', value: `${block.checks.matched} of ${block.checks.total}` });
  if (block.live) rows.push({ what: 'Players', value: `${block.live.players} in ${block.live.teams} teams` });
  // The join link is shown only once a dry run passed, the same rule as the QR code.
  const passed = block.state === 'live' || (block.state === 'ready' && block.dry_run?.passed === true);
  const link = passed && block.join ? (block.join.mode === 'wifi' ? block.join.wifi_url : block.join.anywhere_url) : null;
  if (link) rows.push({ what: 'Join link', value: link });
  const teams = block.results?.teams ?? [];
  teams.forEach((t, i) => rows.push({ what: `${i + 1}. ${t.name}`, value: `${t.points} points` }));
  if (!teams.length && block.ended?.winner) rows.push({ what: 'Winners', value: `${block.ended.winner.name}, ${block.ended.winner.points} points` });
  return rows;
}

const cell = (v: unknown) => (v == null ? '' : typeof v === 'string' ? plainText(v) : String(v));

export function blockCsv(block: SmartBlock): string {
  if (block.type === 'comparison') return comparisonCsv(block);
  const rows = blockRows(block);
  const keys = [...new Set(rows.flatMap((r) => Object.keys(r)))];
  return [keys, ...rows.map((r) => keys.map((k) => cell(r[k])))].map((r) => r.map(csvCell).join(',')).join('\n');
}

const mdCell = (v: unknown) => cell(v).replace(/\|/g, '\\|').replace(/\n/g, ' ');

export function blockMarkdown(block: SmartBlock, done?: Set<string>): string {
  if (block.type === 'steps') return stepsMarkdown(block, done);
  // As the reply wrote it, at its own servings.
  if (block.type === 'recipe') return recipeMarkdown(block, block.serves ?? 1);
  const rows = blockRows(block);
  const keys = [...new Set(rows.flatMap((r) => Object.keys(r)))];
  const table = [
    `| ${keys.map(mdCell).join(' | ')} |`,
    `| ${keys.map(() => '---').join(' | ')} |`,
    ...rows.map((r) => `| ${keys.map((k) => mdCell(r[k])).join(' | ')} |`),
  ].join('\n');
  const head = block.title ? `**${plainText(block.title)}**\n\n` : '';
  const tail = block.type === 'verification' || block.type === 'chart' ? (block.summary ? `\n\n${block.summary}` : '') : '';
  return head + table + tail;
}

/** The block as checked, without internal ids, for pasting into other tools. */
export const blockJson = (block: SmartBlock) => JSON.stringify(block, null, 2);
