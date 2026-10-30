# Payments always ask

A tool call that looks like paying always shows an approval card, whatever the bot's approvals
say: `ask`, `smart` or `off`. Oleks decided this on 2026-10-27 (Phase C D8): with approvals off,
sending, deleting, publishing and signing in run without asking, but payments still ask. The
same will hold at every Flight School tier.

- "Ask only when risky" saying a payment is safe doesn't skip the card.
- "Always this session" for the tool (e.g. all `Bash`) doesn't skip it.
- "Always this session" on a payment's own card allows that one call; nothing is remembered,
  and the next payment asks again.
- No setting turns this off. It costs the occasional card for something that wasn't a payment.
- A card the person allowed after its turn ended (late Allow) runs that exact call once, as for
  any card.

The check is `src/mcp/payment_floor.rs`, a pure function of the tool's name, its arguments and,
for kwnt's own browser, the address the browser itself reports. Nothing the bot writes, no
"Ask only when risky" verdict and no task field can mark a call as "not a payment". The card's
`details.payment_reason` says why it asked. It runs in `BrokerApprover` (claude's tools and
every MCP tool, through claude's permission prompt and the MCP gateway), in
`SelfToolsApprover` (kwnt's own `browser_act`), and for API drivers' `execute_shell`.

## What counts as a payment

**MCP and Composio tools** (by tool name, server name, and the `tool_slug`s inside
`COMPOSIO_MULTI_EXECUTE_TOOL`):

- Every tool of a money-only app that doesn't start with a read verb (`get`, `list`, `retrieve`,
  `search`, …): Stripe, PayPal, Square, Wise, Coinbase, Braintree, Adyen, Revolut, Mollie,
  Paddle, Lemon Squeezy, Razorpay, GoCardless, Klarna, Venmo, Cash App, Chargebee, Recurly,
  Payoneer, Checkout.com, Authorize.net, Plaid, MoonPay, Kraken, Binance.
- Any other tool whose name has a pay word (`pay`, `payment`, `payout`, `purchase`, `checkout`,
  `buy`, `charge`, `donate`, …), a money move (`send`/`transfer`/`withdraw` with `money`,
  `funds`, `crypto`, `btc`, `usd`, …), or an order step (`place`/`submit`/`confirm` with
  `order`, `purchase`, `booking`, `subscription`).
- A browser-type MCP tool (name with `click`, `tap`, `press`, `submit`) whose arguments name a
  pay-type label, or one that types (`type`, `fill`, `input`, …) a card number.
- A remote shell tool (`bash`, `shell`, `exec`, `command`…) whose `command` is a payment below.

**Shell commands** (claude's `Bash`, API drivers' `execute_shell`, MCP shell tools):

- A request that writes (`-X POST/PUT/PATCH/DELETE`, `-d`, `--data*`, `--json`, `-F`, `--form`,
  wget `--post-*`, httpie's method word, or code calling `.post(`) to a payment host:
  `stripe.com`, `paypal.com`, `squareup.com`, `wise.com`, `coinbase.com`, `adyen.com`, … (the
  list in `PAYMENT_HOSTS`). A GET doesn't count.
- `stripe` CLI commands that change something (`post`, `create`, `update`, `delete`,
  `capture`, `confirm`, `refund`, `trigger`, …).
- Crypto wallet sends: `cast send`, `bitcoin-cli sendtoaddress/sendmany/send…`, `solana
  transfer`, `spl-token transfer`, `electrum payto`, `lncli sendpayment/payinvoice/sendcoins`,
  `lightning-cli pay/withdraw`, `monero-wallet-cli transfer`, `cardano-cli … submit`, and others
  in `WALLETS`.

**kwnt's own browser** (`browser_act`):

- Clicking a pay-type label: `Pay`, `Buy`, `Purchase`, `Checkout`/`Check out`, `Subscribe`,
  `Upgrade`, `Donate`, `Place order`, `Confirm order`, `Complete payment`, `Book now`, `Top up`,
  … or any label with a price on it (`$12.99`, `12,99 €`, `USD 10`).
- Any click, or Enter, on a checkout page: a payment host (`checkout.stripe.com`, `paypal.com`,
  `pay.google.com`, …), a host starting `checkout.`/`pay.`/`payment.`/`billing.`, or a path
  segment like `/checkout`, `/payment`, `/pay`, `/billing`, `/buy`, `/subscribe`. The address
  comes from the browser, not the bot.
- Typing anything that looks like a card number: 13–19 digits (spaces or dashes allowed) that
  pass the Luhn check.

**Browser agents that take a whole task** (fastbrowse's `browse`, or any tool of a server
named like fastbrowse): kwnt can't see the clicks, so it judges the task before the call. Any
argument with `buy`, `purchase`, `checkout`/`check out`, `pay`/`payment`, `order`, `card`,
`subscribe`, `donate`, `upgrade`, `top up`, `add funds` or `send money`, a `card_number` field,
or a `start` (or fastbrowse's stopped-at) address that is a checkout page asks first. See
[fastbrowse](fastbrowse.md).

`WebFetch` never pays (it only reads). kwnt's own other tools don't pay, and
`self_tools_have_no_money_words` fails if one is added with a money word in its name until it is
classified here.

## What it doesn't catch

- A plain "Continue", "Next" or "Confirm" button on a checkout page whose address doesn't look
  like one (no checkout word in the host or path).
- A card the website already has saved, one-click buying, and Apple Pay or Google Pay sheets.
- Payment fields inside another site's frame on a normal-looking page (the address checked is
  the page's, not the frame's).
- A card number typed in pieces (one field for each group of 4 digits).
- Shell commands that hide the host or method: variables (`$URL`), scripts, config files, other
  languages' HTTP libraries without `.post(`, and SDKs (`stripe.Charge.create(...)` in Python).
- Payment services, MCP tools and wallets missing from the lists, and tools whose names don't
  say what they do.
- Codex's own shell, which kwnt never sees. Its sandbox has no network, so it can't reach a
  payment service.
- Someone else's browser tools the bot drives through an MCP server, unless the tool's label or
  typed text matches the rules above.

It also asks for some things that aren't payments, e.g. an "Upgrade" link in docs or any click on
a `/billing` page. That's accepted: an extra card costs a tap, a missed payment costs money.

## Tests

`cargo test --test payment_floor_test`, `cargo test --test fastbrowse_test` and the tables in `src/mcp/payment_floor.rs`
(`cargo test --lib payment_floor`).
