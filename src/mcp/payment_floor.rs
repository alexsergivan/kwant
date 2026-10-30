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
];

/// Verbs that complete an order.
const ORDER_VERBS: &[&str] = &["place", "submit", "confirm", "complete", "finalize", "finalise"];
const ORDER_NOUNS: &[&str] = &["order", "orders", "purchase", "booking", "subscription"];

/// A tool whose first action word is one of these only reads.
const READ_VERBS: &[&str] = &[
    "get", "list", "retrieve", "search", "find", "read", "fetch", "describe", "view", "show", "count", "lookup", "query",
    "estimate", "preview", "check", "verify", "validate", "export", "download", "inspect", "status", "info", "balance",
];

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
            let key = args["key"].as_str().unwrap_or("Enter");
            let submits = matches!(key.to_ascii_lowercase().as_str(), "enter" | "return" | "numpadenter");
            if submits {
                page_url.and_then(checkout_page).map(|page| format!("Enter on a checkout page ({page})"))
            } else {
                None
            }
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
            let submits = matches!(key.unwrap_or("Enter").to_ascii_lowercase().as_str(), "enter" | "return" | "numpadenter");
            if !submits {
                return None;
            }
            if frame.is_some() && page.card_fields == 0 {
                return Some(format!("Enter on a page with {}", card_where()));
            }
            let t = target?;
            if t.card_field || t.in_form && t.form_card {
                return Some("Enter in a form with card fields".into());
            }
            if t.in_form {
                if let Some(page) = checkout_page(&t.form_action) {
                    return Some(format!("Enter in a form that sends to a checkout ({page})"));
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
    let action: Vec<&str> = words
        .iter()
        .map(String::as_str)
        .skip_while(|w| MONEY_APPS.contains(w) || ["composio", "mcp"].contains(w))
        .collect();
    if action.first().is_some_and(|w| READ_VERBS.contains(w)) {
        return None;
    }
    if let Some(app) = app {
        if !action.is_empty() {
            return Some(format!("{name} changes something in {app}, a payment app"));
        }
    }
    let has = |list: &[&str]| action.iter().any(|w| list.contains(w));
    if has(PAY_WORDS) || (has(MOVE_VERBS) && has(MONEY_NOUNS)) || (has(ORDER_VERBS) && has(ORDER_NOUNS)) {
        return Some(format!("{name} looks like a payment"));
    }
    None
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

/// `$12.99`, `€5`, `12,99 €`, `USD 10`, `10 eur`.
fn has_price(text: &str) -> bool {
    const SYMBOLS: &[char] = &['$', '€', '£', '¥', '₹', '₩', '₽', '₺', '₴', '₦', '₱', '฿'];
    const CODES: &[&str] = &["usd", "eur", "gbp", "jpy", "chf", "cad", "aud", "inr", "pln", "uah", "sek", "nok", "dkk"];
    let chars: Vec<char> = text.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if SYMBOLS.contains(c) {
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
        assert!(key("Tab", &focus(f)).is_none());
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
