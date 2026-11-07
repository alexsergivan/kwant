# kwant 0.7.7 API: ready-made teams (T) and team discovery (D)

Lin, 2026-11-07. Branch `lin-077-teams` (from main 266032e). Spec: `next-077/spec.md` §T, §D. Design: `next-077/design/design.md`. Tests: `next-077/qa/test-plan.md` §1, §1b.
Where this file and design.md disagree on a field name, this file is what the server does; the differences are listed in §8.

Conventions (same as the rest of `/api`):

- Every route takes `?space=<id>` (default `spc_default`), the usual `Scope`.
- Errors are `{"error": "<plain sentence>", "code": "<snake_case>"}`, plus `field` and `suggestion` where §5 says so.
- **Person only** means a bot's turn token or a connected agent's token gets **403** `{"error": "only the person can do this"}` before the handler runs (the `person_only` list in `src/server/auth.rs`).

| Route | Who |
|---|---|
| `GET /api/team-templates` | anyone with the API token, agents too (Q-T9) |
| `GET /api/team-templates/:id/preview` | person only |
| `POST /api/teams/from-template` | person only |
| `POST /api/teams/suggest` | person only |
| `GET /api/teams/:id/template` | person only |
| `POST /api/bot-actions/:action_id/undo` | person only (unchanged route; Undo for a template team goes through it) |

---

## 1. `GET /api/team-templates?space=`

The catalogue. Static data from `src/teams/templates.rs` (which embeds Mira's generated `templates.json` byte for byte), plus one per-space flag. Cache it on the client for the offline preview.

```json
{
  "templates": [
    {
      "id": "inbox",
      "label": "Inbox",
      "icon": "inbox",
      "handle": "inbox",
      "card_line": "Sorts your mail and drafts replies for you to send.",
      "mission": "Keeps one inbox under control: …",
      "used_in_space": false,
      "bots": [
        {
          "slot": "InboxManager",
          "preset": "support",
          "name": "InboxManager",
          "title": "Inbox Manager",
          "line": "Reads new mail, sorts it, and tells you what needs you today.",
          "avatar_sprite": "tech_lead",
          "lead": true,
          "good_for": ["Sorting new email", "…"],
          "not_for": ["Sending or forwarding email on its own", "…"]
        }
      ],
      "needs": [
        {"id": "email", "label": "Connect email", "why": "So InboxManager can read new mail. Read-only (IMAP).",
         "required": true, "opens": "loose_ends_imap", "done_when": "An IMAP source in Loose ends checks OK"}
      ],
      "never": ["Sends, forwards or replies to mail. It drafts; you send.", "…", "…"],
      "skills": [
        {"owner": "InboxManager", "name": "Morning inbox digest", "description": "…", "body": "Goal\n…"}
      ],
      "routine": {"name": "Inbox digest", "when": "Weekdays at 8:00", "bot": "InboxManager", "enabled": false},
      "first_prompts": ["What needs me today?", "Here's how I usually sound in email: ", "Draft a polite no to the latest invitation"]
    }
  ]
}
```

- Order: inbox, sales, coding. `routine` is `null` for Coding.
- `slot` is the bot's key in `names` (§3). It is the template's own name for that bot and never changes. **Not the preset**: Coding has two `backend` bots (TechLead and Backend), so a preset can't be a key.
- `skills[].body` is included so the preview's **Read** works offline.
- `used_in_space`: a team in this space was made from this template and not undone (T10's "already used here").
- Persona, SOUL and the pinned instructions are not served; the server writes them at create.
- No `taken` list: the client uses the bots and teams it already has (design's preferred option). `…/preview` gives the server's answer.

## 2. `GET /api/team-templates/:id/preview?space=`

The server's free names for this space, for the sheet's clash state (T9). Read-only; makes nothing.

```json
{
  "template_id": "inbox",
  "handle": "inbox2",
  "handle_wanted": "inbox",
  "names": {"InboxManager": "InboxManager", "Drafter": "Drafter2", "Researcher": "Researcher"},
  "clashes": [
    {"field": "handle", "wanted": "inbox", "suggestion": "inbox2"},
    {"field": "names.Drafter", "wanted": "Drafter", "suggestion": "Drafter2"}
  ]
}
```

- **Taken names:** any bot in the space with that name ignoring case, plus a pending hire proposal of that name (the `create_clash` rule, Q-T11).
- **Taken handles:** a live team in the space with that name ignoring case, or a live channel called `team-<handle>`. An archived team chat is renamed `archived-team-…`, so it doesn't hold the handle.
- **Next free:** `Drafter` → `Drafter2` → `Drafter3` …; `inbox` → `inbox2` …. No space before the number (design A3).
- 404 `{"code": "unknown_template"}` for an unknown id.

## 3. `POST /api/teams/from-template?space=`

Creates the whole team in **one transaction**: bots, their Good for / Not for, the team and its chat, pinned instructions, starter skills (on, owned by one bot each, A7), and the routine (off). Then one logged action (`team_template`) for Undo. Nothing on disk. Never turns Dev mode on (Q-T12), never adds tools or changes approvals (T6).

Header `Idempotency-Key: <client uuid>` (optional, recommended): the same key in the same space within 24 h returns the first result (201 with the same body) and creates nothing new. A double click is safe.

### 3a. From a template (T)

```json
{
  "template_id": "inbox",
  "handle": "inbox",
  "names": {"Drafter": "ReplyDrafter"}
}
```

- `handle` optional, default the template's. Lower-case slug: `[a-z0-9-]{1,40}`, starting with a letter or digit. The client slugs as you type ("Inbox Team" → `inbox-team`); the server doesn't, it refuses (400 `bad_handle`).
- `names` optional; keys are `slot`s from §1, missing slots keep the template name. Same rules as any bot name: no whitespace, no `@`, 1–40 characters.
- **Never renames silently** (Q-T7): a taken name or handle is a 409 with the server's suggestion (§5), and nothing is made.

### 3b. From a suggestion (D)

```json
{
  "suggestion_id": "sug_7f3c…",
  "edits": {
    "handle": "shop",
    "rows": {
      "lead":     {"name": "TechLead"},
      "frontend": {"use_existing": "<kwant_id>"},
      "qa":       {"on": false},
      "writer":   {"on": true},
      "sre":      {"preset": "backend", "name": "Platform"}
    }
  }
}
```

- `rows` keys are the `key`s from the suggestion (§4), for `bots[]` and `also_possible[]` alike. A row not mentioned keeps its suggested state: `bots[]` on, `also_possible[]` off.
- Per row, all optional: `on` (bool), `name`, `preset` (one of the D presets, §4), `use_existing` (a live bot id in this space, or `null` for **New bot**).
- `edits` and each row are **strict**: any other field (`persona`, `soul`, `instructions`, `title`, …) is a **400 `unknown_field`** (Q-D6a). The server builds every persona from the preset plus its own facts.
- The lead row can't be turned off (400 `lead_required`, A14). At most 6 bots on (400 `too_many_bots`).
- `use_existing` (D6, A16): that bot joins the team unchanged; its persona is not touched. The stack line goes into the team instructions instead.
- A suggestion made for an existing team (`team_id` in §4) **adds** the ticked bots to that team (A18). `handle` is then refused (400 `unknown_field`), and Undo removes only the bots it added.
- A suggestion is used up by the first successful create (Q-D6c). Using it again, or after 1 hour, is **410 `suggestion_expired`** (A17: the client reads the project again and re-applies its edits).
- `template_id` and `suggestion_id` together: 400 `bad_request`. For the Coding template with a repo, the client calls §4 and posts the suggestion (T1).

### Response 201

```json
{
  "team": { "...": "the usual Team row (GET /api/teams/:id)" },
  "channel_id": "…",
  "action_id": "…",
  "template_id": "inbox",
  "bots": [{"slot": "InboxManager", "id": "…", "name": "InboxManager", "preset": "support", "lead": true, "existing": false}],
  "skills": [{"id": "…", "name": "Morning inbox digest", "owner_id": "…"}],
  "routine": {"id": "…", "name": "Inbox digest", "enabled": false},
  "undo_until": "2026-11-08T10:00:00Z"
}
```

- For D, `template_id` is `"coding"` when it came from the Coding template, else `null`, and `slot` is the row `key`.
- `channel_id` is the team chat to open. The Needs card is the client's overlay (Q-T5), not a message, so the chat starts empty.
- The client gets the usual `team`, `kwant` and `bot_action` events too.

## 4. `POST /api/teams/suggest?space=`

Reads the project. No model call. Nothing is made, and nothing stays on disk.

```json
{"workspace_path": "/Users/me/projects/shop"}
{"repo_url": "https://git.example.com/acme/shop"}
{"team_id": "…"}
```

Exactly one of the three (400 `bad_request` otherwise, and on unknown fields). `team_id` reads that team's folder or repo and only proposes roles the team doesn't have yet.

```json
{
  "suggestion_id": "sug_7f3c…",
  "expires_at": "2026-11-07T11:00:00Z",
  "team_id": null,
  "handle": "shop",
  "facts": [
    {"name": "React 18", "detail": "react in web/package.json"},
    {"name": "TypeScript", "detail": "412 .tsx files"}
  ],
  "stack_line": "This team's code: Rust (axum), React 18 + TypeScript (Vite), Docker",
  "bots": [
    {
      "key": "lead",
      "preset": "backend",
      "lead": true,
      "name": "TechLead",
      "title": "Tech Lead · Backend",
      "avatar_sprite": "architect",
      "reason_parts": [{"text": "Found axum in "}, {"path": "api/Cargo.toml"}],
      "owns": ["api/"],
      "existing_kwant_id": null,
      "existing_name": null
    }
  ],
  "also_possible": [
    {"key": "writer", "preset": "writer", "name": "Writer", "title": "Tech Writer", "avatar_sprite": "writer",
     "reason": "Only 3 pages in docs/ (10 make a Tech Writer).", "existing_kwant_id": null, "existing_name": null}
  ],
  "read": {"files": 1873, "manifests": 6, "ms": 412},
  "partial": null,
  "fallback": false
}
```

- `key`: `lead` for the lead, else the preset id. Unique within one suggestion.
- D presets: `frontend`, `backend`, `qa`, `sre`, `data`, `writer`, `security`, `designer`.
- `reason_parts`: render `text` as text and `path` as code. Every `path` is a folder or manifest path `[A-Za-z0-9._/-]{1,40}`; nothing else from the repo appears in a reason.
- `owns`: at most 3 top folders that carry a signal, each `[A-Za-z0-9._/-]{1,40}`; a longer name is dropped, not cut (Q-D10).
- `existing_kwant_id` / `existing_name`: the oldest live bot in the space with that preset's title or name (D6, Q-D4b). The row then defaults to **Add {existing_name} (already in this space)**; it is still a new bot unless the create sends `use_existing`.
- `partial`: `null`, `"paths"` (over 20,000 paths), `"bytes"` (over 2 MB of manifests, or a file over 256 KB was skipped) or `"time"` (10 s). The suggestion is still made from what was read.
- `fallback: true`: nothing recognised; `bots` is the Coding roster, `facts` is `[]` and `stack_line` is `null` (D7 "Couldn't tell what this project uses").
- Lead (A13, Q-D2): TechLead with the `backend` preset ("Tech Lead · Backend"), or the `frontend` preset ("Tech Lead · Frontend") for a frontend-only project. 2 to 6 bots; a lone signal is padded with the lead.
- No progress events in this version: the request finishes within the 10 s limit, so the sheet shows "Reading file names and manifests" without numbers.

Errors:

| Status | `code` | When |
|---|---|---|
| 400 | `bad_request` / `unknown_field` | not exactly one source; extra fields |
| 400 | `bad_path` | `workspace_path` isn't an absolute path to a readable folder |
| 400 | `bad_repo_url` | not an `https://` (or `ssh`) URL Dev mode accepts; local addresses refused |
| 404 | `team_not_found` | `team_id` isn't a team in this space |
| 409 | `team_has_no_project` | that team has no folder or repo |
| 422 | `cant_read` | a private repo it can't read; body adds `"forge": "github" \| "gitlab" \| null` for **Connect GitHub** |
| 502 | `read_failed` | the mirror or walk failed for another reason |

## 5. Errors from `from-template`

| Status | `code` | Extra | When |
|---|---|---|---|
| 400 | `bad_request` | | neither or both of `template_id`, `suggestion_id` |
| 400 | `unknown_field` | `field` | any field not listed above (top level, `edits`, a row) |
| 400 | `bad_name` | `field: "names.<slot>"` or `"edits.rows.<key>.name"` | whitespace, `@`, empty or over 40 |
| 400 | `bad_handle` | `field: "handle"` | not a lower-case slug |
| 400 | `bad_preset` / `bad_row` / `lead_required` / `too_many_bots` | `field` | D edits |
| 400 | `bad_existing` | `field` | `use_existing` isn't a live bot in this space |
| 404 | `unknown_template` | | |
| 409 | `name_taken` | `field: "names.Drafter"`, `suggestion: "Drafter2"` | a bot name is taken (or two slots got the same name) |
| 409 | `handle_taken` | `field: "handle"`, `suggestion: "inbox2"` | |
| 410 | `suggestion_expired` | | unknown, used, or over 1 hour old |
| 423 | `space_archived` | | |
| 500 | `create_failed` | | the transaction failed; **nothing was made** |

Only the first clash is reported; call `…/preview` for all of them.

## 6. Undo (T4)

`POST /api/bot-actions/:action_id/undo` with the `action_id` from §3. The response is the action, `status: "undone"`, as for every bot action.

Before it, the sheet's confirm reads `GET /api/teams/:id/template`:

```json
{
  "template_id": "inbox",
  "action_id": "…",
  "created_at": "2026-11-07T10:00:00Z",
  "undo_until": "2026-11-08T10:00:00Z",
  "can_undo": true,
  "blocked": [],
  "removes": {
    "bots": ["InboxManager", "Drafter", "Researcher"],
    "skills": 2,
    "routines": 1,
    "handle": "inbox"
  },
  "chat_messages": 4,
  "also_in": [{"bot": "Drafter", "team": "support"}],
  "edited_skills": []
}
```

404 when the team wasn't made by a template (or was made before 0.7.7).

What Undo does, in one transaction:

- Removes the bots it made (renamed or not), their skills and routines, the team and the routine. Bots reused with `use_existing` are left alone and only leave the team.
- The team chat: deleted when it has no messages, otherwise **archived** as a team delete does today (`archived-team-<handle>`). Its own path; the hire Undo is unchanged (Q-T3).
- A bot that was added to another team since leaves that team (`also_in`).
- Edited starter skills are removed too; `edited_skills` names them so the confirm can say so (Q-T2).

Undo refuses (409, nothing changes) when:

| `code` | When |
|---|---|
| `undo_expired` | over 24 hours since create (Q-T1) |
| `bot_leads_elsewhere` | a bot it made now leads another team or channel; `blocked` lists them |
| (existing) | the action is already undone, or a newer change to the same team must be undone first |

`blocked` and `can_undo` in `GET …/template` say the same before the person clicks.

## 7. Test seams (debug and test builds only; ignored in release)

| Seam | What |
|---|---|
| `fail_after: Option<Step>` | `cfg(test)` argument of the create transaction: fails after `Bot(n)`, `Team`, `Skills`, `Routine` or `Action`, then rolls back (Q-T8) |
| `KWNT_TEST_NOW_OFFSET_SECS` | shifts "now" for the 1 h suggestion expiry and the 24 h Undo window (Q-X1) |
| `KWNT_DISCOVER_TIMEOUT_MS` | shortens the 10 s read budget (Q-D9a) |
| loopback forge | `http://127.0.0.1` repos only in test builds, the Dev mode seam (Q-D12) |

## 8. Where this differs from design.md or the test plan

| Topic | design / test plan | This API | Why |
|---|---|---|---|
| Rename map key (A10) | `names: {preset: name}` | `names: {slot: name}`, slot = template bot name | Coding has two `backend` bots |
| Handle taken (Q-T11) | archived teams' handles count | only live teams and live `team-<handle>` chats | an archived chat is renamed, so the handle is free again |
| Undo route | "Undo team" | the existing bot-action Undo, plus `GET /api/teams/:id/template` for the confirm | one Undo path, already person-only |
| Templates source | templates-copy.md | `templates.json` embedded byte for byte from `design/templates.js` | no hand copying |
