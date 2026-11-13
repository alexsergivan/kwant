//! Hover-only controls and clickable cards in kwnt's own browser, through a real Chrome: a tile
//! grid where each tile is a div with a JS click listener and its ⋮ button is opacity 0 until the
//! mouse is over the tile (Google Flow's clip grid). The list names the tiles as cards and the ⋮
//! buttons as "More" (on hover) with their tile; a click on one, by label or by ref, hovers the
//! tile first and then opens its menu, whose Download item is clicked next. The hover action says
//! what appeared. Cards and hover controls are capped at 40 each, icon-only buttons are named by
//! their icon word, and the payment check still judges the same element and label. Skipped when
//! no Chrome is installed.
#[path = "support/chrome_guard.rs"]
mod chrome_guard;
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use chrome_guard::ChromeGuard;
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{models::Kwant, spaces::SpaceId, Database},
    router::LayaRouter,
    sandbox::{computer, BrowserError},
    server::{continuity::browser_act_payment, create_router, routes::get_or_create_browser_controller, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

/// Tiles as Google Flow draws them: a div with a click listener (no button, no link, no role),
/// a ⋮ icon button at opacity 0 that `.tile:hover` shows, and a menu it opens in a portal.
const TILES_PAGE: &str = r##"<!doctype html><title>flow</title>
<style>
body{font:14px sans-serif;margin:20px}
.grid{display:grid;grid-template-columns:repeat(4,200px);gap:12px}
.tile{position:relative;height:140px;background:#333;color:#fff;cursor:pointer}
.tile .more{position:absolute;top:6px;right:6px;opacity:0}
.tile:hover .more{opacity:1}
.tile .share{position:absolute;top:6px;left:6px;visibility:hidden}
.tile:hover .share{visibility:visible}
.menu{position:fixed;background:#fff;border:1px solid #000;color:#000}
.menu [role=menuitem]{padding:6px 12px;cursor:pointer}
</style>
<div class=grid>
 <div class=tile data-name="Clip 1"><div>Clip 1</div><button class=more><i class="google-symbols">more_vert</i></button></div>
 <div class=tile data-name="Clip 2"><div>Clip 2</div><button class=more><i class="google-symbols">more_vert</i></button></div>
 <div class=tile data-name="Clip 3"><div>Clip 3</div><button class=share><span class="material-symbols-outlined">share</span></button></div>
 <div class=tile id=js data-name="Clip 4"><div>Clip 4</div><button id=edit style="position:absolute;bottom:6px;left:6px;opacity:0"><i class="material-icons">edit</i></button></div>
 <div class=tile data-name="Clip 5"><div>Clip 5</div><button id=never style="position:absolute;bottom:6px;right:6px;opacity:0" aria-label="Never shown"></button><button style="display:none">Gone for good</button></div>
</div>
<div id=portal></div>
<script>
document.querySelectorAll('.tile').forEach(function (t) {
  t.addEventListener('click', function (e) { if (e.target.closest('button')) return; document.title = 'opened ' + t.dataset.name; });
  var more = t.querySelector('.more');
  if (more) more.addEventListener('click', function (e) {
    e.stopPropagation();
    var r = this.getBoundingClientRect(), p = document.getElementById('portal');
    p.innerHTML = '<div class=menu role=menu style="left:' + r.left + 'px;top:' + (r.bottom + 4) + 'px"><div role=menuitem>Download</div><div role=menuitem>Delete</div></div>';
    p.querySelector('[role=menuitem]').addEventListener('click', function () { document.title = 'download ' + t.dataset.name; p.innerHTML = ''; });
  });
  var share = t.querySelector('.share');
  if (share) share.addEventListener('click', function (e) { e.stopPropagation(); document.title = 'share ' + t.dataset.name; });
});
// Shown by a script on mouseenter, not by CSS
var js = document.getElementById('js'), edit = document.getElementById('edit');
js.addEventListener('mouseenter', function () { edit.style.opacity = 1; });
js.addEventListener('mouseleave', function () { edit.style.opacity = 0; });
edit.addEventListener('click', function (e) { e.stopPropagation(); document.title = 'edit Clip 4'; });
</script>"##;

/// Icon-only buttons and cards named every way the list knows.
const LABELS_PAGE: &str = r##"<!doctype html><title>labels</title>
<button><i class="material-icons">more_vert</i></button>
<button><span class="material-symbols-outlined">more_horiz</span></button>
<button aria-label="Close dialog"><i class="material-icons">close</i></button>
<button title="Remove clip"><i class="material-icons">delete</i></button>
<button class="material-icons">download</button>
<a href="#s"><i class="google-symbols">share</i></a>
<button><i class="material-icons">photo_camera</i></button>
<button>close</button>
<div style="cursor:pointer;width:200px;height:60px" aria-label="Open project">x</div>
<div style="cursor:pointer;width:200px;height:60px"><img alt="Sunset clip" width=50 height=50></div>
<div style="cursor:pointer;width:200px;height:60px"><button style="width:200px;height:60px">Big</button></div>
<div style="cursor:pointer;position:fixed;inset:0;z-index:-1">Backdrop</div>"##;

const CARD_PAGE: &str = "<!doctype html><title>card</title><form onsubmit=\"return false\">\
<input autocomplete=cc-number placeholder='Card number'><button type=submit>Continue</button></form>";

/// 60 clickable cards, of which the list shows 40.
fn many_page() -> String {
    let cards: String = (0..60)
        .map(|i| format!("<div style='cursor:pointer;display:inline-block;width:120px;height:40px;margin:2px'>Card {i}</div>"))
        .collect();
    format!("<!doctype html><title>many</title>{cards}")
}

/// A small HTTP server for the pages above on 127.0.0.1, so the test needs no network.
async fn serve() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                let page = match path.as_str() {
                    "/tiles" => TILES_PAGE.to_string(),
                    "/labels" => LABELS_PAGE.to_string(),
                    "/card" => CARD_PAGE.to_string(),
                    _ => many_page(),
                };
                let reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{page}",
                    page.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            });
        }
    });
    format!("http://127.0.0.1:{port}")
}

struct Env {
    app: Router,
    state: AppState,
    bot: Kwant,
    tmp: TempDir,
}

/// kwnt in Local mode, so the bot's browser is a real Chrome on this machine.
async fn setup() -> Env {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    db.set_setting(computer::SETTING_KEY, "local").await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), ..Config::default() };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    let bot = state.db.list_kwants(&SpaceId::default_space()).await.unwrap()[0].clone();
    Env { app: create_router(state.clone()), state, bot, tmp }
}

async fn open(e: &Env, url: &str) -> kwnt::sandbox::BrowserController {
    let ctrl = get_or_create_browser_controller(&e.state, &e.bot.id).await.expect("local Chrome session");
    ctrl.navigate(url).await.unwrap();
    ctrl
}


async fn pay(e: &Env, args: Value) -> Option<String> {
    browser_act_payment(&e.state, &e.bot.id, &args).await
}
async fn post_act(app: &Router, bot: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/sandbox/{bot}/act"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn get_page(app: &Router, bot: &str) -> Value {
    let req = Request::builder().uri(format!("/api/sandbox/{bot}/page")).body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// The ref of the first listed control whose line contains `needle`.
fn ref_of(page: &Value, needle: &str) -> u64 {
    let lines = page["clickable"].as_array().expect("a clickable list");
    let line = lines.iter().filter_map(Value::as_str).find(|l| l.contains(needle)).unwrap_or_else(|| panic!("{needle} in {page}"));
    let line = line.trim_start_matches('*');
    line[1..line.find(']').unwrap()].parse().unwrap()
}


fn lines(page: &Value) -> Vec<String> {
    page["clickable"].as_array().expect("a clickable list").iter().filter_map(Value::as_str).map(|l| l.trim_start_matches('*').to_string()).collect()
}

#[tokio::test]
async fn a_hover_only_menu_on_a_tile_opens_and_downloads_by_text_and_by_ref() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.tmp.path().join("local-chrome"));
    let base = serve().await;
    let ctrl = open(&e, &format!("{base}/tiles")).await;

    // The tiles are cards; each ⋮ button is "More", shown on hover, with its tile
    let page = get_page(&e.app, &e.bot.id).await;
    let list = lines(&page);
    for want in [
        r#"card "Clip 1""#,
        r#"button "More" (on hover) · Clip 1"#,
        r#"button "More" (on hover) · Clip 2"#,
        r#"button "Share" (on hover) · Clip 3"#,
        r#"button "Edit" (on hover) · Clip 4"#,
    ] {
        assert!(list.iter().any(|l| l.ends_with(want)), "{want} in {list:#?}");
    }
    assert!(!list.iter().any(|l| l.contains("Gone for good")), "display:none is never listed: {list:#?}");
    assert!(!list.iter().any(|l| l.contains("more_vert")), "the icon word is read as More: {list:#?}");

    // By text: More hovers Clip 1 first, then opens its menu; Download is in it
    let (status, page) = post_act(&e.app, &e.bot.id, json!({"action": "click_text", "text": "More"})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page["error"].is_null(), "{page}");
    assert_eq!(page["result"], "clicked \"More\"", "{page}");
    assert!(lines(&page).iter().any(|l| l.ends_with(r#"menuitem "Download""#)), "{page}");
    let (_, page) = post_act(&e.app, &e.bot.id, json!({"action": "click_text", "text": "Download"})).await;
    assert_eq!(page["title"], "download Clip 1", "{page}");

    // By ref: Clip 2's More, then the Download the menu brought up
    open(&e, &format!("{base}/tiles")).await;
    let page = get_page(&e.app, &e.bot.id).await;
    let (_, page) = post_act(&e.app, &e.bot.id, json!({"action": "click", "ref": ref_of(&page, "(on hover) · Clip 2")})).await;
    assert!(page["error"].is_null(), "{page}");
    let download = page["clickable"].as_array().unwrap().iter().filter_map(Value::as_str).find(|l| l.ends_with(r#"menuitem "Download""#)).unwrap().to_string();
    assert!(download.starts_with('*'), "the menu's items are new: {download}");
    let (_, page) = post_act(&e.app, &e.bot.id, json!({"action": "click", "ref": ref_of(&page, r#"menuitem "Download""#)})).await;
    assert_eq!(page["title"], "download Clip 2", "{page}");

    // Shown by visibility (CSS) and by a mouseenter script: both click
    open(&e, &format!("{base}/tiles")).await;
    let page = get_page(&e.app, &e.bot.id).await;
    let (_, page) = post_act(&e.app, &e.bot.id, json!({"action": "click", "ref": ref_of(&page, "(on hover) · Clip 3")})).await;
    assert_eq!(page["title"], "share Clip 3", "{page}");
    let (_, page) = post_act(&e.app, &e.bot.id, json!({"action": "click_text", "text": "Edit"})).await;
    assert_eq!(page["title"], "edit Clip 4", "{page}");

    // The tile itself, a div with a click listener, by its name
    let (_, page) = post_act(&e.app, &e.bot.id, json!({"action": "click_text", "text": "Clip 2"})).await;
    assert_eq!(page["title"], "opened Clip 2", "{page}");

    // A button that never shows: hovered, still hidden, so nothing is clicked
    open(&e, &format!("{base}/tiles")).await;
    let page = get_page(&e.app, &e.bot.id).await;
    let never = ref_of(&page, "Never shown") as u32;
    let err = ctrl.click_ref(never).await.unwrap_err();
    assert!(matches!(err, BrowserError::StaleRef(_)), "{err}");
    assert!(err.to_string().contains("stayed hidden after hovering over \"Clip 5\""), "{err}");
    let err = ctrl.click_target(&kwnt::sandbox::ClickTarget::Text("Never shown".into())).await.unwrap_err();
    assert!(err.to_string().contains("Didn't click \"Never shown\": it stayed hidden"), "{err}");
    assert_eq!(ctrl.sync_location().await.unwrap().1, "flow");
}

#[tokio::test]
async fn a_hover_says_what_appeared_and_clicks_nothing() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.tmp.path().join("local-chrome"));
    let base = serve().await;
    open(&e, &format!("{base}/tiles")).await;

    let (status, page) = post_act(&e.app, &e.bot.id, json!({"action": "hover", "text": "Clip 1"})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["result"], "Hovered over \"Clip 1\". Appeared: More (button)", "{page}");
    assert_eq!(page["changed"], "Appeared: More (button)", "{page}");
    assert_eq!(page["title"], "flow", "a hover clicks nothing: {page}");

    // By ref, a hover-only control: its card is hovered
    let page = get_page(&e.app, &e.bot.id).await;
    let (_, page) = post_act(&e.app, &e.bot.id, json!({"action": "hover", "ref": ref_of(&page, "(on hover) · Clip 2")})).await;
    assert!(page["result"].as_str().unwrap().starts_with("Hovered over \"Clip 2\". Appeared: More (button)"), "{page}");

    // Nothing to hover, and no target at all
    let (_, page) = post_act(&e.app, &e.bot.id, json!({"action": "hover", "text": "No such thing"})).await;
    assert!(page["error"].as_str().unwrap().contains("Didn't hover over \"No such thing\""), "{page}");
    let (_, page) = post_act(&e.app, &e.bot.id, json!({"action": "hover"})).await;
    assert!(page["error"].as_str().unwrap().contains("hover needs ref"), "{page}");

    // A hover never pays
    assert_eq!(pay(&e, json!({"action": "hover", "text": "More"})).await, None);
}

#[tokio::test]
async fn cards_are_capped_and_icon_buttons_are_named() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.tmp.path().join("local-chrome"));
    let base = serve().await;

    let ctrl = open(&e, &format!("{base}/many")).await;
    let all = ctrl.list_clickables(400).await;
    assert_eq!(all.iter().filter(|l| l.contains("] card \"Card ")).count(), 40, "{all:#?}");

    open(&e, &format!("{base}/labels")).await;
    let all: Vec<String> = ctrl.list_clickables(400).await;
    let names: Vec<&str> = all.iter().map(|l| l.split_once("] ").unwrap().1).collect();
    assert_eq!(
        names,
        [
            r#"button "More""#,
            r#"button "More""#,
            r#"button "Close dialog""#,
            r#"button "Remove clip""#,
            r#"button "Download""#,
            r#"link "Share""#,
            r#"button "photo_camera""#,
            r#"button "close""#,
            r#"card "Open project""#,
            r#"card "Sunset clip""#,
            r#"button "Big""#,
        ],
        "{all:#?}"
    );
}

#[tokio::test]
async fn the_payment_check_still_judges_the_same_control() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.tmp.path().join("local-chrome"));
    let base = serve().await;

    open(&e, &format!("{base}/card")).await;
    let page = get_page(&e.app, &e.bot.id).await;
    let why = pay(&e, json!({"action": "click_text", "text": "Continue"})).await.expect("Continue on a card form");
    assert!(why.contains("card fields"), "{why}");
    let why = pay(&e, json!({"action": "click", "ref": ref_of(&page, "Continue")})).await.expect("by ref too");
    assert!(why.contains("card fields"), "{why}");

    // On the tile page nothing is a payment: More, a hover-only control, and a card
    open(&e, &format!("{base}/tiles")).await;
    let page = get_page(&e.app, &e.bot.id).await;
    assert_eq!(pay(&e, json!({"action": "click_text", "text": "More"})).await, None);
    assert_eq!(pay(&e, json!({"action": "click", "ref": ref_of(&page, "(on hover) · Clip 1")})).await, None);
    assert_eq!(pay(&e, json!({"action": "click_text", "text": "Clip 1"})).await, None);
}
