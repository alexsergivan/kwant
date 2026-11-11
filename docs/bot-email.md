# Bot email

Since 0.10.0 (phase 1), bots can use email with nothing extra to install. It is built into kwnt in Rust, so the Mac, Linux and Windows apps need no Node, no CLI and no outside mail service. The person connects one mailbox in **Settings › Bot email**, and then:

1. **A bot gets its own address for a sign-up.** `mail_address` returns a new address for one use. After the bot submits it on the site, `mail_wait_code` returns only the sign-in code (4–8 digits) or the sign-in link, never the email itself. The bot finishes the step on its own instead of raising a "Sign-in code needed" step.
2. **Asks go out by email (opt-in).** When a bot's question waits for the person, it can also go out by email. A reply from the person's own address answers it, like a reply in the app or on Telegram.
3. **A bot can email the person.** `mail_send` reaches only the send list, which holds the person's own addresses. In phase 1 a bot can't email anyone else.

Mail is untrusted data, the same as a web page. Email text never authorizes an action, and the tool descriptions tell the bot so.

## Credit

The safety rules, their test cases and the one-tag-per-use design are ported from [ClankerInbox](https://github.com/ClankerInbox/clankerinbox) by Danny Postma (MIT; reference copy at commit 8c8cf3be). The ported pieces are `src/safety.ts`, `test/shared-safety.test.ts`, `test/safety.test.ts` and `test/replies.test.ts`. kwnt doesn't bundle or run ClankerInbox: it is a Node CLI, and the kwnt apps don't ship Node. The licence is in [NOTICE](../NOTICE).

## Mailboxes (phase 1: IMAP + SMTP)

| Provider | Sign-in | Plus-addresses | Notes |
|---|---|---|---|
| Gmail | app password (needs 2-Step Verification) | yes | |
| Fastmail | app password | yes | |
| iCloud | app-specific password | **no** | use a mailbox just for the bots; see below |
| Proton Bridge | Bridge password, on 127.0.0.1 | yes | Bridge's self-signed certificate is accepted on loopback only |
| Other | password, host and port | yes | needs the receiving server's authserv-id for DMARC |

Outlook.com isn't offered: it takes only OAuth, not app passwords, for IMAP. The list comes from `mail::config::PROVIDERS` (`GET /api/connections/bot-email/providers`), so a new provider is a new entry, not a new screen.

**Recommended: a mailbox just for the bots** (`mailbox: "bots"`). Their mail and the person's never mix.

**The person's own mailbox** (`mailbox: "own"`) also works, for sign-up codes only:
- kwnt reads only mail sent to its own `+kwant-` addresses (IMAP `SEARCH` on `To` and `Delivered-To`).
- Email asks stay off. A reply from the same mailbox proves nothing, because anyone with that mailbox could have written it (spec E9).
- The send list must be another address than the bot mailbox (`same_mailbox`).

**Shared addresses (iCloud).** Without plus-addressing, every sign-up uses the mailbox's one address:
- A code is matched by the site's domain.
- Only one bot at a time may wait for a given site.

**Read-only by design.** kwnt opens INBOX with `EXAMINE` and reads mail with `BODY.PEEK[]`, exactly like the Zombie hunter's reader (`src/zombie/imap.rs`). It never sends `SELECT`, `STORE`, `COPY`, `MOVE`, `EXPUNGE` or `APPEND`, so the person's mail is never marked read, moved or deleted. What kwnt has already handled is kept in its own database (`bot_email_seen`, keyed by `INBOX:<uidvalidity>:<uid>`), not as IMAP flags. Mail larger than 512 KB is skipped. Only INBOX is read, not Spam.

**Sending** uses [lettre](https://crates.io/crates/lettre) with rustls only, no OpenSSL. Ports 465 (TLS) and 587 (STARTTLS) are supported. Plain connections are allowed only to this computer.

## The password

The mailbox password lives in the password vault, never in the database:
- owner `kwnt-bot-email`, username = the space id;
- the same rules as Loose ends logins ([saved-passwords.md](saved-passwords.md)): with approvals off somewhere and the built-in store, saving returns 409 `needs_bitwarden` until the person accepts the risk (`accept_risk: true`);
- a locked Bitwarden returns 409 `locked`.

The API never returns the password. A pasted password is wiped from memory when the request ends.

## Safety

Every check runs in the core (`src/mail/`), whatever the provider. A provider only moves raw mail; it never decides what is safe.

- **Default deny.** An incoming mail counts only if:
  - its `From` is exactly one address;
  - for answers, that address is on the send list;
  - it passed DMARC aligned, according to `Authentication-Results` from a trusted authserv-id (the provider's receiving server, or the ones given for Other).
  - No trusted result counts as a fail, and a pass from any other server is ignored.
- **Codes.** A code counts only when it came:
  - to the address mail_address gave out;
  - from the site the bot named (the sender's domain, or a subdomain of it);
  - after the address was made.
  - Only the code or link leaves the inbox; the subject, the body and any instructions in it never reach the bot.
  - An address takes one code, and expires after 24 hours.
- **Skipped:** bounces, auto-replies (`Auto-Submitted`, `X-Autoreply`, `Precedence: bulk/list/junk`), list mail (`List-Id`, `List-Unsubscribe`), a foreign `X-Loop`, and kwnt's own mail.
- **Never used:** an incoming `Reply-To`. Header values with control characters are refused.
- **Every mail kwnt sends** carries `Auto-Submitted: auto-generated` and `X-Loop: kwant`.
- **Hourly caps**, per space: 30 sends, and 20 received from one sender.
- **Replies** are read with quoted text and signatures removed.
- An answer picks a choice by its number or its exact text. A reply that picks nothing, or a number that isn't a choice, gets "I couldn't tell which one" and the ask stays open.

Refused mail is logged in `bot_email_log` with its reason (`not_on_list`, `dmarc`, …), never its text.

## Bot tools

| Tool | Read-only | Does |
|---|---|---|
| `mail_address` | yes | a new address for one sign-up; `site` limits it to that site's mail |
| `mail_wait_code` | yes | waits up to `timeout_secs` (10–900, default 300) and returns only `Sign-in code: …` or `Sign-in link: …` |
| `mail_send` | no | mails the person (send list only) |

The tools are offered in every turn. They refuse, and point the bot to `request_human_step` (kind `two_factor`), when:
- email isn't set up;
- the password is locked;
- the bot's email is off.

`mail_wait_code` counts as outside text in the truth ledger. While a bot waits, its chat shows one line, updated in place ("Waiting for the sign-in code from tiktok.com…", then "Got the sign-in code from tiktok.com"). That line never holds the code.

## Asks by email

Asks by email are off until both of these are true:
- the setup's reply check passed: the person answered the test mail with "ok", from the send list, and it passed DMARC;
- the person turned asks on.

They are never on for `mailbox: "own"`.

Then, every 20 seconds:
- An ask is mailed once, to each address on the send list, after `ask_delay_minutes` (0, 10 or 60).
  - It is sent from a new tagged address, with that address as `Reply-To` and Message-ID `q_<question>.<random>@kwant`.
  - Asks from before asks were turned on are never mailed.
- A reply is matched by the address it was sent to, or by `In-Reply-To`/`References`.
  - It must pass every check above.
  - It answers through the same path as the app (`answered_via: "email"`), and kwnt confirms by email.
  - A reply after the question was answered elsewhere gets "already answered".

## API (person only; a bot's token gets 403)

| Method | Path | |
|---|---|---|
| GET | `/api/connections/bot-email` | `state` (`off`/`saved`), config without the password, checks, health |
| GET | `/api/connections/bot-email/providers` | the provider list |
| POST | `/api/connections/bot-email/test` | starts the test; 202 (see below) |
| GET | `/api/connections/bot-email/test` | the test's progress |
| POST | `/api/connections/bot-email/reply-check` | mails the reply check again |
| POST | `/api/connections/bot-email/reply-check/skip` | skips it (asks stay off) |
| PATCH | `/api/connections/bot-email` | `asks`, `ask_delay_minutes`, `send_to` |
| DELETE | `/api/connections/bot-email` | forgets the mailbox, its password and its addresses |
| GET/PUT | `/api/connections/bot-email/bots/{kwant}` | `enabled` per bot, and its last address |
| GET | `/api/connections/bot-email/questions/{id}` | when an ask was emailed, and refused replies |

**Changing `send_to`** resets the reply check and turns asks off.

**The test** (`POST …/test`) runs in four steps:
1. Sign in to IMAP and SMTP.
2. Send one mail to the person.
3. Read it back by its Message-ID.
4. Save.

Nothing is saved until the test passes. A failed test reports the step and a code:
- `wrong_password`;
- `app_password_required`;
- `unreachable`;
- `send_failed`;
- `receive_timeout`;
- and so on.

## Code

| Path | What |
|---|---|
| `src/mail/mod.rs` | the `MailProvider` trait, `RawMail`, `OutgoingMail`, `MailError` |
| `src/mail/imap_smtp.rs` | the IMAP + SMTP provider |
| `src/mail/safety.rs`, `parse.rs` | the ported ClankerInbox rules and the mail parser |
| `src/mail/inbox.rs`, `flow.rs`, `outgoing.rs` | codes, answers, and the mail kwnt writes |
| `src/mail/config.rs` | presets and limits |
| `src/db/bot_email.rs` | config, per-bot on/off, tags, seen mail, log |
| `src/server/bot_email/` | the tools, the asks loop and the setup API |
| `tests/bot_email_e2e.rs` | end to end against an in-process fake IMAP + SMTP server |

## Later phases

- **Phase 2:** your own domain on Cloudflare, with the ported ClankerInbox Worker deployed through the Cloudflare API.
- **Phase 3:** ready-made kwant.si addresses (paid), through a `KwantCloud` provider.

The trait already allows what phase 3 needs: a provider that assigns addresses, signs in with a token and fetches over HTTPS. `src/mail/inbox.rs` tests a hosted provider against a forged From.
