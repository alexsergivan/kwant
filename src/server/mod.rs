pub mod routes;
pub mod sse;

use crate::broker::PermissionBroker;
use crate::config::Config;
use crate::db::Database;
use crate::drivers::events::RuntimeEvent;
use crate::router::LayaRouter;
use crate::tools::ToolRunner;
use axum::{
    routing::{get, post},
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
        Self {
            config,
            db,
            tools,
            broker,
            router,
            event_sender,
            active_cancellations: Arc::new(Mutex::new(HashMap::new())),
        }
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
        .route("/api/events", get(sse::sse_handler))
        .fallback(crate::embedded::static_handler)
        .layer(cors)
        .with_state(state)
}
