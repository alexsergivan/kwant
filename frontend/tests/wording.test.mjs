// Wording (Spaces IA §5): visible text says "kwant bots" in headings and "bot" in sentences,
// never a bare "kwant", "kwnt" or "agent". Scans string literals and JSX text in src/ with the
// TypeScript parser; comments, imports, ids, keys and other single lowercase words are skipped.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';

const root = fileURLToPath(new URL('..', import.meta.url));
const src = join(root, 'src');

const files = (dir) =>
  readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? files(join(dir, e.name)) : /\.tsx?$/.test(e.name) && !e.name.endsWith('.d.ts') ? [join(dir, e.name)] : [],
  );

const BAD = /\b(kwants?|kwnt|agents?)\b/i;
// The product term (headings, tagline), the CLI commands and token file shown on the sign-in screen, and
// Agent Maestro, the VS Code extension's own name (a Custom API preset).
const ALLOWED = /\bkwant bots?\b|\b(?:exec kwnt )?kwnt token\b|~\/\.kwnt\/api-token/gi;
const MAESTRO = /\bAgent Maestro\b/g;

/** Whether a literal reads like text a person sees (a sentence, or a capitalised word). */
const looksVisible = (text, node) => {
  const p = node.parent;
  if (ts.isJsxText(node)) return true;
  if (p && ts.isJsxAttribute(p)) return /^(placeholder|title|aria-label|alt|label|description|hint)$/.test(p.name.getText());
  if (p && (ts.isImportDeclaration(p) || ts.isExportDeclaration(p) || ts.isLiteralTypeNode(p))) return false;
  if (p && ts.isPropertyAssignment(p) && p.name === node) return false;
  if (p && ts.isElementAccessExpression(p)) return false;
  // console.* and test-only ids
  for (let a = p; a; a = a.parent) {
    if (ts.isCallExpression(a) && /^console\./.test(a.expression.getText())) return false;
    if (ts.isCallExpression(a) && /^(localStorage|sessionStorage)\./.test(a.expression.getText())) return false;
  }
  return /\s/.test(text.trim()) || /^[A-Z]/.test(text);
};

export function findBareWording() {
  const hits = [];
  for (const file of files(src)) {
    const text = readFileSync(file, 'utf8');
    const sf = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, file.endsWith('.tsx') ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
    const visit = (node) => {
      let s = null;
      if (ts.isJsxText(node)) s = node.text;
      else if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) s = node.text;
      else if (ts.isTemplateHead(node) || ts.isTemplateMiddle(node) || ts.isTemplateTail(node)) s = node.text;
      if (s !== null) {
        const stripped = s.replace(MAESTRO, '').replace(ALLOWED, '');
        // URLs, paths, event names and storage keys (kwnt:..., kwant.x) are not prose.
        // Example paths a person reads (~/dev/x) stay in.
        const prose = stripped.replace(/(?<!\S)(?!~)\S*(?:[:./_#=]\w)\S*/g, (w) => (/^[A-Za-z]+[.,;:!?)]?$/.test(w) ? w : ''));
        const at = ts.isTemplateLiteralToken(node) ? node.parent.parent ?? node : node;
        if (BAD.test(prose) && looksVisible(s, at)) {
          const { line } = sf.getLineAndCharacterOfPosition(node.getStart());
          hits.push(`${relative(root, file)}:${line + 1}: ${s.trim().replace(/\s+/g, ' ').slice(0, 120)}`);
        }
      }
      ts.forEachChild(node, visit);
    };
    visit(sf);
  }
  return hits;
}

test('no visible text says a bare "kwant", "kwnt" or "agent"', () => {
  assert.deepEqual(findBareWording(), []);
});

test('the page title says "kwant bots"', () => {
  const title = readFileSync(join(root, 'index.html'), 'utf8').match(/<title>([^<]*)<\/title>/)?.[1] ?? '';
  assert.match(title, /kwant bots/);
  assert.doesNotMatch(title.replace(/\bkwant bots?\b/gi, ''), BAD);
});

if (process.argv.includes('--list')) console.log(findBareWording().join('\n'));
