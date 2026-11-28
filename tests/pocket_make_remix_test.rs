//! pa12-V1 (autonomy/pocket-apps/pa12/V1/brief.md): "Make your own", builder path (a). The bot's
//! `pocket_make` tool is offered only while pa12 L1's switch lets `made` through, and its one
//! action, `remix`, routes to the kit that already exists: it names the kit's own create tool and
//! carries that kit's own rules, and makes nothing itself. The pocket is then made by the kit's
//! tool, so the reply gets the kit's normal card. `kit: none` answers that no ready-made app fits
//! and kwant can't make a custom app from chat yet, and makes nothing else. A kit that's off, or has no chat tool on this build, gets a plain
//! "nothing was made". The fake model here is the test calling the bot's own tools endpoint.

mod quizmaster_support;

use chrono::{Datelike, NaiveDate, Utc};
use kwnt::db::models::{Kwant, Message, MessageExtra, NewMessage};
use kwnt::mcp::SELF_SERVER_NAME;
use kwnt::pocket::MAKE_SETTING;
use kwnt::server::continuity::TurnOrigin;
use kwnt::server::{desk_tool, pocket_make_tool, quizmaster_tools, santa_tool};
use quizmaster_support::{card, setup, start_pages, tool, tool_names, turn, turn_in, write_friday_quiz, Env};
use serde_json::{json, Value};

async fn switches(e: &Env, apps: &str, make: Option<&str>) {
    e.state.db.set_setting(quizmaster_tools::SETTING, apps).await.unwrap();
    // `None` leaves pocket_make as it is (never set, on a fresh kwant)
    if let Some(v) = make {
        e.state.db.set_setting(MAKE_SETTING, v).await.unwrap();
    }
}

/// A turn in the bot's own direct chat.
async fn direct_turn(e: &Env, id: &str) -> String {
    let origin = TurnOrigin { direct_kwant_id: Some(e.bot.id.clone()), ..Default::default() };
    turn_in(e, &e.bot, id, origin).await
}

fn remix(kit: &str) -> Value {
    json!({"action": "remix", "kit": kit, "why": "the person's ask fits it"})
}

/// A turn's record of one of its own tool calls, as the self server writes it.
fn call(tool: &str, args: Value, status: &str, result: &str) -> Value {
    json!({"id": format!("call-{tool}"), "name": format!("{SELF_SERVER_NAME}__{tool}"), "server": SELF_SERVER_NAME, "tool": tool,
           "arguments": args, "status": status, "result": result})
}

/// `bot`'s final reply in its direct chat, with the turn's tool calls.
async fn reply_as(e: &Env, bot: &Kwant, calls: Vec<Value>) -> Message {
    let msg = NewMessage {
        channel_id: None,
        direct_kwant_id: Some(bot.id.clone()),
        sender_role: "kwant".into(),
        sender_id: bot.name.clone(),
        text: "Done.".into(),
        reasoning_content: None,
        tool_calls: Some(Value::Array(calls)),
        tokens_used: 0,
    };
    e.state.db.insert_message_with(&msg, &MessageExtra::default()).await.unwrap()
}

fn pocket(m: &Message) -> Option<Value> {
    let b = m.results.as_ref()?.get("blocks")?.get(0)?.clone();
    (b["type"] == json!("pocket")).then_some(b)
}

/// The kits of every pocket that exists.
async fn pocket_kits(e: &Env) -> Vec<String> {
    e.state.db.pocket_list().await.unwrap().into_iter().map(|p| p.kit).collect()
}

/// The next Dec 18 at least a week away (the Santa kit refuses past dates).
fn gift_day() -> NaiveDate {
    let today = Utc::now().date_naive();
    let this_year = NaiveDate::from_ymd_opt(today.year(), 12, 18).unwrap();
    if this_year - today >= chrono::Duration::days(7) {
        this_year
    } else {
        NaiveDate::from_ymd_opt(today.year() + 1, 12, 18).unwrap()
    }
}

/// "A Secret Santa for our book swap", as the kit's own spec.
fn book_swap_santa() -> Value {
    let d = gift_day().format("%Y-%m-%d").to_string();
    json!({
        "title": "Book Swap Secret Santa",
        "gift_on": d,
        "reveal": true,
        "reveal_at": format!("{d}T19:00:00+00:00"),
        "people": ["Anna", "Ben", "Chloe", "Dev"],
        "budget": "one book",
    })
}

#[tokio::test]
async fn pocket_make_is_offered_only_while_made_is_on() {
    let e = setup(false).await;
    // pocket_apps=on alone (pocket_make never set first, then off or a near miss): not offered
    let mut off_names = None;
    for (i, make) in [None, Some("off"), Some("ON"), Some("yes")].into_iter().enumerate() {
        switches(&e, "on", make).await;
        let token = turn(&e, &e.bot, &format!("t-off-{i}")).await;
        let names = tool_names(&e, &token).await;
        assert!(!names.iter().any(|n| n == pocket_make_tool::TOOL), "{make:?}: {names:?}");
        assert!(names.iter().any(|n| n == santa_tool::TOOL), "the kits stay on: {names:?}");
        let (text, err) = tool(&e, &token, pocket_make_tool::TOOL, remix("santa")).await;
        assert!(err && text.contains("off"), "{make:?}: {text}");
        off_names.get_or_insert(names);
    }
    // pocket_make=on but pocket_apps names no `made`: still not offered
    switches(&e, "santa", Some("on")).await;
    let token = turn(&e, &e.bot, "t-santa-only").await;
    assert!(!tool_names(&e, &token).await.iter().any(|n| n == pocket_make_tool::TOOL));
    // both on: offered, and it is the only tool that changed
    for apps in ["on", "santa,made"] {
        switches(&e, apps, Some("on")).await;
        let token = turn(&e, &e.bot, &format!("t-on-{apps}")).await;
        let names = tool_names(&e, &token).await;
        assert!(names.iter().any(|n| n == pocket_make_tool::TOOL), "{apps}: {names:?}");
        if apps == "on" {
            let mut without: Vec<String> = names.into_iter().filter(|n| n != pocket_make_tool::TOOL).collect();
            let mut before = off_names.clone().unwrap();
            without.sort();
            before.sort();
            assert_eq!(without, before, "switching made on adds pocket_make and nothing else");
        }
    }
    // and off again: gone
    switches(&e, "on", Some("off")).await;
    let token = turn(&e, &e.bot, "t-off-again").await;
    assert!(!tool_names(&e, &token).await.iter().any(|n| n == pocket_make_tool::TOOL));
    assert!(pocket_kits(&e).await.is_empty(), "nothing made");
}

#[tokio::test]
async fn with_no_arguments_it_answers_the_order_and_the_kits_that_are_on() {
    let e = setup(false).await;
    switches(&e, "on", Some("on")).await;
    let token = turn(&e, &e.bot, "t-rules").await;
    let (text, err) = tool(&e, &token, pocket_make_tool::TOOL, json!({})).await;
    assert!(!err, "{text}");
    assert!(text.starts_with(pocket_make_tool::RULES), "{text}");
    assert!(text.find("(a) Remix a kit first").unwrap() < text.find("(b) Only if no kit fits").unwrap(), "{text}");
    assert!(text.contains(pocket_make_tool::CUSTOM_APP), "{text}");
    assert!(text.ends_with("On this kwant now: quiz (pocket_create), santa (santa_create), desk (desk_create)."), "{text}");
    // The description holds the same order
    let v = quizmaster_support::rpc(&e, &token, "tools/list", json!({})).await;
    let t = v["result"]["tools"].as_array().unwrap().iter().find(|t| t["name"] == pocket_make_tool::TOOL).unwrap().clone();
    let d = t["description"].as_str().unwrap();
    assert!(d.contains("It makes nothing itself") && d.contains("custom app"), "{d}");
    // Scoped to apps guests open on their phones, and it never promises a custom app (Mira F4)
    assert!(d.contains("open on their phones"), "{d}");
    assert!(d.contains("can't make a custom app from chat yet") && !d.contains("can make it as a custom app"), "{d}");
    // The code rule is about pocket apps only (Mira F2)
    assert!(text.contains("Don't write the code or screens for a pocket app yourself"), "{text}");
    assert!(!text.contains("I can make that as a custom app"), "{text}");
    assert!(pocket_kits(&e).await.is_empty());
}

#[tokio::test]
async fn remix_santa_routes_to_santa_create_and_the_reply_gets_the_santa_card() {
    let e = setup(false).await;
    switches(&e, "on", Some("on")).await;
    e.state.db.set_setting("timezone", "Europe/London").await.unwrap();
    let token = direct_turn(&e, "t-swap").await;
    let args = json!({"action": "remix", "kit": "santa", "why": "a Secret Santa for a book swap"});
    let (routed, err) = tool(&e, &token, pocket_make_tool::TOOL, args.clone()).await;
    assert!(!err, "{routed}");
    assert!(routed.starts_with("Remix: make it as a Secret Santa with santa_create."), "{routed}");
    assert!(routed.contains("not \"remix\" or \"kit\""), "the person hears their own words: {routed}");
    assert!(routed.ends_with(santa_tool::RULES), "the kit's own rules, whole: {routed}");
    assert!(pocket_kits(&e).await.is_empty(), "the remix itself makes nothing");
    // The fake model follows the answer: santa_create with the kit's own spec
    let (made, err) = tool(&e, &token, santa_tool::TOOL, json!({"spec": book_swap_santa()})).await;
    assert!(!err, "{made}");
    let code = made.strip_prefix(santa_tool::CARD_TEXT_HEAD).and_then(|r| r.split(']').next()).unwrap().to_string();
    assert_eq!(pocket_kits(&e).await, ["santa"], "one santa pocket, no made pocket");
    let row = e.state.db.pocket_get(&code).await.unwrap().unwrap();
    assert_eq!(row.owner_kwant_id.as_deref(), Some(e.bot.id.as_str()));
    // The reply that carries both calls gets the Santa kit's normal card
    let m = reply_as(
        &e,
        &e.bot,
        vec![call(pocket_make_tool::TOOL, args, "ok", &routed), call(santa_tool::TOOL, json!({"spec": book_swap_santa()}), "ok", &made)],
    )
    .await;
    let b = pocket(&m).unwrap_or_else(|| panic!("a pocket block: {:?}", m.results));
    assert_eq!((b["kit"].clone(), b["code"].clone(), b["name"].clone()), (json!("santa"), json!(code), json!("Book Swap Secret Santa")), "{b}");
    assert_eq!(b["santa"]["phase"], "collecting", "{b}");
    // pocket_make alone on a reply carries no card
    let alone = reply_as(&e, &e.bot, vec![call(pocket_make_tool::TOOL, remix("santa"), "ok", &routed)]).await;
    assert_eq!(pocket(&alone), None);
}

#[tokio::test]
async fn remix_quiz_routes_to_pocket_create_with_its_template_and_the_card_is_ready() {
    let e = setup(false).await;
    switches(&e, "on", Some("on")).await;
    let (pages_url, _pages) = start_pages().await;
    let token = direct_turn(&e, "t-poetry").await;
    let args = json!({"action": "remix", "kit": "quiz", "why": "a pub quiz about poetry"});
    let (routed, err) = tool(&e, &token, pocket_make_tool::TOOL, args.clone()).await;
    assert!(!err, "{routed}");
    assert!(routed.starts_with("Remix: make it as a quiz with pocket_create."), "{routed}");
    let (template, _) = tool(&e, &token, "pocket_create", json!({})).await;
    assert!(routed.ends_with(&template), "the quiz's own template and rules, whole: {routed}");
    // The fake model writes the quiz from the template the remix gave it
    let spec = write_friday_quiz(&routed, &pages_url);
    let (made, err) = tool(&e, &token, "pocket_create", json!({"spec": spec.clone()})).await;
    assert!(!err, "{made}");
    assert_eq!(card(&made)["state"], json!("ready"), "{made}");
    assert!(pocket_kits(&e).await.is_empty(), "a quiz is a draft until it opens, and no made pocket");
    let m = reply_as(&e, &e.bot, vec![call(pocket_make_tool::TOOL, args, "ok", &routed), call("pocket_create", json!({"spec": spec}), "ok", &made)])
        .await;
    let b = pocket(&m).unwrap_or_else(|| panic!("a pocket block: {:?}", m.results));
    assert_eq!((b["kit"].clone(), b["state"].clone()), (json!("quiz"), json!("ready")), "{b}");
    assert_eq!(b["name"], json!("Friday Pub Quiz"));
}

#[tokio::test]
async fn remix_desk_routes_to_desk_create_with_its_rules() {
    let e = setup(false).await;
    switches(&e, "desk,made", Some("on")).await;
    let token = turn(&e, &e.bot, "t-desk").await;
    let (routed, err) = tool(&e, &token, pocket_make_tool::TOOL, remix("desk")).await;
    assert!(!err, "{routed}");
    assert!(routed.starts_with("Remix: make it as a front desk with desk_create."), "{routed}");
    assert!(routed.ends_with(desk_tool::RULES), "{routed}");
    assert!(pocket_kits(&e).await.is_empty());
}

#[tokio::test]
async fn kit_none_says_custom_app_and_makes_nothing() {
    let e = setup(false).await;
    switches(&e, "on", Some("on")).await;
    let token = direct_turn(&e, "t-none").await;
    let args = json!({"action": "remix", "kit": "none", "why": "a shelf where everyone adds a book and votes for the next read"});
    let (text, err) = tool(&e, &token, pocket_make_tool::TOOL, args.clone()).await;
    assert!(!err, "{text}");
    assert!(text.contains(&format!("\"{}\"", pocket_make_tool::CUSTOM_APP)), "{text}");
    assert!(text.contains("nothing was made") && text.contains("no code"), "{text}");
    // V1 can't keep a custom-app promise (Mira F1/F3): it says so, offers a close kit, makes nothing else
    assert!(text.contains("I can't make a custom app from chat yet."), "{text}");
    assert!(!text.contains("I can make that as a custom app"), "{text}");
    assert!(text.contains("If a kit comes close, say what it would leave out and offer it."), "{text}");
    assert!(text.contains("Make nothing else") && text.contains("unless the person picks that kit"), "{text}");
    assert!(!text.contains("_create"), "names no kit tool: {text}");
    assert!(pocket_kits(&e).await.is_empty(), "no pocket row");
    let m = reply_as(&e, &e.bot, vec![call(pocket_make_tool::TOOL, args, "ok", &text)]).await;
    assert_eq!(pocket(&m), None, "no card");
}

#[tokio::test]
async fn a_kit_thats_off_or_has_no_chat_tool_gets_a_plain_answer_and_nothing_is_made() {
    let e = setup(false).await;
    // pocket_apps names only santa (and made): the quiz and the desk are off
    switches(&e, "santa,made", Some("on")).await;
    let token = turn(&e, &e.bot, "t-kit-off").await;
    for kit in ["quiz", "desk"] {
        let (text, err) = tool(&e, &token, pocket_make_tool::TOOL, remix(kit)).await;
        assert!(err, "{text}");
        assert!(text.contains("kit is off on this kwant, so nothing was made"), "{text}");
        assert!(text.ends_with("On this kwant now: santa (santa_create)."), "{text}");
        assert!(!text.contains("Template ("), "no rules for a kit that's off: {text}");
    }
    // queue, gather and wall have no chat tool on this build
    for (kit, name) in [("queue", "queue"), ("gather", "sign-up, RSVP or vote"), ("wall", "photo wall")] {
        let (text, err) = tool(&e, &token, pocket_make_tool::TOOL, remix(kit)).await;
        assert!(err, "{text}");
        assert!(text.starts_with(&format!("kwant can't make a {name} from chat yet, so nothing was made.")), "{text}");
    }
    // Nothing is on: says so
    switches(&e, "made", Some("on")).await;
    let token = turn(&e, &e.bot, "t-none-on").await;
    let (text, _) = tool(&e, &token, pocket_make_tool::TOOL, remix("santa")).await;
    assert!(text.ends_with("On this kwant now: no kit you can make from chat."), "{text}");
    assert!(pocket_kits(&e).await.is_empty());
}

#[tokio::test]
async fn bad_arguments_are_refused_and_nothing_is_echoed() {
    let e = setup(false).await;
    switches(&e, "on", Some("on")).await;
    let token = turn(&e, &e.bot, "t-bad").await;
    let cases = [
        (json!({"action": "draft", "kit": "quiz"}), "one action, remix"),
        (json!({"action": "publish"}), "one action, remix"),
        (json!({"action": "remix"}), "needs both"),
        (json!({"kit": "santa"}), "needs both"),
        (json!({"action": "remix", "kit": "made"}), "kit must be one of"),
        (json!({"action": "remix", "kit": "<img src=x onerror=alert(1)>"}), "kit must be one of"),
        (json!({"action": "remix", "kit": 7}), "must be a string"),
        (json!({"action": "remix", "kit": "santa", "why": "x".repeat(301)}), "at most 300"),
    ];
    for (args, want) in cases {
        let (text, err) = tool(&e, &token, pocket_make_tool::TOOL, args.clone()).await;
        assert!(err, "{args}: {text}");
        assert!(text.contains(want) && text.contains("Nothing was made"), "{args}: {text}");
        assert!(!text.contains("onerror") && !text.contains("xxxxxxxx"), "nothing echoed: {text}");
    }
    // `why` is never echoed, even on a good call
    let (text, err) = tool(&e, &token, pocket_make_tool::TOOL, json!({"action": "Remix", "kit": "Santa", "why": "<script>zz9</script>"})).await;
    assert!(!err, "{text}");
    assert!(!text.contains("zz9"), "{text}");
    assert!(pocket_kits(&e).await.is_empty());
}
