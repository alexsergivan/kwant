//! The payment floor: tool calls that look like paying always get an approval card, whatever the
//! bot's approvals are (`ask`, `smart` or `off`), whatever "always allow" was given, and at every
//! tier. Oleks's decision D8 (2026-10-27): with approvals off, everything else runs without
//! asking, but payments still ask.
//!
//! [`reason`] is pure: it reads only the tool's name, its arguments and, for kwnt's own browser,
//! the address the browser itself reports. Nothing the bot writes can mark a call as "not a
//! payment". The lists err on the side of asking: an extra card costs a tap, a missed payment
//! costs money. What it can't catch is written down in docs/payments.md.

use super::ToolApprovalRequest;
use serde_json::Value;

/// Apps whose every write tool moves money (MCP server names, Composio toolkit prefixes).
const MONEY_APPS: &[&str] = &[
    "stripe", "paypal", "square", "squareup", "wise", "transferwise", "coinbase", "braintree", "adyen", "revolut", "mollie",
    "paddle", "lemonsqueezy", "razorpay", "gocardless", "klarna", "venmo", "cashapp", "chargebee", "recurly", "payoneer",
    "checkoutcom", "authorizenet", "plaid", "moonpay", "kraken", "binance",
];

/// Words that name paying on their own.
const PAY_WORDS: &[&str] = &[
    "pay", "payment", "payments", "payout", "payouts", "purchase", "purchases", "checkout", "buy", "donate", "donation",
    "charge", "charges", "remittance", "remit",
];

/// Verbs that move money when the noun is money.
const MOVE_VERBS: &[&str] = &["send", "transfer", "withdraw", "wire", "move", "deposit"];
const MONEY_NOUNS: &[&str] = &[
    "money", "funds", "fund", "cash", "crypto", "bitcoin", "btc", "eth", "ether", "usdc", "usdt", "sol", "usd", "eur", "gbp",
    "stablecoin",
];
// `token`, `coin` and `asset` aren't money nouns: `move_asset`, `send_token_email` or
// `deposit_coin` often move files, logins or game items. A crypto send with those words is caught
// by its arguments (an amount plus an address or asset, or a swap pair) instead.

/// Argument keys that hold an amount (also any `*_amount` and `amount_*`, like `amount_wei`), and keys that say where it goes or
/// what moves. A write tool moves value, whatever its name or server, when one of each also has
/// the right shape: a number for the amount, and an account address or a crypto asset for the
/// other (a crypto wallet's `send_transfer {token, to: "0x…", amount: "5"}`). A swap pair
/// (`token_in` and `token_out`) counts on its own, since wallets take any token there.
const AMOUNT_KEYS: &[&str] = &["amount", "amount_in", "amount_out", "value", "quantity", "sum"];
const DESTINATION_KEYS: &[&str] = &[
    "to", "recipient", "destination", "to_address", "address", "token", "token_in", "token_out", "asset", "currency", "coin",
];
const SWAP_PAIRS: &[(&str, &str)] =
    &[("token_in", "token_out"), ("from_token", "to_token"), ("sell_token", "buy_token"), ("input_mint", "output_mint")];
/// Token symbols and crypto nouns a destination or asset value can be. Symbols that are also
/// everyday words or names (ADA, TON, DOT, LINK, ONE, OP) are left out.
const CRYPTO_ASSETS: &[&str] = &[
    "btc", "wbtc", "eth", "weth", "steth", "usdc", "usdt", "dai", "usde", "pyusd", "eurc", "sol", "matic", "pol", "bnb",
    "avax", "arb", "xrp", "doge", "trx", "ltc", "bch", "xlm", "bitcoin", "ether", "ethereum", "solana", "crypto", "token",
    "coin", "stablecoin", "native",
];

/// Verbs that complete an order.
const ORDER_VERBS: &[&str] = &["place", "submit", "confirm", "complete", "finalize", "finalise"];
const ORDER_NOUNS: &[&str] = &["order", "orders", "purchase", "booking", "subscription"];

/// A tool whose first action word is one of these only reads.
const READ_VERBS: &[&str] = &[
    "get", "list", "retrieve", "search", "find", "read", "fetch", "describe", "view", "show", "count", "lookup", "query",
    "estimate", "preview", "check", "verify", "validate", "export", "download", "inspect", "status", "info", "balance",
    // Pure compute: works something out or moves the view, nothing else
    "calculate", "compute", "format", "translate", "scroll",
];

/// A name only reads when its first action word reads, and so does the first word after each
/// `and`/`then`: `get_quote` reads, `estimate_and_send` sends.
fn reads_only<S: AsRef<str>>(action: &[S]) -> bool {
    let mut parts = action.split(|w| matches!(w.as_ref(), "and" | "then")).filter(|p| !p.is_empty()).peekable();
    parts.peek().is_some() && parts.all(|p| READ_VERBS.contains(&p[0].as_ref()))
}

/// `check_out` and `checkOut` are `checkout`, not a `check`.
fn join_checkout(action: &mut Vec<String>) {
    while let Some(i) = action.windows(2).position(|w| w[0] == "check" && w[1] == "out") {
        action.splice(i..i + 2, ["checkout".to_string()]);
    }
}

/// Tools that type or fill text in a page (any browser MCP): a card number typed there pays.
const TYPE_WORDS: &[&str] = &["type", "fill", "input", "enter", "keyboard", "insert", "set", "write"];
/// Tools that click or submit in a page (any browser MCP): a pay-type label there pays.
const CLICK_WORDS: &[&str] = &["click", "tap", "press", "submit"];

/// Button and link labels that pay, as whole words or phrases.
const PAY_LABEL_WORDS: &[&str] = &["pay", "buy", "purchase", "checkout", "subscribe", "upgrade", "donate"];
const PAY_LABEL_PHRASES: &[&str] = &[
    "check out", "place order", "place your order", "complete order", "confirm order", "submit order", "send money",
    "send payment", "make payment", "complete payment", "confirm payment", "authorize payment", "start subscription",
    "start membership", "add funds", "top up", "book now", "reserve now", "rent now", "order now",
];

/// Hosts that take payments: a call to one of them that writes, or a click on one of their pages.
const PAYMENT_HOSTS: &[&str] = &[
    "stripe.com", "paypal.com", "squareup.com", "square.com", "wise.com", "transferwise.com", "coinbase.com",
    "braintreegateway.com", "adyen.com", "revolut.com", "mollie.com", "paddle.com", "lemonsqueezy.com", "razorpay.com",
    "gocardless.com", "klarna.com", "checkout.com", "authorize.net", "payoneer.com", "venmo.com", "cash.app",
    "pay.google.com", "pay.amazon.com", "chargebee.com", "recurly.com",
];

/// Address path segments and first host labels that mean a checkout page.
const CHECKOUT_SEGMENTS: &[&str] = &[
    "checkout", "checkouts", "payment", "payments", "pay", "billing", "purchase", "buy", "subscribe", "donate",
    "place-order", "placeorder", "order-review", "review-order",
];

/// Why this call is a payment, or `None`. `page_url` is the address kwnt's own browser shows now
/// (from the browser, never from the bot); only used for kwnt's `browser_act`.
pub fn reason(req: &ToolApprovalRequest, page_url: Option<&str>) -> Option<String> {
    if req.builtin.is_some() {
        return builtin_reason(&req.tool, &req.arguments);
    }
    if req.server == super::SELF_SERVER_NAME {
        return self_reason(&req.tool, &req.arguments, page_url);
    }
    // Local shell commands reach the approver as execute_shell
    if req.tool == "execute_shell" {
        return req.arguments["command"].as_str().and_then(shell_reason);
    }
    mcp_reason(&req.server, &req.tool, &req.arguments)
}

/// claude's own tools: only Bash can pay (WebFetch only reads; file tools touch files).
fn builtin_reason(tool: &str, input: &Value) -> Option<String> {
    match tool {
        "Bash" => input["command"].as_str().and_then(shell_reason),
        _ => None,
    }
}

/// kwnt's `browser_act` by its arguments and the page address alone (before the page is read).
pub fn browser_act_reason(args: &Value, page_url: Option<&str>) -> Option<String> {
    self_reason("browser_act", args, page_url)
}

/// kwnt's own tools: only `browser_act` can pay today. A new self tool with a money word in its
/// name fails `self_tools_have_no_money_words` until it is classified here.
fn self_reason(tool: &str, args: &Value, page_url: Option<&str>) -> Option<String> {
    if tool != "browser_act" {
        return None;
    }
    let text = args["text"].as_str().unwrap_or("");
    match args["action"].as_str().unwrap_or("") {
        "click_text" | "click" => {
            if let Some(why) = pay_label(text) {
                return Some(why);
            }
            page_url.and_then(checkout_page).map(|page| format!("a click on a checkout page ({page})"))
        }
        "type" => card_number(text).then(|| "typing what looks like a card number".to_string()),
        "key" => {
            let name = submit_key(args["key"].as_str())?;
            page_url.and_then(checkout_page).map(|page| format!("{name} on a checkout page ({page})"))
        }
        _ => None,
    }
}

/// What kwnt's browser sees on the page before a `browser_act`, read by
/// `BrowserController::payment_signals`: the element the action would hit, card fields and
/// payment frames. Read from the page itself, so a plain "Continue" on a card form still asks.
#[derive(Debug, Default, Clone, serde::Deserialize)]
#[serde(default)]
pub struct PageSignals {
    /// The element a click would hit, or the focused one for a key.
    pub target: Option<TargetSignals>,
    /// Visible card number / CVC fields on the page (open shadow roots and same-origin frames too).
    pub card_fields: u32,
    /// Frames on the page as (host, title): a payment provider's card frame is cross-origin.
    pub frames: Vec<FrameSignal>,
}

#[derive(Debug, Default, Clone, serde::Deserialize)]
#[serde(default)]
pub struct TargetSignals {
    pub label: String,
    /// `button`, `link`, `field` or `other`.
    pub kind: String,
    /// Inside a form (or the form a button submits).
    pub in_form: bool,
    /// The form's address when it names one (`action` or the button's `formaction`).
    pub form_action: String,
    /// The form has a card number or CVC field.
    pub form_card: bool,
    /// The element itself is a card field.
    pub card_field: bool,
}

#[derive(Debug, Default, Clone, serde::Deserialize)]
#[serde(default)]
pub struct FrameSignal {
    pub host: String,
    pub title: String,
}

/// Card-field frame hosts that aren't in [`PAYMENT_HOSTS`].
const PAY_FRAME_HOSTS: &[&str] = &[
    "braintreegateway.com", "braintree-api.com", "squarecdn.com", "paddle.com", "worldpay.com", "cybersource.com",
    "spreedly.com", "globalpay.com", "chargebee.com", "recurly.com", "hsforms.com",
];

/// First words of buttons that never pay, even on a card form.
const NEVER_PAYS: &[&str] = &[
    "cancel", "back", "close", "remove", "edit", "change", "help", "menu", "logout", "log", "sign", "show", "hide", "view",
    "apply", "use", "add",
];

/// Why a payment frame is one: its host, or a card-input title ("Secure card payment input frame").
fn payment_frame(frame: &FrameSignal) -> Option<String> {
    let host = frame.host.trim_end_matches('.').to_lowercase();
    let listed = !host.is_empty()
        && (payment_host(&host) || PAY_FRAME_HOSTS.iter().any(|h| host == *h || host.ends_with(&format!(".{h}"))));
    if listed {
        return Some(host);
    }
    let title = words(&frame.title.to_lowercase());
    let has = |w: &str| title.iter().any(|t| t == w);
    (has("card") && (has("payment") || has("number") || has("input")) || has("payment") && has("input"))
        .then(|| frame.title.chars().take(60).collect())
}

/// The name of a key that can submit or press what has focus: Enter submits a form, and Space
/// presses a focused button. A missing key means Enter. Every other key is `None`.
fn submit_key(key: Option<&str>) -> Option<&'static str> {
    match key.unwrap_or("Enter").to_ascii_lowercase().as_str() {
        "enter" | "return" | "numpadenter" => Some("Enter"),
        " " | "space" | "spacebar" => Some("Space"),
        _ => None,
    }
}

/// Why a `browser_act` is a payment from what the page shows (see [`PageSignals`]), or `None`.
/// Pure, so the rules are tested as tables; the bot's own words never mark a call as safe.
pub fn page_reason(action: &str, key: Option<&str>, page: &PageSignals) -> Option<String> {
    let frame = page.frames.iter().find_map(payment_frame);
    let card_on_page = page.card_fields > 0 || frame.is_some();
    let card_where = || match &frame {
        Some(f) if page.card_fields == 0 => format!("a payment frame ({f})"),
        _ => "card fields".to_string(),
    };
    let target = page.target.as_ref();
    match action {
        "click_text" | "click" => {
            let t = target?;
            // The label of the element it really hits, not the words the bot wrote
            if let Some(why) = pay_label(&t.label) {
                return Some(why);
            }
            let presses = t.kind == "button" || t.kind == "link";
            if presses && t.in_form {
                if let Some(page) = checkout_page(&t.form_action) {
                    return Some(format!("a button that sends a checkout form ({page})"));
                }
            }
            let first = words(&t.label.to_lowercase()).into_iter().next().unwrap_or_default();
            let never = NEVER_PAYS.contains(&first.as_str()) || has_phrase(&t.label.to_lowercase(), "continue shopping");
            if presses && !never && (t.form_card || card_on_page) {
                let label = t.label.chars().take(40).collect::<String>();
                return Some(format!("\"{label}\" on a page with {}", card_where()));
            }
            None
        }
        "key" => {
            let name = submit_key(key)?;
            if frame.is_some() && page.card_fields == 0 {
                return Some(format!("{name} on a page with {}", card_where()));
            }
            let t = target?;
            // Enter or Space on a focused button or link presses it, so the click rules apply
            if t.kind == "button" || t.kind == "link" {
                if let Some(why) = page_reason("click", None, page) {
                    return Some(why);
                }
            }
            if t.card_field || t.in_form && t.form_card {
                return Some(format!("{name} in a form with card fields"));
            }
            if t.in_form {
                if let Some(page) = checkout_page(&t.form_action) {
                    return Some(format!("{name} in a form that sends to a checkout ({page})"));
                }
            }
            None
        }
        _ => None,
    }
}

/// An MCP tool (incl. Composio): by its name and server, the Composio slugs it runs, and for
/// browser-type tools, the label it clicks or the card number it types.
fn mcp_reason(server: &str, tool: &str, args: &Value) -> Option<String> {
    let mut names = vec![tool.to_string()];
    collect_slugs(args, &mut names, 0);
    for name in &names {
        if let Some(why) = name_reason(server, name) {
            return Some(why);
        }
    }
    if let Some(why) = amount_shape_reason(tool, args, false) {
        return Some(why);
    }
    // Composio runs other tools by slug, each with its own arguments
    let mut calls = Vec::new();
    collect_slug_calls(args, &mut calls, 0);
    if let Some(why) = calls.into_iter().find_map(|(slug, args)| amount_shape_reason(slug, args, true)) {
        return Some(why);
    }
    let words = words(tool);
    let has = |list: &[&str]| words.iter().any(|w| list.contains(&w.as_str()));
    let mut texts = Vec::new();
    collect_strings(args, &mut texts, 0);
    if has(TYPE_WORDS) && texts.iter().any(|t| card_number(t)) {
        return Some(format!("{tool} types what looks like a card number"));
    }
    if has(CLICK_WORDS) {
        if let Some(why) = texts.iter().find_map(|t| pay_label(t)) {
            return Some(format!("{tool}: {why}"));
        }
    }
    // A shell an MCP server runs (a remote bash tool) is checked like Bash
    if has(&["bash", "shell", "exec", "execute", "command", "terminal", "run"]) {
        for key in ["command", "cmd", "script"] {
            if let Some(why) = args[key].as_str().and_then(shell_reason) {
                return Some(why);
            }
        }
    }
    None
}

/// A tool or slug name that pays: any write tool of a money-only app, or a money verb with a
/// money noun. A name whose first action word only reads never pays.
pub fn name_reason(server: &str, name: &str) -> Option<String> {
    // `mcp__stripe__create_charge` → server stripe, tool create_charge
    match name.strip_prefix("mcp__").and_then(|r| r.split_once("__")) {
        Some((server, tool)) => name_words_reason(&words(server), &words(tool), name),
        None => name_words_reason(&words(server), &words(name), name),
    }
}

fn name_words_reason(server_words: &[String], words: &[String], name: &str) -> Option<String> {
    let app = server_words
        .iter()
        .chain(words.first())
        .find(|w| MONEY_APPS.contains(&w.as_str()))
        .cloned();
    // The action words: the name without its app prefix
    let mut action: Vec<String> = words
        .iter()
        .skip_while(|w| MONEY_APPS.contains(&w.as_str()) || ["composio", "mcp"].contains(&w.as_str()))
        .cloned()
        .collect();
    join_checkout(&mut action);
    if reads_only(&action) {
        return None;
    }
    if let Some(app) = app {
        if !action.is_empty() {
            return Some(format!("{name} changes something in {app}, a payment app"));
        }
    }
    let has = |list: &[&str]| action.iter().any(|w| list.contains(&w.as_str()));
    if has(PAY_WORDS) || (has(MOVE_VERBS) && has(MONEY_NOUNS)) || (has(ORDER_VERBS) && has(ORDER_NOUNS)) {
        return Some(format!("{name} looks like a payment"));
    }
    None
}

/// A tool that doesn't only read and whose arguments carry a number under an amount key and an
/// account address or crypto asset under a destination or asset key, or a swap pair (top level,
/// or one object down): it moves value, whatever its name says. Key names alone aren't enough:
/// `convert {to: "mi", value: 5}` or `update_cell {address: "A1", value: 5}` don't pay. A
/// Composio `slug` starts with its app (`SOMEWALLET_GET_QUOTE`), so its first word isn't an
/// action word.
fn amount_shape_reason(tool: &str, args: &Value, slug: bool) -> Option<String> {
    let mut action: Vec<String> = words(tool)
        .into_iter()
        .skip(usize::from(slug))
        .skip_while(|w| MONEY_APPS.contains(&w.as_str()) || ["composio", "mcp"].contains(&w.as_str()))
        .collect();
    join_checkout(&mut action);
    if reads_only(&action) {
        return None;
    }
    let Value::Object(top) = args else { return None };
    let mut pairs: Vec<(String, &Value)> = Vec::new();
    for (k, v) in top {
        pairs.push((arg_key(k), v));
        if let Value::Object(inner) = v {
            pairs.extend(inner.iter().map(|(k, v)| (arg_key(k), v)));
        }
    }
    let has_key = |key: &str| pairs.iter().any(|(k, _)| k == key);
    let amount = pairs
        .iter()
        .any(|(k, v)| (AMOUNT_KEYS.contains(&k.as_str()) || k.ends_with("_amount") || k.starts_with("amount_")) && amount_value(v));
    let destination = pairs.iter().any(|(k, v)| DESTINATION_KEYS.contains(&k.as_str()) && destination_value(v))
        || SWAP_PAIRS.iter().any(|(a, b)| has_key(a) && has_key(b));
    (amount && destination).then(|| format!("{tool} moves an amount to another account or asset"))
}

/// A number, or a string that is one: `5`, `"0.001"`, `"1,000"`, `"1e18"`, `"0x2386f26fc10000"`,
/// `"5 USDC"`, `"$5"`, or `"max"`/`"all"` (send everything). An amount object counts by its
/// number: `{value: "5", currency: "USDC"}`.
fn amount_value(v: &Value) -> bool {
    match v {
        Value::Number(_) => true,
        Value::Object(map) => ["value", "amount", "units", "quantity", "raw", "decimal"]
            .iter()
            .filter_map(|k| map.get(*k))
            .any(|v| !v.is_object() && amount_value(v)),
        Value::String(s) => {
            let s = s.trim_start().trim_start_matches(CURRENCY_SYMBOLS);
            let first = s.split_whitespace().next().unwrap_or("").trim_end_matches(CURRENCY_SYMBOLS).to_ascii_lowercase();
            if matches!(first.as_str(), "max" | "all") {
                return true;
            }
            if let Some(hex) = first.strip_prefix("0x") {
                return !hex.is_empty() && hex.len() <= 64 && hex.chars().all(|c| c.is_ascii_hexdigit());
            }
            let plain: String = first.chars().filter(|c| !matches!(c, ',' | '_')).collect();
            plain.starts_with(|c: char| c.is_ascii_digit() || c == '.')
                && plain.parse::<f64>().is_ok_and(f64::is_finite)
        }
        _ => false,
    }
}

/// An account address (EVM `0x…`, ENS `*.eth`, bech32, base58, IBAN) or a crypto asset (a token
/// symbol, a crypto noun, or a mint or contract address, which the address shapes cover).
fn destination_value(v: &Value) -> bool {
    let Some(s) = v.as_str().map(str::trim) else { return false };
    let lower = s.to_ascii_lowercase();
    CRYPTO_ASSETS.contains(&lower.as_str())
        || evm_address(s)
        || ens_name(&lower)
        || bech32_address(s)
        || base58_address(s)
        || iban(s)
}

/// `0x` and 40 hex digits (EVM), or 64 (Sui, Aptos, Starknet).
fn evm_address(s: &str) -> bool {
    s.strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .is_some_and(|h| matches!(h.len(), 40 | 64) && h.chars().all(|c| c.is_ascii_hexdigit()))
}

fn ens_name(lower: &str) -> bool {
    lower.strip_suffix(".eth").is_some_and(|name| {
        !name.is_empty()
            && !name.starts_with('.')
            && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
    })
}

/// `bc1q…`, `cosmos1…`: a lowercase prefix, `1`, then 20 or more bech32 characters.
fn bech32_address(s: &str) -> bool {
    const CHARSET: &str = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
    let lower = s.to_ascii_lowercase();
    if s != lower && s != s.to_ascii_uppercase() {
        return false;
    }
    let Some((hrp, data)) = lower.rsplit_once('1') else { return false };
    (1..=20).contains(&hrp.len())
        && hrp.chars().all(|c| c.is_ascii_lowercase())
        && (20..=90).contains(&data.len())
        && data.chars().all(|c| CHARSET.contains(c))
}

/// Solana accounts and mints, Bitcoin legacy, Tron: 25–44 base58 characters, digits and letters.
fn base58_address(s: &str) -> bool {
    const ALPHABET: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    (25..=44).contains(&s.len())
        && s.chars().all(|c| ALPHABET.contains(c))
        && s.chars().any(|c| c.is_ascii_digit())
        && s.chars().any(|c| c.is_ascii_alphabetic())
}

/// An IBAN with a valid mod-97 check.
fn iban(s: &str) -> bool {
    let compact: String = s.chars().filter(|c| *c != ' ').collect::<String>().to_ascii_uppercase();
    let b = compact.as_bytes();
    if !(15..=34).contains(&b.len())
        || !b[..2].iter().all(u8::is_ascii_uppercase)
        || !b[2..4].iter().all(u8::is_ascii_digit)
        || !b.iter().all(u8::is_ascii_alphanumeric)
    {
        return false;
    }
    let mut rem: u32 = 0;
    for c in compact[4..].chars().chain(compact[..4].chars()) {
        let n = c.to_digit(36).unwrap_or(0);
        rem = if n >= 10 { (rem * 100 + n) % 97 } else { (rem * 10 + n) % 97 };
    }
    rem == 1
}

/// An argument key in one spelling: `amountIn`, `amount-in` and `AMOUNT_IN` → `amount_in`.
fn arg_key(key: &str) -> String {
    words(key).join("_")
}

/// The tools Composio runs by slug, each with its own `arguments`.
fn collect_slug_calls<'a>(v: &'a Value, out: &mut Vec<(&'a str, &'a Value)>, depth: usize) {
    if depth > 6 || out.len() > 50 {
        return;
    }
    match v {
        Value::Object(map) => {
            let slug = ["tool_slug", "slug", "action", "action_name", "tool", "tool_name"]
                .iter()
                .find_map(|k| map.get(*k).and_then(Value::as_str));
            if let (Some(slug), Some(args)) = (slug, map.get("arguments")) {
                out.push((slug, args));
            }
            map.values().for_each(|v| collect_slug_calls(v, out, depth + 1));
        }
        Value::Array(list) => list.iter().for_each(|v| collect_slug_calls(v, out, depth + 1)),
        _ => {}
    }
}

/// Composio runs other tools by slug (`COMPOSIO_MULTI_EXECUTE_TOOL {tools: [{tool_slug}]}`).
fn collect_slugs(v: &Value, out: &mut Vec<String>, depth: usize) {
    if depth > 6 {
        return;
    }
    match v {
        Value::Object(map) => {
            for (k, v) in map {
                if matches!(k.as_str(), "tool_slug" | "slug" | "action" | "action_name" | "tool" | "tool_name") {
                    if let Some(s) = v.as_str() {
                        out.push(s.to_string());
                    }
                }
                collect_slugs(v, out, depth + 1);
            }
        }
        Value::Array(list) => list.iter().for_each(|v| collect_slugs(v, out, depth + 1)),
        _ => {}
    }
}

fn collect_strings(v: &Value, out: &mut Vec<String>, depth: usize) {
    if depth > 6 || out.len() > 200 {
        return;
    }
    match v {
        Value::String(s) => out.push(s.clone()),
        Value::Number(n) => out.push(n.to_string()),
        Value::Object(map) => map.values().for_each(|v| collect_strings(v, out, depth + 1)),
        Value::Array(list) => list.iter().for_each(|v| collect_strings(v, out, depth + 1)),
        _ => {}
    }
}

/// Lowercase words of a name: `STRIPE_CreatePaymentIntent` → stripe, create, payment, intent.
fn words(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for c in name.chars() {
        if !c.is_alphanumeric() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
            continue;
        }
        if c.is_uppercase() && prev_lower && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        prev_lower = c.is_lowercase() || c.is_ascii_digit();
        cur.extend(c.to_lowercase());
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// A button or link label that pays: a pay word or phrase, or a price.
pub fn pay_label(label: &str) -> Option<String> {
    let lower = label.to_lowercase();
    if lower.chars().count() > 120 {
        // Long text is a description, not a label: only phrases count
        return PAY_LABEL_PHRASES.iter().find(|p| has_phrase(&lower, p)).map(|p| format!("a \"{p}\" button"));
    }
    let ws = words(&lower);
    if let Some(w) = ws.iter().find(|w| PAY_LABEL_WORDS.contains(&w.as_str())) {
        return Some(format!("a \"{w}\" button"));
    }
    if let Some(p) = PAY_LABEL_PHRASES.iter().find(|p| has_phrase(&lower, p)) {
        return Some(format!("a \"{p}\" button"));
    }
    has_price(&lower).then(|| "a button with a price on it".to_string())
}

fn has_phrase(text: &str, phrase: &str) -> bool {
    let text = format!(" {} ", words(text).join(" "));
    text.contains(&format!(" {phrase} "))
}

/// Currency signs a price or an amount can start or end with.
const CURRENCY_SYMBOLS: &[char] = &['$', '€', '£', '¥', '₹', '₩', '₽', '₺', '₴', '₦', '₱', '฿'];

/// `$12.99`, `€5`, `12,99 €`, `USD 10`, `10 eur`.
fn has_price(text: &str) -> bool {
    const CODES: &[&str] = &["usd", "eur", "gbp", "jpy", "chf", "cad", "aud", "inr", "pln", "uah", "sek", "nok", "dkk"];
    let chars: Vec<char> = text.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if CURRENCY_SYMBOLS.contains(c) {
            let after = chars.get(i + 1..).unwrap_or(&[]).iter().find(|c| !c.is_whitespace());
            let before = chars.get(..i).unwrap_or(&[]).iter().rev().find(|c| !c.is_whitespace());
            if after.is_some_and(|c| c.is_ascii_digit()) || before.is_some_and(|c| c.is_ascii_digit()) {
                return true;
            }
        }
    }
    let ws = words(text);
    ws.windows(2).any(|w| {
        (CODES.contains(&w[0].as_str()) && w[1].starts_with(|c: char| c.is_ascii_digit()))
            || (CODES.contains(&w[1].as_str()) && w[0].chars().all(|c| c.is_ascii_digit()))
    })
}

/// 13–19 digits (spaces or dashes between groups allowed) that pass the Luhn check.
pub fn card_number(text: &str) -> bool {
    let mut run = String::new();
    let check = |run: &str| (13..=19).contains(&run.len()) && luhn(run);
    let mut last_sep = false;
    for c in text.chars() {
        if c.is_ascii_digit() {
            run.push(c);
            last_sep = false;
        } else if (c == ' ' || c == '-') && !run.is_empty() && !last_sep {
            last_sep = true;
        } else {
            if check(&run) {
                return true;
            }
            run.clear();
            last_sep = false;
        }
    }
    check(&run)
}

fn luhn(digits: &str) -> bool {
    let mut sum = 0;
    for (i, c) in digits.chars().rev().enumerate() {
        let mut d = c.to_digit(10).unwrap_or(0);
        if i % 2 == 1 {
            d *= 2;
            if d > 9 {
                d -= 9;
            }
        }
        sum += d;
    }
    sum % 10 == 0 && digits.chars().any(|c| c != '0')
}

/// Whether `url` is a checkout page; returns its host and path for the card.
pub fn checkout_page(url: &str) -> Option<String> {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.rsplit('@').next().unwrap_or(host).split(':').next().unwrap_or(host).to_lowercase();
    let path = path.split(['?', '#']).next().unwrap_or("").to_lowercase();
    let shown = || format!("{host}/{path}").trim_end_matches('/').to_string();
    if payment_host(&host) {
        return Some(shown());
    }
    let first_label = host.split('.').next().unwrap_or("");
    if CHECKOUT_SEGMENTS.contains(&first_label) {
        return Some(shown());
    }
    path.split('/').any(|seg| CHECKOUT_SEGMENTS.contains(&seg) || seg.starts_with("checkout")).then(shown)
}

fn payment_host(host: &str) -> bool {
    let host = host.trim_end_matches('.');
    PAYMENT_HOSTS.iter().any(|h| host == *h || host.ends_with(&format!(".{h}")))
}

/// Programs that send crypto, with the subcommands that send.
const WALLETS: &[(&str, &[&str])] = &[
    ("cast", &["send", "publish"]),
    ("bitcoin-cli", &["sendtoaddress", "sendmany", "send", "sendall", "sendrawtransaction", "walletcreatefundedpsbt"]),
    ("solana", &["transfer", "pay"]),
    ("spl-token", &["transfer"]),
    ("electrum", &["payto", "paytomany", "broadcast"]),
    ("lncli", &["sendpayment", "payinvoice", "sendcoins", "sendmany", "sendtoroute"]),
    ("lightning-cli", &["pay", "withdraw", "keysend", "xpay"]),
    ("monero-wallet-cli", &["transfer", "sweep_all"]),
    ("cardano-cli", &["submit"]),
    ("near", &["send", "send-near"]),
    ("aptos", &["transfer"]),
    ("sui", &["pay", "pay-sui", "pay-all-sui", "transfer-sui"]),
];

/// `stripe` CLI words that change something.
const STRIPE_WRITES: &[&str] = &[
    "post", "delete", "create", "update", "capture", "confirm", "pay", "refund", "void", "finalize", "send", "trigger",
    "fixtures", "cancel", "approve",
];

/// A shell command that pays: a write to a payment API host, a `stripe` CLI write, or a crypto
/// wallet send. Commands that hide the host or method (variables, scripts) aren't caught.
pub fn shell_reason(command: &str) -> Option<String> {
    let tokens: Vec<String> = command
        .split(|c: char| c.is_whitespace())
        .map(|t| t.trim_matches(|c| c == '\'' || c == '"' || c == '`' || c == '(' || c == ')').to_string())
        .filter(|t| !t.is_empty())
        .collect();
    let lower = command.to_lowercase();
    if let Some(host) = payment_host_in(&lower) {
        if writes(&tokens, &lower) {
            return Some(format!("a request that writes to {host}, a payment service"));
        }
    }
    for segment in command.split([';', '&', '|', '\n', '(', ')', '`']) {
        let words: Vec<&str> = segment
            .split_whitespace()
            .map(|w| w.trim_matches(|c| c == '\'' || c == '"'))
            .skip_while(|w| (w.contains('=') && !w.starts_with('-')) || matches!(*w, "sudo" | "env" | "command" | "exec" | "nohup" | "time" | "npx" | "-E"))
            .collect();
        let Some((program, args)) = words.split_first() else { continue };
        let program = program.rsplit('/').next().unwrap_or(program).to_lowercase();
        if program == "stripe" && args.iter().any(|a| STRIPE_WRITES.contains(&a.to_lowercase().as_str())) {
            return Some("a stripe CLI command that changes something".into());
        }
        if let Some((_, subs)) = WALLETS.iter().find(|(p, _)| *p == program) {
            if args.iter().any(|a| subs.contains(&a.to_lowercase().as_str())) {
                return Some(format!("a {program} command that sends crypto"));
            }
        }
    }
    None
}

/// The first payment host named in a command.
fn payment_host_in(lower: &str) -> Option<&'static str> {
    lower
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-'))
        .filter(|t| t.contains('.'))
        .find_map(|t| {
            let t = t.trim_matches('.');
            PAYMENT_HOSTS.iter().find(|h| t == **h || t.ends_with(&format!(".{h}"))).copied()
        })
}

/// Whether a request in this command writes (anything but a plain GET).
fn writes(tokens: &[String], lower: &str) -> bool {
    let methods = ["post", "put", "patch", "delete"];
    for (i, t) in tokens.iter().enumerate() {
        let tl = t.to_lowercase();
        if (tl == "-x" || tl == "--request" || tl == "--method")
            && tokens.get(i + 1).is_some_and(|m| methods.contains(&m.to_lowercase().as_str()))
        {
            return true;
        }
        if let Some(m) = tl.strip_prefix("-x").or_else(|| tl.strip_prefix("--request=")).or_else(|| tl.strip_prefix("--method=")) {
            if methods.contains(&m) {
                return true;
            }
        }
        // curl's data and form flags make it a POST; wget's post flags; httpie's method word
        if (t.starts_with("-d") && !t.starts_with("--"))
            || tl.starts_with("--data")
            || tl.starts_with("--json")
            || t == "-F"
            || tl.starts_with("--form")
            || tl.starts_with("--post-")
            || tl.starts_with("--body")
            || ["POST", "PUT", "PATCH", "DELETE"].contains(&t.as_str())
        {
            return true;
        }
    }
    // Code that posts (python requests, fetch with a method, …)
    [".post(", ".put(", ".patch(", ".delete(", "method: 'post'", "method:'post'", "method: \"post\"", "method=\"post\"", "method='post'"]
        .iter()
        .any(|p| lower.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mcp(server: &str, tool: &str, args: Value) -> Option<String> {
        mcp_reason(server, tool, &args)
    }

    #[test]
    fn money_app_writes_and_money_names_pay() {
        assert!(mcp("stripe", "create_payment_intent", json!({})).is_some());
        assert!(mcp("stripe", "create_customer", json!({})).is_some(), "every write of a money-only app");
        assert!(mcp("stripe", "list_charges", json!({})).is_none(), "reads don't pay");
        assert!(mcp("stripe", "retrieve_balance", json!({})).is_none());
        assert!(mcp("shop", "purchase_item", json!({})).is_some());
        assert!(mcp("shop", "placeOrder", json!({})).is_some());
        assert!(mcp("bank", "send_money", json!({})).is_some());
        assert!(mcp("wallet", "transfer_funds", json!({})).is_some());
        assert!(mcp("billing", "pay_invoice", json!({})).is_some());
        assert!(mcp("gmail", "send_email", json!({})).is_none());
        assert!(mcp("drive", "transfer_ownership", json!({})).is_none());
        assert!(mcp("github", "create_issue", json!({})).is_none());
        assert!(mcp("shop", "get_order", json!({})).is_none());
    }

    #[test]
    fn composio_slugs_are_read_from_the_arguments() {
        let multi = |slug: &str| json!({"tools": [{"tool_slug": "GMAIL_FETCH_EMAILS", "arguments": {}}, {"tool_slug": slug, "arguments": {"amount": 100}}]});
        assert!(mcp("composio", "COMPOSIO_MULTI_EXECUTE_TOOL", multi("STRIPE_CREATE_PAYMENT_INTENT")).is_some());
        assert!(mcp("composio", "COMPOSIO_MULTI_EXECUTE_TOOL", multi("PAYPAL_CREATE_ORDER")).is_some());
        assert!(mcp("composio", "COMPOSIO_MULTI_EXECUTE_TOOL", multi("SQUARE_CREATE_PAYMENT")).is_some());
        assert!(mcp("composio", "COMPOSIO_MULTI_EXECUTE_TOOL", multi("GMAIL_SEND_EMAIL")).is_none());
        assert!(mcp("composio", "COMPOSIO_MULTI_EXECUTE_TOOL", multi("STRIPE_LIST_CHARGES")).is_none());
        assert!(mcp("composio", "STRIPE_CREATE_REFUND", json!({})).is_some());
        assert!(name_reason("", "mcp__stripe__create_charge").is_some());
    }

    /// A crypto wallet MCP (Incentiv's `src/wallet/tools.ts`): its sends name no money app and no
    /// money noun, so the amount and destination in the arguments decide.
    #[test]
    fn wallet_sends_pay_by_their_arguments() {
        let to = "0x52908400098527886E0F7030069857D2E4169EE7";
        let transfer = json!({"request_id": "6f1c1f5e-8a0b-4c1e-9f57-0d4f1c1b2a3d", "token": "USDC", "to": to, "amount": "5", "gas_token": "USDC", "wait_for_confirmation": false});
        let swap = json!({"request_id": "6f1c1f5e-8a0b-4c1e-9f57-0d4f1c1b2a3d", "token_in": "USDC", "token_out": "ETH", "amount_in": "5", "minimum_out": "0.001", "slippage_bps": 50});
        for server in ["incentiv", "wallet-7", "x"] {
            let why = mcp(server, "send_transfer", transfer.clone()).unwrap_or_else(|| panic!("{server} send_transfer"));
            assert_eq!(why, "send_transfer moves an amount to another account or asset");
            assert!(mcp(server, "send_swap", swap.clone()).is_some(), "{server} send_swap");
            assert!(mcp(server, "anything", swap.clone()).is_some(), "{server}: the arguments decide, not the name");
        }
        // Other spellings and one object down
        assert!(mcp("w", "execute", json!({"toAddress": to, "amountIn": 1})).is_some());
        assert!(mcp("w", "do", json!({"transfer": {"recipient": "vitalik.eth", "fee_amount": 1}})).is_some());
        assert!(mcp("w", "do", json!({"transfer": {"recipient": "bob", "fee_amount": 1}})).is_none(), "bob isn't an account address");
        assert!(mcp("w", "create_order", json!({"asset": "BTC", "quantity": 2})).is_some());
        // Through Composio, by the slug's own arguments
        let multi = json!({"tools": [{"tool_slug": "SOMEWALLET_SEND", "arguments": {"to": to, "value": "1"}}]});
        assert!(mcp("composio", "COMPOSIO_MULTI_EXECUTE_TOOL", multi).is_some());
        let quote = json!({"tools": [{"tool_slug": "SOMEWALLET_GET_QUOTE", "arguments": {"to": to, "value": "1"}}]});
        assert!(mcp("composio", "COMPOSIO_MULTI_EXECUTE_TOOL", quote).is_none(), "a read slug");

        // Reads, and amounts with nowhere to go, don't pay
        assert!(mcp("incentiv", "get_swap_quote", json!({"token_in": "USDC", "token_out": "ETH", "amount_in": "5"})).is_none());
        assert!(mcp("incentiv", "get_agent_wallet", json!({})).is_none());
        assert!(mcp("incentiv", "get_agent_operation", json!({"id": "6f1c1f5e-8a0b-4c1e-9f57-0d4f1c1b2a3d"})).is_none());
        assert!(mcp("incentiv", "create_agent_wallet", json!({})).is_none());
        assert!(mcp("github", "create_issue", json!({"title": "t", "body": "b"})).is_none());
        assert!(mcp("speaker", "set_volume", json!({"amount": 5})).is_none(), "an amount but no destination or asset");
        assert!(mcp("mail", "send_email", json!({"to": "a@b.c", "subject": "s"})).is_none(), "a destination but no amount");
        assert!(mcp("w", "do", json!({"a": {"b": {"to": to, "amount": 1}}})).is_none(), "two objects down is out of reach");
    }

    /// Rigel's F7 corpus: ordinary tools with amount-like and destination-like key names.
    #[test]
    fn key_names_alone_dont_pay() {
        for (tool, args) in [
            ("convert", json!({"from": "km", "to": "mi", "value": 5})),
            ("convert_units", json!({"to": "mi", "value": 5})),
            ("convert_currency", json!({"amount": 100, "from": "USD", "to": "EUR"})),
            ("calculate_tax", json!({"amount": 100, "currency": "USD"})),
            ("format_price", json!({"currency": "USD", "value": 100})),
            ("calculate_tip", json!({"amount": 100, "to": "split"})),
            ("translate", json!({"text": "hi", "to": "fr", "value": "x"})),
            ("browser_scroll", json!({"amount": 300, "to": "bottom"})),
            ("set_volume", json!({"address": "x", "value": 50})),
            ("update_cell", json!({"spreadsheet": "s", "address": "A1", "value": 5})),
            ("set", json!({"address": "h", "key": "k", "value": "v"})),
            ("put_secret", json!({"token": "abc", "value": "v"})),
            ("set_limit", json!({"token": "t", "value": 100})),
            ("kwnt_create_task", json!({"amount": 1, "title": "t", "to": "Ada"})),
            ("http_request", json!({"method": "POST", "url": "https://x", "body": {"amount": 5, "to": "x"}})),
        ] {
            assert_eq!(mcp("x", tool, args.clone()), None, "{tool} {args}");
        }
        // A wallet that names its swap `convert` still asks: its values are assets
        assert!(mcp("w", "convert", json!({"amount": "1", "from": "USDC", "to": "ETH"})).is_some());
        assert!(mcp("w", "convert", json!({"amount_in": "1", "token_in": "CENT", "token_out": "XYZ"})).is_some(), "a swap pair");
        // Compute verbs only read, even with wallet-shaped values
        assert!(mcp("w", "calculate_fee", json!({"amount": "1", "to": "0x52908400098527886E0F7030069857D2E4169EE7"})).is_none());
        // The amount must be a number
        assert!(mcp("w", "send", json!({"amount": "lots", "to": "0x52908400098527886E0F7030069857D2E4169EE7"})).is_none());
    }

    #[test]
    fn destination_and_amount_value_shapes() {
        for yes in [
            "0x52908400098527886E0F7030069857D2E4169EE7",
            "0x52908400098527886e0f7030069857d2e4169ee752908400098527886e0f7030",
            "vitalik.eth",
            "pay.team-wallet.eth",
            "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq",
            "cosmos1qypqxpq9qcrsszg2pvxq6rs0zqg3yyc5lzv7xu",
            "7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU",
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa",
            "DE89 3704 0044 0532 0130 00",
            "GB82WEST12345698765432",
            "USDC",
            "usdt",
            "native",
        ] {
            assert!(destination_value(&json!(yes)), "{yes}");
        }
        for no in [
            "A1", "mi", "fr", "EUR", "USD", "bob", "Ada", "bottom", "split", "abc", "Lin", "0x5290",
            "DE89370400440532013001", "hello.eth.com", ".eth", "sk_live_51Habc", "C1", "/a/b.txt",
            "https://x", "a@b.c", "climate.living_room",
        ] {
            assert!(!destination_value(&json!(no)), "{no}");
        }
        assert!(!destination_value(&json!(5)));
        for yes in [json!(5), json!(0.5), json!("5"), json!("0.001"), json!("1,000"), json!("1e18"), json!("0x2386f26fc10000"), json!("5 USDC"), json!("max"), json!("$5"), json!("5€"), json!({"value": "5", "currency": "USDC"})] {
            assert!(amount_value(&yes), "{yes}");
        }
        for no in [json!("v"), json!("x"), json!("lots"), json!("NaN"), json!("inf"), json!("-"), json!(""), json!(true), json!(null), json!({"a": 1}), json!("$"), json!("$x"), json!({"value": {"value": 5}})] {
            assert!(!amount_value(&no), "{no}");
        }
    }

    /// Rigel's F7b: `token`, `coin` and `asset` in a name don't pay on their own (files, logins,
    /// game items); with a wallet send's arguments they do. The older money nouns still pay by name.
    #[test]
    fn crypto_nouns_in_a_name_need_the_argument_shape() {
        let to = "0x52908400098527886E0F7030069857D2E4169EE7";
        for name in [
            "move_asset", "transfer_asset", "move_assets", "send_token", "send_token_email", "send_tokens", "deposit_coin",
            "send_asset_to_review", "move_token", "transfer_token", "withdraw_coins",
        ] {
            assert_eq!(mcp("x", name, json!({})), None, "{name} by its name alone");
            assert_eq!(mcp("x", name, json!({"asset_id": "a1", "to": "review", "path": "/a/b.png", "count": 2})), None, "{name}");
            assert_eq!(mcp("x", name, json!({"token": "abc123", "to": "a@b.c"})), None, "{name}: a login token by email");
            assert!(mcp("x", name, json!({"token": "USDC", "to": to, "amount": "5"})).is_some(), "{name}: a wallet send");
            assert!(mcp("x", name, json!({"token_in": "CENT", "token_out": "XYZ", "amount_in": "5"})).is_some(), "{name}: a swap");
        }
        for name in ["send_money", "transfer_funds", "withdraw_cash", "send_crypto", "send_usdc", "send_btc", "send_stablecoin"] {
            assert!(mcp("x", name, json!({})).is_some(), "{name}");
        }
        assert!(mcp("x", "list_tokens", json!({})).is_none());
        assert!(mcp("auth", "create_token", json!({})).is_none(), "no move verb");
        assert!(mcp("x", "get_asset", json!({})).is_none());
    }

    /// Rigel's F7b misses: an amount object, `amount_wei`, `"$5"`, a read verb before a send, and
    /// `check_out`.
    #[test]
    fn sends_rigel_found_missing() {
        let to = "0x52908400098527886E0F7030069857D2E4169EE7";
        let send = |amount: Value| json!({"to": to, "amount": amount});
        assert!(mcp("w", "send", send(json!({"value": "5", "currency": "USDC"}))).is_some(), "an amount object");
        assert!(mcp("w", "send", send(json!({"units": 5, "currency": "USD"}))).is_some());
        assert!(mcp("w", "do", json!({"payment": send(json!({"value": "5", "currency": "USD"}))})).is_some(), "one object down");
        assert!(mcp("w", "send", send(json!({"currency": "USD"}))).is_none(), "no number in it");
        assert!(mcp("w", "execute", json!({"to": to, "amount_wei": "1000000000000000000"})).is_some(), "amount_wei");
        assert!(mcp("w", "execute", json!({"to": to, "amountWei": "1000"})).is_some());
        assert!(mcp("w", "execute", json!({"to": "bob", "amount_wei": "1000"})).is_none(), "still needs an address");
        for amount in ["$5", "$ 5", "€1,000.50", "5€", "£0.99"] {
            assert!(mcp("w", "send", send(json!(amount))).is_some(), "{amount}");
        }
        assert!(mcp("w", "send", send(json!("$"))).is_none());
        let transfer = json!({"token": "USDC", "to": to, "amount": "5"});
        assert!(mcp("w", "estimate_and_send", transfer.clone()).is_some(), "a read verb, then a send");
        assert!(mcp("w", "check_then_transfer", transfer.clone()).is_some());
        assert!(mcp("w", "get_quote_and_execute", json!({"token_in": "USDC", "token_out": "ETH", "amount_in": "5"})).is_some());
        assert!(mcp("bank", "check_and_send_money", json!({})).is_some(), "by name");
        assert!(mcp("w", "estimate_fee", transfer.clone()).is_none(), "still only reads");
        assert!(mcp("w", "get_and_check_balance", transfer).is_none(), "reads all the way");
        assert!(mcp("stripe", "get_and_refund", json!({})).is_some());
        assert!(mcp("shop", "check_out", json!({})).is_some(), "check_out is checkout");
        assert!(mcp("shop", "checkOutCart", json!({})).is_some());
        assert!(mcp("shop", "check_order_status", json!({})).is_none());
        assert!(mcp("mail", "check_outbox", json!({})).is_none());
    }

    #[test]
    fn browser_mcp_tools_pay_by_label_or_card_number() {
        assert!(mcp("playwright", "browser_click", json!({"element": "Place order button", "ref": "e12"})).is_some());
        assert!(mcp("playwright", "browser_click", json!({"element": "Continue", "ref": "e3"})).is_none());
        assert!(mcp("playwright", "browser_type", json!({"ref": "e5", "text": "4242 4242 4242 4242"})).is_some());
        assert!(mcp("playwright", "browser_type", json!({"ref": "e5", "text": "hello"})).is_none());
        // A long id elsewhere isn't a card number to type
        assert!(mcp("discord", "get_message", json!({"id": "4242424242424242"})).is_none());
    }

    fn target(label: &str, kind: &str) -> TargetSignals {
        TargetSignals { label: label.into(), kind: kind.into(), ..Default::default() }
    }

    #[test]
    fn the_page_decides_what_a_click_really_hits() {
        let click = |page: &PageSignals| page_reason("click_text", None, page);
        // The bot wrote "Order" but the element it hits is "Place order"
        let hits = |t: TargetSignals| PageSignals { target: Some(t), ..Default::default() };
        assert!(click(&hits(target("Place order", "button"))).is_some());
        assert!(click(&hits(target("Pay $12.99", "button"))).is_some());
        assert!(click(&hits(target("Continue", "button"))).is_none(), "a plain button on a plain page");
        // A plain "Continue" on a page with card fields asks; Cancel, Back and the fields don't
        let card = |t: TargetSignals| PageSignals { target: Some(t), card_fields: 2, ..Default::default() };
        for label in ["Continue", "Next", "Finish", "Submit", "Confirm", "Review"] {
            assert!(click(&card(target(label, "button"))).is_some(), "{label}");
        }
        for label in ["Cancel", "Back to cart", "Continue shopping", "Apply coupon", "Edit address"] {
            assert!(click(&card(target(label, "button"))).is_none(), "{label}");
        }
        assert!(click(&card(target("Card number (field)", "field"))).is_none(), "clicking into a field");
        // A card frame from Stripe (cross-origin) counts, as does its title
        let frame = |host: &str, title: &str, t: TargetSignals| PageSignals {
            target: Some(t),
            frames: vec![FrameSignal { host: host.into(), title: title.into() }],
            ..Default::default()
        };
        let why = click(&frame("js.stripe.com", "", target("Continue", "button"))).unwrap();
        assert!(why.contains("js.stripe.com"), "{why}");
        assert!(click(&frame("pay.example-psp.io", "Secure card payment input frame", target("Next", "button"))).is_some());
        assert!(click(&frame("www.youtube.com", "YouTube video player", target("Next", "button"))).is_none());
        // A submit button whose form posts to a checkout address
        let mut t = target("Continue", "button");
        t.in_form = true;
        t.form_action = "https://shop.example.com/checkout/confirm".into();
        assert!(click(&hits(t.clone())).is_some());
        t.form_action = "https://shop.example.com/search".into();
        assert!(click(&hits(t)).is_none());
        // Nothing found under the label: no page rule (the label and address rules still run)
        assert!(click(&PageSignals { card_fields: 3, ..Default::default() }).is_none());
    }

    #[test]
    fn enter_asks_in_a_card_form_or_a_checkout_form() {
        let key = |k: &str, page: &PageSignals| page_reason("key", Some(k), page);
        let focus = |t: TargetSignals| PageSignals { target: Some(t), ..Default::default() };
        let mut f = target("Card number (field)", "field");
        f.card_field = true;
        assert!(key("Enter", &focus(f.clone())).is_some());
        assert!(key(" ", &focus(f.clone())).is_some(), "Space too");
        assert!(key("Tab", &focus(f)).is_none());
        // Space or Enter on a focused button presses it: the click rules decide
        let mut finish = target("Finish", "button");
        finish.in_form = true;
        finish.form_action = "https://shop.example.com/checkout/complete".into();
        for k in [" ", "Space", "Spacebar", "Enter"] {
            assert!(key(k, &focus(finish.clone())).is_some(), "{k:?} on a checkout button");
        }
        let place = target("Place order", "button");
        assert!(key(" ", &focus(place.clone())).expect("Space on Place order").contains("place order"));
        assert!(key("Tab", &focus(place)).is_none());
        assert!(key(" ", &focus(target("Help", "button"))).is_none());
        let mut name = target("Name on card (field)", "field");
        name.in_form = true;
        name.form_card = true;
        assert!(key("Enter", &focus(name)).is_some());
        let mut search = target("Search (field)", "field");
        search.in_form = true;
        search.form_action = "https://example.com/search".into();
        assert!(key("Enter", &focus(search.clone())).is_none());
        search.form_action = "https://example.com/billing/upgrade".into();
        assert!(key("Enter", &focus(search)).is_some());
        let stripe = PageSignals { frames: vec![FrameSignal { host: "js.stripe.com".into(), title: String::new() }], ..Default::default() };
        assert!(key("Enter", &stripe).is_some());
        assert!(page_reason("scroll", None, &stripe).is_none());
        assert!(page_reason("type", None, &stripe).is_none(), "typing alone doesn't pay; the submit does");
    }

    #[test]
    fn kwnt_browser_act() {
        let act = |a: Value, url: Option<&str>| self_reason("browser_act", &a, url);
        for label in ["Pay now", "Place order", "Buy", "Subscribe", "Upgrade to Pro", "Complete purchase", "Pay €12,99", "Donate", "Check out"] {
            assert!(act(json!({"action": "click_text", "text": label}), Some("https://example.com/")).is_some(), "{label}");
        }
        assert!(act(json!({"action": "click_text", "text": "Add to cart – $12.99"}), None).is_some(), "a price");
        for label in ["Continue", "Next", "Sign in", "Payload docs", "Display", "Repay"] {
            assert!(act(json!({"action": "click_text", "text": label}), Some("https://example.com/docs")).is_none(), "{label}");
        }
        // Any click or Enter on a checkout page
        assert!(act(json!({"action": "click_text", "text": "Continue"}), Some("https://shop.example.com/checkout/step-2")).is_some());
        assert!(act(json!({"action": "click_text", "text": "Next"}), Some("https://checkout.stripe.com/c/pay/cs_test")).is_some());
        assert!(act(json!({"action": "key", "key": "Enter"}), Some("https://www.paypal.com/checkoutnow?token=x")).is_some());
        for k in [" ", "Space", "space", "Spacebar"] {
            let why = act(json!({"action": "key", "key": k}), Some("https://shop.example.com/checkout-step-two.html"));
            assert!(why.expect(k).starts_with("Space on a checkout page"), "{k:?}");
        }
        assert!(act(json!({"action": "key", "key": " "}), Some("https://example.com/search")).is_none());
        assert!(act(json!({"action": "key", "key": "Tab"}), Some("https://shop.example.com/checkout")).is_none());
        assert!(act(json!({"action": "key", "key": "Enter"}), Some("https://example.com/search")).is_none());
        assert!(act(json!({"action": "scroll"}), Some("https://shop.example.com/checkout")).is_none());
        // Typing a card number anywhere
        assert!(act(json!({"action": "type", "text": "4111-1111-1111-1111"}), None).is_some());
        assert!(act(json!({"action": "type", "text": "4111 1111 1111 1112"}), None).is_none(), "fails Luhn");
        assert!(act(json!({"action": "type", "text": "+44 20 7946 0958"}), None).is_none());
        assert!(self_reason("browser_open", &json!({"url": "https://checkout.stripe.com/x"}), None).is_none());
        assert!(self_reason("memory_add", &json!({"text": "pay now"}), None).is_none());
    }

    #[test]
    fn claude_builtin_tools() {
        let checkout = json!({"url": "https://checkout.stripe.com/pay/cs_1", "prompt": "pay now"});
        assert!(builtin_reason("WebFetch", &checkout).is_none(), "WebFetch only reads");
        assert!(builtin_reason("Write", &json!({"file_path": "pay.sh", "content": "stripe charges create"})).is_none());
        assert!(builtin_reason("Bash", &json!({"command": "stripe charges create --amount 1"})).is_some());
        assert!(builtin_reason("Bash", &json!({"command": "stripe charges list"})).is_none());
    }

    #[test]
    fn shell_commands() {
        let pays = [
            "curl -X POST https://api.stripe.com/v1/charges -u sk_test_x: -d amount=100",
            "curl https://api.stripe.com/v1/payment_intents -d amount=2000 -d currency=usd",
            "curl --request PATCH 'https://api-m.paypal.com/v2/checkout/orders/1'",
            "curl -sS --json '{\"amount\":1}' https://api.wise.com/v1/transfers",
            "wget --post-data='a=1' https://connect.squareup.com/v2/payments",
            "http POST https://api.coinbase.com/v2/accounts/x/transactions to=abc amount=1",
            "python3 -c \"import requests; requests.post('https://api.stripe.com/v1/charges', data={})\"",
            "stripe charges create --amount 100 --currency usd",
            "stripe post /v1/refunds -d charge=ch_1",
            "cd x && sudo bitcoin-cli sendtoaddress bc1q 0.1",
            "cast send 0xabc --value 1ether",
            "solana transfer ABC 1",
        ];
        for c in pays {
            assert!(shell_reason(c).is_some(), "{c}");
        }
        let free = [
            "curl https://api.stripe.com/v1/balance -u sk_test_x:",
            "curl -s https://stripe.com/docs/api",
            "stripe charges list --limit 3",
            "stripe get /v1/charges",
            "stripe login",
            "curl -X POST https://api.github.com/repos/x/issues -d '{}'",
            "touch x",
            "bitcoin-cli getbalance",
            "cast call 0xabc 'balanceOf(address)'",
            "git push",
        ];
        for c in free {
            assert!(shell_reason(c).is_none(), "{c}");
        }
    }

    #[test]
    fn card_numbers_and_prices() {
        assert!(card_number("4242424242424242"));
        assert!(card_number("card: 5555 5555 5555 4444, exp 12/30"));
        assert!(card_number("378282246310005"), "amex, 15 digits");
        assert!(!card_number("4242424242424241"));
        assert!(!card_number("123456789012"), "too short");
        assert!(!card_number("0000000000000000"));
        assert!(has_price("pay $5"));
        assert!(has_price("12,99 €"));
        assert!(has_price("usd 10"));
        assert!(!has_price("version 2.1"));
    }

    #[test]
    fn checkout_pages() {
        assert!(checkout_page("https://shop.example.com/checkout").is_some());
        assert!(checkout_page("https://www.amazon.com/gp/buy/spc/handlers/display.html").is_some());
        assert!(checkout_page("https://pay.example.com/").is_some());
        assert!(checkout_page("https://buy.stripe.com/test_123").is_some());
        assert!(checkout_page("https://example.com/account/billing").is_some());
        assert!(checkout_page("https://example.com/blog/checkout-tips?x=1").is_some(), "a checkout* segment, accepted noise");
        assert!(checkout_page("https://example.com/docs/payments-overview").is_none());
        assert!(checkout_page("https://news.example.com/").is_none());
        assert!(checkout_page("https://example.com/?next=/checkout").is_none());
    }
}
