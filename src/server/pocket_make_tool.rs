//! pa12-V1 (autonomy/pocket-apps/pa12/V1/brief.md): "Make your own", builder path (a). The bot's
//! `pocket_make` tool, offered beside `desk_create` only while pa12 L1's switch lets `made`
//! through (`pocket_make` on, and `pocket_apps` `on` or naming `made`).
//!
//! V1 has one action, `remix`, and it **routes; it never makes**:
//! - With no arguments it answers with the order to try things in ([`RULES`]) and the kits that
//!   are on now.
//! - `kit` quiz, santa or desk (a kit whose chat tool exists and is on): the answer names that
//!   kit's own create tool (`pocket_create`, `santa_create`, `desk_create`) and carries that kit's
//!   own rules, so the bot makes the pocket through the kit's tool. The kit's card, checks and
//!   smart_results (which find the pocket by the kit tool's name) stay as they are.
//! - A kit that is off, or that has no chat tool on this build (queue, gather, wall here): a plain
//!   "nothing was made" naming the kits that are on.
//! - `kit` none (the ask needs its own lists, fields or steps that no kit has): the answer tells
//!   the bot to say it can make that as a custom app, and to make nothing. No `made` pocket, no
//!   spec and no code: that is V2/V3.
//!
//! Nothing from the arguments is echoed back (`why` is checked for size and kept out of the
//! answer), and no call writes anything.

use super::continuity::SelfScope;
use super::AppState;
use crate::pocket::{desk, gather, kits, queue, santa, wall};
use serde_json::{json, Value};

type ToolResult = (String, bool);

pub const TOOL: &str = "pocket_make";
/// The one action V1 has.
pub const REMIX: &str = "remix";
/// The `kit` that means no kit fits.
pub const NONE: &str = "none";
/// What a `kit: none` answer tells the bot to say, word for word.
pub const CUSTOM_APP: &str = "I can make that as a custom app.";
/// The longest `why` (one line) a call may carry.
const WHY_MAX_CHARS: usize = 300;

/// The kits path (a) can name: (kit id, the name people know it by, what it is for).
const KITS: [(&str, &str, &str); 6] = [
    (crate::kits::quiz::KIT_ID, "quiz", "a pub quiz, class quiz or party quiz that guests play on their phones"),
    (santa::ID, "Secret Santa", "a Secret Santa or gift exchange: kwant draws the names, and each person sees only their own"),
    (desk::ID, "front desk", "a guest guide that answers guests' questions, or a booking desk where people ask for a time"),
    (queue::ID, "queue", "a walk-in queue: people take a ticket on their phone and wait to be called"),
    (gather::ID, "sign-up, RSVP or vote", "people say if they're coming, sign up for a slot, or vote"),
    (wall::ID, "photo wall", "guests send party photos, and the host picks which ones show"),
];

pub fn handles(name: &str) -> bool {
    name == TOOL
}

/// Whether "Make your own" is on: pa12 L1's switch lets `made` through (`pocket_make` on, and
/// `pocket_apps` `on` or naming `made`).
pub async fn enabled(state: &AppState) -> bool {
    crate::pocket::enabled_for(&state.db, kits::MADE_ID).await
}

pub fn tool() -> Value {
    json!({
        "name": TOOL,
        "description": "Use this first when the person asks you to make an app, game or page for a group (\"make me an app for our book \
            swap\", \"a game for the party\"). It finds which of kwant's ready-made pocket apps fits and tells you which tool makes it: \
            a quiz, a Secret Santa or a front desk, with the person's own words. It makes nothing itself. Call it first with no \
            arguments to get the rules, then with action \"remix\" and the kit. If no kit fits, use kit \"none\": then tell the \
            person you can make it as a custom app, and make nothing.",
        "inputSchema": {"type": "object", "properties": {
            "action": {"type": "string", "enum": [REMIX], "description": "remix: find the kit that makes this"},
            "kit": {"type": "string", "enum": ["quiz", "santa", "desk", "queue", "gather", "wall", NONE],
                "description": "The kit that fits, or none when the app needs its own lists, fields or steps that no kit has"},
            "why": {"type": "string", "description": "One line: why this kit fits (or why none does)"}
        }},
    })
}

/// What a call with no arguments answers (then the kits that are on now, see [`rules`]).
pub const RULES: &str = "Make your own: the rules.

Try these in order:
(a) Remix a kit first. Most asks are a kit kwant already has, with the person's own words: \"a Secret Santa for a book swap\" is a \
Secret Santa, and \"a pub quiz about poetry\" is a quiz. Pick the kit and call pocket_make with {\"action\": \"remix\", \"kit\": \
\"<kit>\", \"why\": \"<one line>\"}. Its answer names the tool that makes it and gives that kit's own rules. Make it with that tool, so \
the person gets the kit's normal card and checks.
(b) Only if no kit fits, because the app needs its own lists, fields or steps that no kit has, call pocket_make with kit \"none\". Then \
tell the person \"I can make that as a custom app.\" and make nothing: no pocket and no code.

Never write an app's code or screens yourself, and never make two pockets for one ask.

Kits:";

/// [`RULES`], each kit on one line, and which ones are on now.
fn rules(on: &[&str]) -> String {
    let mut out = String::from(RULES);
    for (id, name, what) in KITS {
        out.push_str(&format!("\n- {id} ({name}): {what}."));
    }
    out.push_str(&format!("\n\n{}", on_now(on)));
    out
}

/// The tool that makes `kit` from chat on this build, and that kit's own rules. `None`: this build
/// has no chat tool for it (queue, gather and wall on this base).
fn kit_tool(kit: &str) -> Option<(&'static str, String)> {
    match kit {
        crate::kits::quiz::KIT_ID => Some(("pocket_create", super::quizmaster_tools::template())),
        santa::ID => Some((super::santa_tool::TOOL, super::santa_tool::RULES.to_string())),
        desk::ID => Some((super::desk_tool::TOOL, super::desk_tool::RULES.to_string())),
        _ => None,
    }
}

/// The kits with a chat tool that are on now, in [`KITS`] order.
async fn kits_on(state: &AppState) -> Vec<&'static str> {
    let mut on = Vec::new();
    for (id, ..) in KITS {
        if kit_tool(id).is_some() && crate::pocket::enabled_for(&state.db, id).await {
            on.push(id);
        }
    }
    on
}

/// "On this kwant now: quiz (pocket_create), santa (santa_create)." or that none is.
fn on_now(on: &[&str]) -> String {
    if on.is_empty() {
        return "On this kwant now: no kit you can make from chat.".into();
    }
    let list: Vec<String> = on.iter().filter_map(|id| kit_tool(id).map(|(t, _)| format!("{id} ({t})"))).collect();
    format!("On this kwant now: {}.", list.join(", "))
}

fn name_of(kit: &str) -> &'static str {
    KITS.iter().find(|(id, ..)| *id == kit).map(|(_, n, _)| *n).unwrap_or("kit")
}

fn fail(msg: impl Into<String>) -> ToolResult {
    (msg.into(), true)
}

/// A string argument: `Ok(None)` when absent, null or blank.
fn str_arg<'a>(args: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.trim())),
        Some(_) => Err(format!("`{key}` must be a string.")),
    }
}

pub async fn call(state: &AppState, _scope: &SelfScope, args: &Value) -> ToolResult {
    if !enabled(state).await {
        return fail("Making your own pocket apps is off on this kwant. Nothing was made.");
    }
    if !args.is_object() && !args.is_null() {
        return fail("The arguments must be a JSON object.");
    }
    let (action, kit, why) = match (str_arg(args, "action"), str_arg(args, "kit"), str_arg(args, "why")) {
        (Ok(a), Ok(k), Ok(w)) => (a, k, w),
        (Err(e), ..) | (_, Err(e), _) | (.., Err(e)) => return fail(format!("{e} Nothing was made.")),
    };
    if why.is_some_and(|w| w.chars().count() > WHY_MAX_CHARS) {
        return fail(format!("`why` is one line, at most {WHY_MAX_CHARS} characters. Nothing was made."));
    }
    let on = kits_on(state).await;
    let kit = match (action, kit) {
        (None, None) => return (rules(&on), false),
        (Some(a), _) if !a.eq_ignore_ascii_case(REMIX) => return fail("pocket_make has one action, remix. Nothing was made."),
        (Some(_), Some(k)) => k.to_ascii_lowercase(),
        _ => return fail("A remix needs both action \"remix\" and kit. Call pocket_make with no arguments for the rules. Nothing was made."),
    };
    if kit == NONE {
        return (
            format!(
                "No kit fits, so nothing was made. Tell the person, in one line: \"{CUSTOM_APP}\" Make nothing now: no pocket, no spec and \
                 no code, and don't make it with another kit's tool."
            ),
            false,
        );
    }
    if !KITS.iter().any(|(id, ..)| *id == kit) {
        let names: Vec<&str> = KITS.iter().map(|(id, ..)| *id).chain([NONE]).collect();
        return fail(format!("kit must be one of {}. Nothing was made.", names.join(", ")));
    }
    let name = name_of(&kit);
    let Some((next, kit_rules)) = kit_tool(&kit) else {
        return fail(format!(
            "kwant can't make a {name} from chat yet, so nothing was made. Tell the person plainly, and offer a kit that's on if one fits. {}",
            on_now(&on)
        ));
    };
    if !on.contains(&kit.as_str()) {
        return fail(format!("The {name} kit isn't on in this kwant, so nothing was made. Tell the person plainly. {}", on_now(&on)));
    }
    (
        format!(
            "Remix: make it as a {name} with {next}. Nothing was made yet. Call {next} next, with the person's own words in the kit's own \
             fields, and follow its rules below. Its card and checks come with it.\n\n{kit_rules}"
        ),
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kit_is_a_real_kit_and_only_three_have_a_chat_tool() {
        for (id, ..) in KITS {
            assert!(kits::kit(id).is_some(), "{id} is a kit");
            assert_ne!(id, kits::MADE_ID);
        }
        let with_tool: Vec<&str> = KITS.iter().map(|(id, ..)| *id).filter(|id| kit_tool(id).is_some()).collect();
        assert_eq!(with_tool, ["quiz", "santa", "desk"]);
    }

    #[test]
    fn the_rules_name_the_order_and_every_kit() {
        let r = rules(&["quiz", "santa"]);
        assert!(r.find("(a) Remix a kit first").unwrap() < r.find("(b) Only if no kit fits").unwrap(), "{r}");
        assert!(r.contains(CUSTOM_APP), "{r}");
        for (id, ..) in KITS {
            assert!(r.contains(&format!("\n- {id} (")), "{id} in {r}");
        }
        assert!(r.ends_with("On this kwant now: quiz (pocket_create), santa (santa_create)."), "{r}");
        assert!(rules(&[]).ends_with("On this kwant now: no kit you can make from chat."));
    }
}
