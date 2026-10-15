pub mod routes;
pub mod sse;
pub mod browser_stream;

use crate::broker::PermissionBroker;
use crate::config::Config;
use crate::db::Database;
use crate::drivers::events::RuntimeEvent;
use crate::router::LayaRouter;
use crate::sandbox::{BrowserController, SandboxManager};
use crate::tools::ToolRunner;
use axum::{
    routing::{delete, get, post},
    Router,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use tokio_util::sync::CancellationToken;
use tower_http::cors::{Any, CorsLayer};

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub db: Database,
    pub tools: ToolRunner,
    pub broker: PermissionBroker,
    pub router: LayaRouter,
    pub event_sender: broadcast::Sender<RuntimeEvent>,
    pub active_cancellations: Arc<Mutex<HashMap<String, CancellationToken>>>,
    pub sandbox_manager: Arc<SandboxManager>,
    pub browser_controllers: Arc<Mutex<HashMap<String, BrowserController>>>,
    pub browser_service: Arc<crate::sandbox::service::BrowserService>,
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
        let browser_service = Arc::new(crate::sandbox::service::BrowserService::new(config.data_dir.clone(), config.browser_cdp_url.clone()));
        Self {
            config,
            db,
            tools: tools.with_sandbox(sandbox_manager.clone()),
            broker,
            router,
            event_sender,
            active_cancellations: Arc::new(Mutex::new(HashMap::new())),
            sandbox_manager,
            browser_controllers,
            browser_service,
        }
    }

    pub fn with_sandbox_manager(mut self, manager: Arc<SandboxManager>) -> Self {
        self.sandbox_manager = manager.clone();
        self.tools = self.tools.with_sandbox(manager);
        self
    }
}

pub fn create_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/api/health", get(routes::health))
        .route("/api/kwants", get(routes::list_kwants).post(routes::create_kwant))
        .route("/api/channels", get(routes::list_channels).post(routes::create_channel))
        .route("/api/channels/:id/messages", get(routes::list_messages))
        .route("/api/teams", get(routes::list_teams).post(routes::create_team))
        .route("/api/teams/:id", get(routes::get_team).put(routes::update_team).delete(routes::delete_team))
        .route("/api/chat/send", post(routes::send_chat))
        .route("/api/chat/cancel", post(routes::cancel_chat))
        .route("/api/approvals/:id/resolve", post(routes::resolve_approval))
        .route("/api/settings", get(routes::get_settings).post(routes::save_settings))
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
        .route("/api/events", get(sse::sse_handler))
        .fallback(crate::embedded::static_handler)
        .layer(cors)
        .with_state(state)
}
