# Camera-to-Completed

The person photographs or uploads a letter (a bill, a council form, a tax notice). kwant reads it, pulls out who sent it, what they
want, by when and what to quote back, and puts it in Loose ends as a case with a reply draft and deadline reminders. Nothing is ever
sent: the draft is copy or `mailto:` only. The code is `src/documents/`, `src/db/documents.rs` and `src/server/document_routes.rs`.

**Status: backend slice.** The API, reading, fields, reminders and delete are here; the UI is Pixel's. The plan is
`/home/maus/shared/ship-software-safely/camera-to-completed/plan.md`.

## API

The person only. The auth middleware lists `/api/documents/*` and `/api/zombie/*` as person-only: no token is 401 from another machine (passes on loopback unless `KWNT_REQUIRE_TOKEN=always`), a bot's turn
token or a wrong token 403.

| route | does |
|---|---|
| `POST /api/documents {attachment_ids: [id, ..]}` | 202 `{case_id}`. 1–5 uploads from `POST /api/attachments`, each PNG, JPEG, WebP or PDF, not sent anywhere yet. The letter is read in the background. |
| `GET /api/documents/:id` | `DocumentView` (below). Poll it until `extraction.status` is no longer `running`. |
| `DELETE /api/documents/:id` | `{deleted: true}`. Removes the case, the extracted text and fields, the reminders, the page rows and the page files nothing else uses. |
| `PATCH /api/zombie/cases/:id/fields {..}` | `DocumentView`. Confirms or corrects fields; letter cases only. |

Errors are `{error}` with these statuses:

- `POST`: 400 for 0 or more than 5 ids, the same id twice, an unknown id, or a file that is not a page type. 409 if a page was already
  used. 409 `{code: "busy"}` while another letter is being read: one at a time per server.
- `GET` and `DELETE`: 404 for an unknown letter. `DELETE` is 409 while the letter is being read.
- `PATCH`: 404 for an unknown case, 422 for a case that isn't a letter or has nothing read, 409 while it is being read, and 400 for a
  value that does not parse or an unknown key.

The case also shows in `GET /api/zombie/cases` from the moment it is created (`source: "document"`, `kind: "document_request"`,
`conv_kind: "document"`, title `Letter (reading…)`, then `Letter from <sender>`). Done, dismiss, reopen and the draft routes work on it
like on any case. A letter's pages under `/api/attachments` (context `document:<case id>`) need the person's token too.

```text
DocumentView {
  case: CaseView,                      // as in /api/zombie/cases
  extraction: {
    status: "running" | "read" | "text_only" | "unreadable",
    method: "pdf_text" | "model_vision" | "pdf_text+model_vision" | null,
    note: string | null,               // why it is text only or unreadable, in kwant's words
    fields: Fields,
    text: string,                      // the PDF text and the model's transcript of the photos
    read_by: "api:<model>" | "cli" | null,
    pages: [{id, name, mime, url}],    // url = /api/attachments/:id/content
    updated_at
  } | null,
  reminders: [{offset_days: 7 | 1 | 0, due_at, fired_at | null}]
}

Fields {
  sender, request, reply_to, deadline: Field | null,
  references, amounts, checklist: [Field]
}
Field { value, quote, uncertain, confirmed, auto?, done? }   // done: checklist items only
```

## Reading

1. The text layer of each PDF is read with the attachments' PDF extractor (2 at a time, 20 s each). A PDF with almost no text
   (under 20 characters) counts as a picture.
2. Photos and picture-only PDFs go to **one** model call together with the PDF text. The call has no tools: the Messages API with no
   `tools` field, or the signed-in `claude` CLI with `--tools ""`, `--strict-mcp-config`, `--safe-mode` and no session, run in an empty
   folder with the letter on stdin (never in argv). Claude only, on the Loose ends model setting when it names a Claude model. A page
   over 5 MB is skipped with a note.
3. If no model can run, or it fails, or its answer is not the JSON asked for, the letter is `text_only` (the PDF text is kept) or
   `unreadable`. The fields are left empty for the person to fill in, and the note says why.

`method` says what was used. Notes never hold the letter's or the model's words.

## Field rules

The model returns each field as `{value, quote, uncertain}`. kwant marks a field uncertain when:

- the model says so;
- the quote is empty or isn't in the letter. Quotes are matched against the PDF text and, only when photos were sent, the model's
  transcript of them;
- a deadline isn't an ISO date (`YYYY-MM-DD`);
- an amount has no currency, or its number is ambiguous ("1.234" could be 1234 or 1.234). Amounts are normalised to
  `CUR 1234.50`.

A field that is **not** uncertain is confirmed as it is read, with `auto: true`; the UI shows it with an "auto" mark, and the
person can still edit or clear it. A sure deadline therefore schedules reminders straight away. An uncertain field (and an
uncertain deadline) stays unconfirmed until the person confirms it, so no reminders run off a date the model wasn't sure of.
Fields stored before `auto` existed have no `auto` and keep their state. `PATCH /api/zombie/cases/:id/fields` takes:

- for a single field: `null` to clear it, `{}` to confirm it as it is (the value must still parse), or `{"value": "..."}` to set it
  (which also confirms it). Either one clears `auto`: the field is now the person's;
- for a list: the whole list, as `[{"value": "...", "done"?: bool, "confirmed"?: true}]`. `done` applies to checklist items only.
  A new or changed item is confirmed. An item whose value is unchanged keeps its quote and its confirmed/uncertain/auto state, so
  ticking one checklist item confirms nothing; send `"confirmed": true` on an item to confirm it as it is.

Values are at most 500 characters and lists at most 20 items.

The case takes the sender as `counterpart_name`, the first reference, and the request as `why`. It takes the deadline only when the
deadline is confirmed. The draft is an email reply (with a `mailto:`) when `reply_to` is an email address, else a `letter` draft to
the sender's name.

## Reminders

A **confirmed, not uncertain** deadline schedules reminders 7 days before, 1 day before and on the day, at 09:00 in the Loose ends
time zone. Days already past are skipped, and one due earlier today fires at once. Changing the deadline replaces the reminders;
clearing it or marking it uncertain removes them. Done or dismiss clears them, and reopen schedules the ones still ahead.

A background loop checks every 5 minutes. Per case only the nearest due reminder fires: after downtime the person gets "due tomorrow",
not "in 7 days" as well. One notification goes out per tick, kind `loose_ends`, with counts only. For example, the title is "Letter
deadlines: 3 to deal with" and the body "1 letter is due today. 2 letters have a deadline in 7 days.". It never includes a sender, an
amount or a word of the letter.

## Privacy

- Logs carry case ids, page counts, statuses and counts only. `tests/documents_test.rs` captures every log line of a full read,
  correction, reminder and delete and checks that none of the letter's words appear.
- Delete removes everything (see the API table). Loose ends retention, when it wipes a closed case, also drops its extracted text,
  fields and reminders.
- A restart in the middle of a read marks that letter `unreadable` ("kwant restarted while reading it") so it never stays `running`.

## Tests

- Unit tests in `src/documents/*` cover amount and date parsing, the uncertain rules, answer parsing, the toolless request and CLI
  arguments, the reader fallbacks (no model, text only, nonsense answer, transcript rule), field patches, case mapping and the
  reminder schedule with an injected clock.
- `tests/documents_test.rs` covers:
  - route auth: 401 with no token, 403 with a turn token or a wrong token, including a letter's attachment routes;
  - validation;
  - busy;
  - the unreadable path;
  - one end-to-end read of `tests/fixtures/letters/water-bill.pdf` plus the synthetic `letter-page2.png` with a stub model: one call,
    fields, uncertain markers, confirming, reminders, a tick with an injected clock, done/reopen and delete;
  - the log check.

No test calls a real model.

## Not covered yet

- A live read by the real API or CLI with images. Only stubs are tested.
- For photo-only letters, quotes are checked against the model's own transcript, which is weaker than checking against a PDF text
  layer. The UI should show the photo next to each field.
- Codex, Grok and Ollama can't read letters; only Claude can.
- Retention keeps the page rows and files of wiped letter cases. Delete removes them.
- No OCR without a model: a photo with no model set up is `unreadable`.
