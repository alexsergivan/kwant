pub mod discussion;
pub mod routes;
pub mod sse;
pub mod browser_stream;
pub mod mcp_oauth;
pub mod mcp_routes;
pub mod mcp_runtime;
pub mod profile_routes;
pub mod profile_turn;
pub mod engine_routes;
pub mod auth;
pub mod continuity;
pub mod scheduler;
pub mod turns;
pub mod delegation;
pub mod attachments;
pub mod questions;
pub mod skills;
pub mod away;
pub mod computer;

use crate::broker::PermissionBroker;
use crate::config::Config;
use crate::db::Database;
use crate::drivers::events::RuntimeEvent;
use crate::router::LayaRouter;
use crate::sandbox::{BrowserController, SandboxManager};
use crate::tools::ToolRunner;
use axum::{
    routing::{delete, get, post, put},
    Router,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub db: Database,
    pub tools: ToolRunner,
    pub broker: PermissionBroker,
    pub router: LayaRouter,
    pub event_sender: broadcast::Sender<RuntimeEvent>,
    /// Running dispatches (their cancel tokens) and turns, and the caps on concurrent turns.
    pub turns: turns::TurnRegistry,
    pub sandbox_manager: Arc<SandboxManager>,
    pub browser_controllers: Arc<Mutex<HashMap<String, BrowserController>>>,
    pub browser_service: Arc<crate::sandbox::service::BrowserService>,
    /// Live claude CLI turns whose MCP permission prompts kwnt answers, by URL token.
    pub mcp_sessions: Arc<Mutex<HashMap<String, mcp_runtime::PermissionSession>>>,
    pub mcp_oauth: Arc<mcp_oauth::OAuthState>,
    /// LLM profiles: sign-in checks and in-app logins.
    pub profiles: Arc<crate::profiles::ProfileManager>,
    /// Which engines can run and their models: checked at startup and on Re-check.
    pub engines: Arc<crate::drivers::engines::EngineRegistry>,
    /// API token for requests from other machines (see `auth`).
    pub api_token: Arc<auth::ApiToken>,
    /// Live turns that have the agents' own tools, by bearer token.
    pub self_tools: Arc<Mutex<HashMap<String, continuity::SelfScope>>>,
    /// Routines with a run in progress; a run that would overlap is skipped.
    pub running_routines: Arc<Mutex<std::collections::HashSet<String>>>,
    /// Wakes the scheduler early when routines change.
    pub scheduler_wake: Arc<tokio::sync::Notify>,
    /// `ask_user` calls waiting for the user's answer.
    pub questions: Arc<questions::QuestionHub>,
}

impl AppState {
    pub fn new(
        config: Config,
        db: Database,
        tools: ToolRunner,
        broker: PermissionBroker,
        router: LayaRouter,
    ) -> Self {
        let (event_sender, _) = broadcast::channel(128);
        let sandbox_manager = Arc::new(SandboxManager::new());
        let browser_controllers = Arc::new(Mutex::new(HashMap::new()));
        let profiles = Arc::new(crate::profiles::ProfileManager::new(config.data_dir.clone()));
        let api_token = Arc::new(auth::ApiToken::load(&config));
        let browser_service = Arc::new(crate::sandbox::service::BrowserService::new(config.data_dir.clone(), config.browser_cdp_url.clone()));
        Self {
            config,
            db,
            tools: tools.with_sandbox(sandbox_manager.clone()),
            broker,
            router,
            event_sender,
            turns: turns::TurnRegistry::default(),
            sandbox_manager,
            browser_controllers,
            browser_service,
            mcp_sessions: Arc::new(Mutex::new(HashMap::new())),
            mcp_oauth: Arc::new(mcp_oauth::OAuthState::default()),
            profiles,
            engines: Default::default(),
            api_token,
            self_tools: Default::default(),
            running_routines: Default::default(),
            scheduler_wake: Default::default(),
            questions: Default::default(),
        }
    }

    pub fn with_sandbox_manager(mut self, manager: Arc<SandboxManager>) -> Self {
        self.sandbox_manager = manager.clone();
        self.tools = self.tools.with_sandbox(manager);
        self
    }
}

pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(routes::health))
        .route("/api/auth/status", get(auth::status))
        .route("/api/auth/session", post(auth::create_session).delete(auth::delete_session))
        .route("/api/profiles", get(profile_routes::list).post(profile_routes::create))
        .route("/api/profiles/:id", axum::routing::patch(profile_routes::update).delete(profile_routes::remove))
        .route("/api/profiles/:id/key", put(profile_routes::set_key).delete(profile_routes::delete_key))
        .route("/api/profiles/:id/login", post(profile_routes::start_login))
        .route(
            "/api/profiles/:id/login/:flow_id",
            get(profile_routes::get_login).delete(profile_routes::cancel_login),
        )
        .route("/api/profiles/:id/login/:flow_id/code", post(profile_routes::submit_login_code))
        .route("/api/profiles/:id/logout", post(profile_routes::logout))
        .route("/api/engines", get(engine_routes::list))
        .route("/api/engines/recheck", post(engine_routes::recheck))
        .route("/api/laya/status", get(routes::laya_status))
        .route("/api/kwants", get(routes::list_kwants).post(routes::create_kwant))
        .route("/api/kwants/:id", put(routes::update_kwant))
        .route("/api/channels", get(routes::list_channels).post(routes::create_channel))
        .route("/api/channels/:id/messages", get(routes::list_messages))
        .route("/api/teams", get(routes::list_teams).post(routes::create_team))
        .route(
            "/api/teams/:id",
            get(routes::get_team).put(routes::update_team).patch(continuity::patch_team).delete(routes::delete_team),
        )
        .route("/api/chat/send", post(routes::send_chat))
        .route("/api/chat/cancel", post(routes::cancel_chat))
        .route("/api/approvals/:id/resolve", post(routes::resolve_approval))
        .route("/api/settings", get(routes::get_settings).post(routes::save_settings))
        .route("/api/settings/computer", get(computer::get_computer).put(computer::put_computer))
        .route("/api/system/directories", get(routes::list_system_directories))
        .route("/api/system/select-folder", post(routes::select_folder))
        .route("/api/sandbox/:team_id/status", get(routes::get_sandbox_status))
        .route("/api/sandbox/:team_id/screenshot", get(routes::get_browser_screenshot))
        .route("/api/sandbox/:team_id/restart", post(routes::restart_sandbox))
        .route("/api/sandbox/:team_id/navigate", post(routes::navigate_sandbox))
        .route("/api/sandbox/:team_id/interact", post(routes::interact_sandbox))
        .route("/api/sandbox/:team_id/stream", get(browser_stream::upgrade))
        .route("/api/sandbox/:team_id/page", get(routes::get_sandbox_page))
        .route("/api/sandbox/:team_id/act", post(routes::act_sandbox))
        .route("/api/sandbox/:team_id/passwords", get(routes::get_saved_passwords).post(routes::save_password))
        .route("/api/sandbox/:team_id/passwords/:id", delete(routes::delete_password))
        .route("/api/mcp/servers", get(mcp_routes::list_servers).post(mcp_routes::create_server))
        .route(
            "/api/mcp/servers/:id",
            get(mcp_routes::get_server).put(mcp_routes::update_server).delete(mcp_routes::delete_server),
        )
        .route("/api/mcp/servers/:id/test", post(mcp_routes::test_server))
        .route("/api/mcp/servers/:id/oauth/start", post(mcp_oauth::start))
        .route("/api/mcp/servers/:id/oauth", delete(mcp_oauth::disconnect))
        .route("/api/mcp/oauth/callback", get(mcp_oauth::callback))
        .route("/api/kwants/:id/mcp-servers", get(mcp_routes::get_kwant_servers).put(mcp_routes::set_kwant_servers))
        .route("/api/mcp/permission/:token", post(mcp_runtime::permission_endpoint))
        .route("/api/mcp/gateway", post(mcp_runtime::gateway_endpoint))
        .route("/api/mcp/gateway/grok", post(mcp_runtime::gateway_grok_endpoint))
        .route("/api/mcp/self", post(continuity::self_endpoint))
        .route("/api/channels/:id", get(continuity::get_channel).patch(continuity::patch_channel))
        .route("/api/search", get(continuity::search))
        .route("/api/kwants/:id/memories", get(continuity::list_memories).post(continuity::add_memory))
        .route(
            "/api/kwants/:id/memories/:mid",
            axum::routing::patch(continuity::update_memory).delete(continuity::delete_memory),
        )
        .route("/api/routines", get(scheduler::list).post(scheduler::create))
        .route(
            "/api/routines/:id",
            get(scheduler::get).patch(scheduler::update).delete(scheduler::remove),
        )
        .route("/api/routines/:id/run", post(scheduler::run_now))
        .route("/api/routines/:id/runs", get(scheduler::runs))
        .route("/api/turns", get(delegation::list_turns))
        .route("/api/kwants/:id/threads", get(delegation::list_threads).post(delegation::create_thread))
        .route("/api/threads/:id", get(delegation::get_thread).patch(delegation::patch_thread))
        .route("/api/threads/:id/messages", get(delegation::thread_messages))
        .route("/api/threads/:id/cancel", post(delegation::cancel_thread))
        .route("/api/delegations", get(delegation::list_delegations).post(delegation::create_delegation))
        .route("/api/delegations/:id", get(delegation::get_delegation))
        .route("/api/delegations/:id/cancel", post(delegation::cancel_delegation))
        .route("/api/attachments", get(attachments::list).post(attachments::upload))
        .route("/api/attachments/:id", get(attachments::get))
        .route("/api/attachments/:id/content", get(attachments::content))
        .route("/api/questions", get(questions::list))
        .route("/api/questions/:id", get(questions::get))
        .route("/api/questions/:id/answer", post(questions::answer))
        .route("/api/questions/:id/cancel", post(questions::cancel))
        .route("/api/skills", get(skills::list).post(skills::create))
        .route("/api/skills/:id", get(skills::get).patch(skills::update).delete(skills::remove))
        .route("/api/skills/:id/diff", get(skills::diff))
        .route("/api/skills/:id/approve", post(skills::approve))
        .route("/api/skills/:id/reject", post(skills::reject))
        .route("/api/away", get(away::get))
        .route("/api/away/seen", post(away::seen))
        .route("/api/events", get(sse::sse_handler))
        .fallback(crate::embedded::static_handler)
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth::guard))
        .with_state(state)
}
