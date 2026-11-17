//! 26h: pictures an MCP tool returns are saved in the turn's chat as the bot's files and named for
//! the model, and paid generations (FLUX, Picsart credits) always ask. The FLUX-shaped tools of
//! `kwnt mcp-echo-server --media-tools` play Black Forest Labs' app, offline: no BFL or Picsart call.

mod common;

use async_trait::async_trait;
use common::*;
use kwnt::{
    broker::ApprovalDecision,
    db::{models::McpServer, spaces::SpaceId},
    drivers::events::RuntimeEvent,
    mcp::{
        session::{Caller, McpToolSession},
        ApprovalOutcome, McpTurnContext, ToolApprovalRequest, ToolApprover,
    },
    server::{
        attachments::ChatMedia,
        continuity::{SelfScope, TurnOrigin},
        mcp_runtime::BrokerApprover,
    },
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};

/// Allows every call and keeps the payment reason each was asked with.
#[derive(Default)]
struct Recording(Mutex<Vec<(String, Option<String>)>>);

#[async_trait]
impl ToolApprover for Recording {
    async fn approve(&self, req: ToolApprovalRequest) -> bool {
        self.0.lock().unwrap().push((req.tool.clone(), req.payment.clone()));
        true
    }
}

fn echo(name: &str) -> McpServer {
    McpServer {
        id: String::new(),
        name: name.into(),
        transport: "stdio".into(),
        command: Some(env!("CARGO_BIN_EXE_kwnt").into()),
        args: vec!["mcp-echo-server".into(), "--media-tools".into(), "--server-name".into(), name.into()],
        url: None,
        env: BTreeMap::new(),
        headers: BTreeMap::new(),
        enabled: true,
        timeout_secs: 10,
        created_at: None,
    }
}

fn scope(env: &Env) -> SelfScope {
    SelfScope {
        kwant_id: env.a.clone(),
        kwant_name: "A".into(),
        origin: Some(TurnOrigin { direct_kwant_id: Some(env.a.clone()), ..Default::default() }),
        turn_id: Some("t-media".into()),
        ..Default::default()
    }
}

fn ctx(env: &Env, server: McpServer, approver: Arc<dyn ToolApprover>, chat: bool) -> McpTurnContext {
    McpTurnContext {
        servers: vec![server],
        approver,
        config_dir: std::env::temp_dir(),
        permission_url: None,
        gateway: None,
        max_tool_iterations: 10,
        approvals: kwnt::mcp::Approvals::Ask,
        records: Default::default(),
        kwnt_only: false,
        media: chat.then(|| Arc::new(ChatMedia { state: env.state.clone(), scope: scope(env) }) as Arc<dyn kwnt::mcp::media::MediaSink>),
    }
}

fn caller(env: &Env) -> Caller {
    Caller { kwant_id: env.a.clone(), kwant_name: "A".into(), turn_id: "t-media".into() }
}

/// The bot's file messages in its direct chat: (text, meta).
async fn shared_files(env: &Env) -> Vec<(String, Value)> {
    messages(env, &env.a)
        .await
        .into_iter()
        .filter_map(|m| {
            let meta: Value = serde_json::from_str(m.meta.as_deref()?).ok()?;
            (meta["kind"] == "file").then_some((m.text, meta))
        })
        .collect()
}

#[tokio::test]
async fn flux_pictures_are_saved_in_the_chat_and_named_for_the_model() {
    let env = setup().await;
    let approver = Arc::new(Recording::default());
    let (session, problems) = McpToolSession::open(&ctx(&env, echo("flux"), approver.clone(), true), None).await;
    assert!(problems.is_empty(), "{problems:?}");

    let out = session.call(&caller(&env), "c1", "mcp__flux__generate_image", r#"{"prompt": "a red fox", "count": 2}"#).await;
    assert_eq!(out.status, "ok", "{}", out.text);
    let lines: Vec<&str> = out.text.lines().collect();
    assert_eq!(lines[0], "Generated 2 image(s) for: a red fox");
    assert!(lines[1].starts_with("Image saved in this chat as flux-generate_image-1.png (id "), "{}", out.text);
    assert!(lines[2].starts_with("Image saved in this chat as flux-generate_image-2.png (id "), "{}", out.text);
    // The model and the record never carry the base64 ("iVBOR" starts every PNG's)
    assert!(!out.text.contains("iVBOR"));
    assert!(!out.record.to_string().contains("iVBOR"));
    assert!(out.record.to_string().contains("flux-generate_image-1.png"), "{}", out.record);

    let files = shared_files(&env).await;
    assert_eq!(files.len(), 2, "{files:?}");
    for (text, meta) in &files {
        assert!(text.starts_with("Shared a file: flux-generate_image-"), "{text}");
        assert_eq!(meta["auto"], true);
        assert_eq!(meta["made_with"], "flux", "{meta}");
        assert_eq!(meta["turn_id"], "t-media");
        assert_eq!(meta["attachments"][0]["mime"], "image/png", "{meta}");
    }

    // The same pictures again: already in the chat, nothing posted twice
    let again = session.call(&caller(&env), "c2", "mcp__flux__generate_image", r#"{"prompt": "a red fox", "count": 2}"#).await;
    assert!(again.text.contains("Image already in this chat as flux-generate_image-1.png"), "{}", again.text);
    assert_eq!(shared_files(&env).await.len(), 2);

    // A read: no payment, and its text comes through as it is
    let credits = session.call(&caller(&env), "c3", "mcp__flux__get_credits", "{}").await;
    assert_eq!(credits.text, r#"{"credits": 412.5}"#);

    let asked = approver.0.lock().unwrap().clone();
    assert_eq!(asked[0], ("generate_image".to_string(), Some("generate_image uses your Black Forest Labs credits".to_string())));
    assert_eq!(asked[2], ("get_credits".to_string(), None));
    session.shutdown().await;
}

#[tokio::test]
async fn without_a_chat_the_model_hears_the_picture_wasnt_saved() {
    let env = setup().await;
    let approver = Arc::new(Recording::default());
    let (session, _) = McpToolSession::open(&ctx(&env, echo("flux"), approver, false), None).await;
    let out = session.call(&caller(&env), "c1", "mcp__flux__generate_image", r#"{"prompt": "x"}"#).await;
    assert_eq!(
        out.text,
        "Generated 1 image(s) for: x\n[image: image/png] An image came back but couldn't be saved: this turn has no chat to save it in."
    );
    assert!(shared_files(&env).await.is_empty());
    session.shutdown().await;
}

#[tokio::test]
async fn another_app_that_makes_pictures_is_not_a_payment_but_its_pictures_still_save() {
    let env = setup().await;
    let approver = Arc::new(Recording::default());
    let (session, _) = McpToolSession::open(&ctx(&env, echo("shop"), approver.clone(), true), None).await;
    let out = session.call(&caller(&env), "c1", "mcp__shop__generate_image", r#"{"prompt": "a mug"}"#).await;
    assert!(out.text.contains("Image saved in this chat as shop-generate_image-1.png"), "{}", out.text);
    assert_eq!(approver.0.lock().unwrap()[0], ("generate_image".to_string(), None));
    session.shutdown().await;
}

fn approver(env: &Env) -> BrokerApprover {
    BrokerApprover {
        db: env.state.db.clone(),
        data_dir: env.state.config.data_dir.clone(),
        broker: env.state.broker.clone(),
        events: env.state.event_sender.clone(),
        timeout: Duration::from_millis(300),
        conversation: env.a.clone(),
        chat: None,
        always_ask: false,
        strict_ask: false,
        started_by: None,
    }
}

/// A call to `server`'s `tool` the way an agent CLI asks it: no payment reason of its own.
fn cli_call(env: &Env, server: &str, tool: &str) -> ToolApprovalRequest {
    ToolApprovalRequest {
        kwant_id: env.a.clone(),
        kwant_name: "A".into(),
        turn_id: "t-media".into(),
        tool_name: format!("mcp__{server}__{tool}"),
        server: server.into(),
        tool: tool.into(),
        arguments: json!({"prompt": "a fox"}),
        builtin: None,
        smart: None,
        payment: None,
    }
}

/// A renamed BFL server (`images` at mcp.bfl.ai): its generations ask with approvals off and a
/// session grant, the card says whose credits, and "always this session" covers one call.
#[tokio::test]
async fn a_renamed_bfl_server_still_asks_and_the_card_names_the_credits() {
    let env = setup().await;
    let mut server = echo("images");
    server.transport = "http".into();
    server.command = None;
    server.args = vec![];
    server.url = Some("https://mcp.bfl.ai".into());
    let server = env.state.db.create_mcp_server(&SpaceId::default_space(), &server).await.unwrap();
    env.state.db.set_kwant_mcp_servers(&env.a, std::slice::from_ref(&server.id)).await.unwrap();
    env.state.db.set_kwant_approvals(&env.a, "off").await.unwrap();
    let a = env.a.clone();
    env.state.broker.allow_for_session(&a, &a, "mcp__images__generate_image").await;

    // A read runs without a card
    assert_eq!(approver(&env).decide(cli_call(&env, "images", "get_credits")).await, ApprovalOutcome::Allowed);

    // A generation asks; the card names the credits and never a price
    for decision in [ApprovalDecision::AlwaysAllowSession, ApprovalDecision::AllowOnce] {
        let mut events = env.state.event_sender.subscribe();
        let ap = Arc::new(approver(&env));
        let req = cli_call(&env, "images", "generate_image");
        let pending = tokio::spawn(async move { ap.decide(req).await });
        let (id, details) = loop {
            if let RuntimeEvent::ApprovalRequired { id, details, .. } = events.recv().await.unwrap() {
                break (id, details);
            }
        };
        assert_eq!(details["credits"], "Black Forest Labs", "{details}");
        assert_eq!(details["payment_reason"], "generate_image uses your Black Forest Labs credits");
        assert!(!details.to_string().contains('$'));
        assert!(env.state.broker.resolve(&id, decision).await);
        assert_eq!(pending.await.unwrap(), ApprovalOutcome::Allowed);
    }
}
