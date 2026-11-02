# Smart Results (0.6.5)

A bot's final reply can carry one typed block, `results`, that the chat draws in place of a table
or a list in the reply: a comparison card for a table, a checklist for numbered steps. The reply's
text is never changed. When there is no block, or the chat can't draw it, the reply shows its
markdown as it always has.

Smart Results always runs. There is no setting. The release fallback is one server constant,
`smart_results::ENABLED_TYPES` (`src/server/smart_results/mod.rs`): when a type is taken out of
it, kwant stops storing blocks of that type and stops serving the ones already stored.

Code: `src/server/smart_results/` (contract types, selectors, checks) and
`src/db/smart_results.rs` (storage and serving).

## When a reply gets a block

- **Which messages qualify:** every bot reply, meaning `sender_role` is `"kwant"` and there is no
  `meta.kind`, or the kind is `delegation_result` (a teammate's hand-off result: its reply, drawn
  as markdown inside the hand-off card). Other cards and kwant's own posts have a `meta.kind` and
  never get a block. A reply's own meta (`ran_on`, usage, routing, `decide_first`) doesn't stop it
  from qualifying.
- **When it is worked out:** once, when the message is saved (`insert_message_on`), never while
  the reply is streaming. The selectors are deterministic: no model calls and no network. A reply
  longer than 128 KB stays text.
- **How the block is chosen:** the candidates are offered in priority order, every comparison
  table first, then every steps list, each in the order it appears in the text. The first candidate
  whose type is in `ENABLED_TYPES` and that passes the checks is stored, and only that one: a
  message has at most one block.
- **What it never costs:** the reply. If a selector panics, a block fails its checks, or the write
  fails, kwant logs it and saves the message exactly as before.
- **Edits:** when a reply's text is replaced (`set_message_text`), its block is deleted, because the
  block was copied from the old text.

## Storage and API

- **Table:** `message_results(message_id PK → messages.id ON DELETE CASCADE, version, payload, created_at)`.
  The migration is additive (`CREATE TABLE IF NOT EXISTS`). The payload is JSON of at most 64 KB.
- **Field:** every message object has a `results` field. Its value is the block set, or `null`
  when there is none. Messages from before 0.6.5 have no row, so they return `null`.
- **Where `results` is set:**
  - the message returned on insert, and so the `MessageAdded` broadcast for that message;
  - `GET /api/channels/:id/messages`, which covers channels and direct chats, including the
    `around` and `before` pages;
  - `GET /api/threads/:id/messages`;
  - the message re-broadcast after `POST /api/messages/:id/decisions/:index/change`.

  Other endpoints that return messages leave it `null`. The chat reloads the list at
  `turn_ended`, and that reload is what delivers a final reply's block.
- **Checks when serving:** a stored block is checked again every time it is read. An unknown
  `version`, a type this build doesn't know or no longer enables, or a payload that fails today's
  checks is served as `null`.

## Contract (payload version 1)

```ts
type ResultSet = { version: 1, blocks: [Block] }     // exactly one block when there is a row
type Source = { kind: "markdown_table" | "markdown_list", index: number }
type Provenance = { kind: "reply_text" }

type Comparison = {
  id: "b0", type: "comparison", source: Source /* markdown_table */, provenance: Provenance,
  columns: { key: "c0".., label: string, numeric: boolean, unit: string | null }[],  // 2–8
  rows: { id: "r0".., cells: string[] }[],                                           // 3–50
  highlight_row: string | null, highlight_reason: string | null,
}
type Steps = {
  id: "b0", type: "steps", source: Source /* markdown_list */, provenance: Provenance,
  items: { id: "s0".., text: string }[],                                             // 3–15
}
```

These are the v1 shapes. Plan v2 (§A.3) adds fields to them: `title`, column `role`, and the
steps `variant`, `done`, `when` and `owner`. The client reads a missing field with its v1 meaning
(`title` null, role `label` for the first column and `value` for the rest, variant `steps`).
`smart-results-more` writes these fields.

### Values are copied, never made up

- Every label, unit, cell and step is a substring of the reply. The store-time check rejects any
  value that isn't.
- Cells are the raw markdown between the pipes, trimmed. Emphasis, code spans, links and escaped
  pipes (`\|`) are kept as written; the chat renders them.
- Limits: a header or cell is at most 200 characters, a step at most 300, a unit at most 10.
  Nothing may contain HTML (`<` followed by a letter, `/`, `!` or `?`).
- **`numeric`:** at least 80% of the column's cells are a number with the same affix. The affix
  is a prefix of up to 4 letters or currency signs (`EUR 12`, `$12`), or a suffix of up to 10
  (`12 GB`, `-4%`, `4.5/5`). That shared affix is the `unit`; no affix means `unit: null`.
  Numbers may use `,` or `.` thousands grouping. Blocks of this version carry no parsed number
  values.
- **`highlight_row`:** set only when the prose outside the table names exactly one row's
  first-column label in a recommending sentence. A recommending sentence contains "recommend…",
  "best", "pick", "choose", "winner" or "go with", is not a question, and has no hedge or negative
  word ("not", "but", "if", "unless", "instead", "depends", …).
  - A name only counts as a whole name: `A` is not `A+`, `Pro` inside `Pro Max` names `Pro Max`,
    `C` is not `C++`, `Node` is not `Node.js`.
  - Labels shorter than 4 characters match case for case. A lone `A` or `I` opening a sentence is
    the word.
  - When the pick is unclear, no row is highlighted. That includes very long recommending prose
    (over 8,000 characters) and labels found over 1,000 times. `highlight_reason` is always
    "named as the pick in this reply".

## Selectors

**Comparison:**
- the table is a clean top-level GFM pipe table;
- it has 2–8 columns and 3–50 rows, every row with as many cells as the header;
- no header or first cell is empty, and first cells don't repeat (ignoring case and emphasis).

**Steps:**
- the list is a clean top-level ordered list that starts at 1, with 3–15 single-line items;
- no item is a task (`[ ]`, `[x]`) and none contains HTML;
- either every item starts with "Step N", or the list is introduced as steps: the line just above
  it, or the nearest heading above it, says "step(s)", "plan", "checklist", "todo", "how to" or
  "to do";
- the introduction doesn't call it reasons, tips, examples, ideas, options, pros, cons, prices or
  tiers.

## How `source.index` is counted

`index` must name the same table or list the chat renders with react-markdown and remark-gfm.

- **Tables:** every GFM table in document order, from 0. A table inside fenced or indented code,
  or in raw HTML, is not a table and isn't counted.
- **Lists:** every ordered list in pre-order, nested ones included, from 0; bullet lists aren't
  counted. An ordered task list counts as an ordered list, but it isn't offered as steps in this
  version.
- **Uncertainty:** the server reads markdown line by line. Some constructs could make its count
  differ from the chat's: a table or numbered list inside a quote or a bullet item, a list nested
  in a list, raw HTML, tabs, or a table run straight into a paragraph. From the first such
  construct on, kwant offers no block for that kind, and the reply stays text. Candidates before
  that point are still offered.

The client checks every block again: the labels, the row and item counts, and the cells
(letters and digits only). On any mismatch it shows the markdown.

## Adding a block type

1. Add a `Block` variant and its `kind()` arm (`mod.rs`).
2. Add a selector to `candidates()`, in the priority order of plan §A.2.
3. Add its checks to `validate()`.
4. Add the type to `ENABLED_TYPES`.
5. Add tests in `selector_tests.rs`. Each test must cover the cases where the selector abstains,
   not only the ones where it fires.
