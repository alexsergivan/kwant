import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import ts from 'typescript';

const source = await readFile(new URL('../src/discussionThreads.ts', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ESNext },
});
const { groupDiscussionThreads, stripDiscussionMarkers } = await import(
  `data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`
);

const msg = (id, sender_id, reasoning_content = null) => ({ id, sender_id, reasoning_content });

test('folds discussion turns that end in a team summary into one thread', () => {
  const items = groupDiscussionThreads([
    msg('u1', 'You'),
    msg('t1', 'TechLead', 'Team Discussion • Turn 1 • @core'),
    msg('t2', 'Architect', 'Team Discussion • Turn 2 • @core'),
    msg('t3', 'TechLead', 'Team Discussion • Turn 3 • @core'),
    msg('s1', 'TechLead', 'Team Summary • @core • with TechLead, Architect'),
  ]);
  assert.deepEqual(
    items.map((i) => (i.kind === 'thread' ? `thread:${i.turns.length}:${i.speakers.join(',')}` : i.message.id)),
    ['u1', 'thread:3:TechLead,Architect', 's1'],
  );
});

test('keeps in-progress or summary-less turns inline', () => {
  const items = groupDiscussionThreads([
    msg('t1', 'TechLead', 'Team Discussion • Turn 1 of 2 • @core'),
    msg('t2', 'Architect', 'Team Discussion • Turn 2 of 2 • @core'),
    msg('u2', 'You'),
  ]);
  assert.deepEqual(items.map((i) => i.kind), ['message', 'message', 'message']);
});

test('hides pass and ask-user markers while streaming', () => {
  assert.equal(stripDiscussionMarkers('[ASK_USER] Who is it for?'), 'Who is it for?');
  assert.equal(stripDiscussionMarkers('[PASS]'), '');
});
