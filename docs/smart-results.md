# Smart Results (0.6.5)

A bot's final reply can carry one typed block, `results`, that the chat draws in place of a table
or a list in the reply: a chart, a comparison, key numbers, findings, cards, steps (plain, checklist
or timeline), facts or a recipe. The reply's
text is never changed. When there is no block, or the chat can't draw it, the reply shows its
markdown as it always has.

Smart Results always runs. There is no setting. The release fallback is one server constant,
`smart_results::ENABLED_TYPES` (`src/server/smart_results/mod.rs`): when a type is taken out of
it, kwant stops storing blocks of that type and stops serving the ones already stored.

Code: `src/server/smart_results/` (contract types, selectors, checks) and
`src/db/smart_results.rs` (storage and serving).

## When a reply gets a block

- **Which messages qualify:** two kinds of card (task reports and shared files, below), and every bot reply, meaning `sender_role` is `"kwant"` and there is no
  `meta.kind`, or the kind is `delegation_result` (a teammate's hand-off result: its reply, drawn
  as markdown inside the hand-off card). Other cards and kwant's own posts have a `meta.kind` and
  never get a block. A reply's own meta (`ran_on`, usage, routing, `decide_first`) doesn't stop it
  from qualifying.
- **When it is worked out:** once, when the message is saved (`insert_message_on`), never while
  the reply is streaming. The selectors are deterministic: no model calls and no network. A reply
  longer than 128 KB stays text.
- **How the block is chosen:** each table gets at most one class (line chart, else bar chart unless
  the prose names a pick, else metrics, else comparison) and each list at most one (metrics,
  findings, cards, timeline, checklist, steps, facts). The candidates are then offered in the order
  of `selector::PRIORITY`: recipe > chart > comparison > metrics > findings > cards > timeline > checklist >
  steps > facts. A tie goes to the earliest in the text. The first candidate whose type is in
  `ENABLED_TYPES` and that passes the checks is stored, and only that one: a message has at most
  one block.
- **The bot prompt line:** every turn's system prompt carries `smart_results::PROMPT_LINE` once
  (in the cached tail, `drivers::TurnInput::system_prompt_parts`), asking for tables, numbered
  steps, an "Ingredients" list then numbered steps for a recipe, and `**Label:** value` lines.
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
type Source = { kind: "markdown_table" | "markdown_list" | "slot", index: number | null,
                ordered: boolean | null }  // lists: ordered or bullet; slot: no index
type Provenance =
  | { kind: "reply_text" }
  | { kind: "bot_declared", tool_call_id: string }          // present_result, grounded in the reply
  | { kind: "kwant_data", ref: "task:<id>" | "attachment:<id>" }  // verification, files

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

These are the v1 shapes. v2 (plan §A.3) adds fields to them: `title`, column `role`, and the
steps `variant` (`steps`, `checklist`, `timeline`), `done`, `when` and `owner`. The client reads a
missing field with its v1 meaning (`title` null, role `label` for the first column and `value` for
the rest, variant `steps`). v2 also adds `chart`, `metrics`, `facts`, `cards` and `findings`; the
Rust types in `src/server/smart_results/mod.rs` are the reference. Numbers (`values`, `value`) are
parsed from the cell as written, never computed; a cell without one is `null`, never 0.

### Values are copied, never made up

- Every label, unit, cell and step is a substring of the reply. The store-time check rejects any
  value that isn't.
- Cells are the raw markdown between the pipes, trimmed. Emphasis, code spans, links and escaped
  pipes (`\|`) are kept as written; the chat renders them.
- Limits: a header or cell is at most 200 characters, a step at most 300, a unit at most 10. A header or cell counts what it shows: a markdown link `[text](url)` counts as its text, a bare URL in full, and a cell holds at most 1,000 characters raw.
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

Every selector abstains when in doubt.

**Chart** (table, 1–4 series that share one unit, ≥ 80% of each series' cells numbers, the rest
gap tokens such as `-` or `n/a`):
- line: 4–60 rows and every first cell time-like (ISO date, `YYYY-MM`, month name, Q1–Q4,
  Week N, Day N, a year 1900–2100);
- bar: 5–20 rows, a text first column, 1–2 series, and no pick named in the prose; 2–4 rows stay
  a comparison.
- A series' unit is the affix its cells share, or for bare numbers the bracketed end of its
  header (`Price (EUR)`, `Rating [1-5]`). Series whose units differ are never drawn on one Y
  axis: the table is not a chart (SR-R-13). Units compare as written, so `€` and `EUR` differ.
- Numbers are read with both `,` and `.` as the decimal mark; when the readings disagree for the
  column (`1,299` alone), there is no chart.

**Metrics:** 2–6 list items `**Label:** value` or `Label: value` whose value starts with a number
(`$4.2M`, `12 ms`, `98%`, `3 of 5`), or a 2-column table of 2–6 rows with at least two different
units (one shared unit is a comparison, or a bar chart from 5 rows). Not when the list is
introduced as ingredients, a recipe, packing, a schedule or an agenda, nor for time or date labels.

**Findings:** 2–30 items that each start with a severity token (`**High:**`, `[High]`, `High:`;
Critical, Blocker, High, Major, Medium, Moderate, Low, Minor, Info, Nit, P0–P3), plus a second
signal: a lead line or heading saying review, audit, findings, issues, bugs or risks, or a
`path:line` in a code span. One item whose text starts with a number ("High: 18C") keeps the
whole list text.

**Cards:** 3–12 items starting with an `https` link or a `**Title**`, then " — ", " – " or " - "
(or ": " after a link) and a description of at least 10 characters. Link text that looks like a
different URL keeps the list text. `**Title:** text` reads as facts.

**Steps:** a timeline when every item has a time-like or clock label (3–15 items); a checklist
when every item is a task (3–20); otherwise the numbered-steps rule below. `owner` comes only from
a literal "Owner: X" or "(owner: X)".

**Facts:** 3–12 items `**Label:** value`, more than half of them not starting with a number.

**Recipe** (0.7.6; the `Recipe` type in `mod.rs` is the reference). One block for the
recipe part of a reply: the dish heading, a Serves/time line, the Ingredients lists (with group
labels), the method and optional notes. The intro and the closing question stay text.
- Exactly one ingredients cue: a heading or one-line paragraph "Ingredients" (also
  "Ingredients (serves 4)", "Ingredient list"), or a one-line intro ending in ":" with "ingredients"
  in it, right before the list. A second cue (a meal plan) gives no recipe.
- Then 1+ bullet lists of 2–40 ingredients in total (each list may sit under a short group label),
  then the method: an ordered list, or any list under Method / Instructions / Steps / Directions /
  Preparation (2–30 steps). Optional notes after it: a Notes / Tips label and a list of 1–6 items.
- Every top-level block in the section must be accounted for. A prose paragraph, code block, table,
  quote or HTML in between, a list straight after the method, a second list→numbered-list pair in
  the reply, steps that read as a weekly plan, or any list past the line where the count becomes
  unsure: no recipe.
- `serves` (1–24) and up to 3 `times` come from a meta line or list (`**Serves 4** · Prep 15 min`,
  `- **Prep time:** 10 minutes`); one the server can't read ("Serves 4–6") stays text above the card.
- `qty` is the leading amount as written, split off only when `qty + text` (with a space, or none
  when the text starts with ",") rebuilds the line exactly. A temperature ("350 °F oven", "180C",
  "350 degrees") is not an amount, so it gets no `qty` and never scales. A GFM task box in front
  of an ingredient or a note is dropped.
- Ranks above every other reply selector; recipe replies hold lists the steps and metrics selectors
  would otherwise take.
- Source `markdown_section` `{index, ordered, last_index, last_ordered, lead}`: the first and last
  list of the section, and how many top-level nodes before the first list belong to it. Only a
  recipe may use it, and a recipe may use nothing else.

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
- **Lists:** two counters, each from 0: numbered lists and bullet lists (`ordered: false`), in
  pre-order, nested ones included. A task list counts in its own kind. A change of marker (`-` to
  `*`, `.` to `)`) starts a new list. `slot` blocks have no index and shift nothing.
- **Uncertainty:** the server reads markdown line by line. Some constructs could make its count
  differ from the chat's: a table or list inside a quote or an item, a list nested in a list, raw
  HTML, tabs, footnotes, a table run straight into a paragraph, or a leftover `[DECISIONS]` tag
  (the whole reply). From the first such
  construct on, kwant offers no block for that kind, and the reply stays text. Candidates before
  that point are still offered.

The client checks every block again: the labels, the row and item counts, and the cells
(letters and digits only). On any mismatch it shows the markdown.

## Typed blocks (smart-results-typed)

Three more sources feed the same one-block-per-message rule. All of them go through `validate()`
and the `ENABLED_TYPES` gate (`compose.rs` orders them).

**`verification`** is attached to a task's report line (`meta` `{"kind": "mission", "task_id"}`),
read from the task's stored outcome checks on the same connection as the insert, never from the
text. One to 20 checks (none, or more than 20, gives no block). Each check has `label` (its kind
and main field), `kind`, `status` (`passed`, `failed`, `error`, or `pending` for anything not yet
finished), a redacted and cut `excerpt`, and `checked_at`. The `summary` ("2 of 5 checks passed")
counts the rows, so a pending check is never a pass. It is a snapshot taken when the report is
posted. Provenance `kwant_data` with `ref` `task:<id>`.

**`files`** is attached to the agent "Shared a file" card (`meta.kind` "file"). Each listed
attachment must be a row of this bot's own share (`source` "agent", `kwant_id` = sender, same
channel, direct chat and thread, not yet tied to a message) or the whole block is dropped. At most
10 files. Previews: CSV/TSV up to 1 MB as a table (8 columns by 20 rows, quote-aware), text and
markdown up to 20 lines, inline images as `image`; anything else is a row with no preview. A file
over 1 MB is previewed from its first megabyte (up to the last whole line); one with no whole line
in that megabyte has no preview.

A cut preview says so, so a 20-row preview is never read as the whole file. Three optional fields
sit on the preview object (`kind` `table` or `text`) and are left out of the JSON when the preview
is complete, so old fixtures stay valid:

  | field | on | meaning |
  |---|---|---|
  | `truncated: true` | table, text | the file has more rows, columns or lines than the preview shows, or was cut at the 1 MB read cap |
  | `total_rows`, `total_cols` | table | records after the header, and header width, counted exactly |
  | `total_lines` | text | lines in the file, counted exactly |

Totals come with `truncated` and only when counted exactly: a file cut at the 1 MB cap has
`truncated: true` and no totals (the renderer shows "Showing first 20 rows" without a count), and
so does a table with a row over 1,000 fields or a cell over 100 KB. A total is never below what the
preview shows. Per-cell and per-line cuts (300 characters) do not set `truncated`. Lines are counted
as a reader counts them: a final newline or trailing blank lines add none, so a 20-line file is
complete and never reports 22 lines. Example:
`{"kind":"table","columns":[...],"rows":[...20 rows],"truncated":true,"total_rows":143,"total_cols":5}`
shows as "Showing 20 of 143 rows". Bytes
are read only through `attachments::blob_path` (a sha256 hex string under the attachments
directory). A message of only inline images gets no block, since the chat draws them already. The
block carries ids and previews only; downloads go through the existing authorized attachment
routes.

**`present_result`** is a self tool (offered in `tool_list`, dispatched in `call_tool`). A bot
calls it once before its final reply to say which table or list it wants drawn: `type`
(`comparison`, `steps` or `recipe`), `source` (`markdown_table` or `markdown_list` and the `index`), `data`.
Nothing is trusted at call time. When the message is saved, the last call that was valid (status
`ok` or `allowed`) is held to the final reply: the comparison's column labels are in the table's
header, every declared row matches one row of that table cell by cell (a value moved under another
row is refused), numbers compare by value ("1800" is "1,800"), URLs must be links in the reply
(no plain-text URL, no login in the URL, no `javascript:`/`data:`/`file:`), steps quote items of
the named list. Any failure drops the call (logged) and the normal selectors run on the reply. A
call without `source` is refused for a comparison (and for steps). A bot cannot declare
`verification` or `files`, and cannot set provenance: kwant writes `bot_declared` with the call's id.
Calls over 64 KB are refused.
A `recipe` call has no `source` and `data: {ingredients: [full lines], steps: [texts]}`; it grounds
only when the reply's own recipe has the same ingredients and steps in the same order, and the
stored block is the one read from the reply, with `bot_declared` provenance.

Not yet declarable: `chart`, `metrics`, `facts`, `cards`, `findings`. They arrive with
`smart-results-more`; for metrics the plan's rule applies then (each label and its value must be on
the same line of the reply).

## Adding a block type

1. Add a `Block` variant and its `kind()` arm (`mod.rs`).
2. Add a selector to `candidates()` and the type to `PRIORITY` (plan §A.4).
3. Add its checks to `validate()`.
4. Add the type to `ENABLED_TYPES`.
5. Add tests in `block_tests.rs`. Each test must cover the cases where the selector abstains,
   not only the ones where it fires.
