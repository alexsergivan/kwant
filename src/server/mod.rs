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
use tokio::sync::broadcast;
use tower_http::cors::{Any, CorsLayer};

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub db: Database,
    pub tools: ToolRunner,
    pub broker: PermissionBroker,
    pub router: LayaRouter,
    pub event_sender: broadcast::Sender<RuntimeEvent>,
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
        .route("/api/chat/send", post(routes::send_chat))
        .route("/api/approvals/:id/resolve", post(routes::resolve_approval))
        .route("/api/events", get(sse::sse_handler))
        .fallback(crate::embedded::static_handler)
        .layer(cors)
        .with_state(state)
}
