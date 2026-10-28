//! `kwnt serve` as a library call: opens the database, starts the Laya sidecar and background
//! jobs, and serves the API and embedded UI until `shutdown` resolves. The CLI and the Mac app
//! (apps/mac) both start kwant through [`run`], so they behave the same.

use crate::{
    broker::PermissionBroker,
    config::Config,
    db::Database,
    router::{
        sidecar::{self, SidecarOptions},
        LayaRouter,
    },
    server::{create_router, AppState},
    tools::ToolRunner,
};
use std::future::Future;
use tokio_util::sync::CancellationToken;

pub type ServeError = Box<dyn std::error::Error + Send + Sync>;

/// Runs the server on `config.host:config.port` until `shutdown` resolves, then stops the
/// background jobs, stops kwant's browser container and gives the Laya sidecar up to 5 s to exit.
pub async fn run(config: Config, shutdown: impl Future<Output = ()>) -> Result<(), ServeError> {
    let db = Database::init(&config.data_dir).await?;
    restrict_data_dir(&config.data_dir);
    let tools = ToolRunner::new(config.data_dir.join("workspace"));
    let broker = PermissionBroker::new();
    let router = LayaRouter::new(config.laya_url.clone());
    let cancel = CancellationToken::new();
    let sidecar = sidecar::spawn(
        router.clone(),
        SidecarOptions {
            data_dir: config.data_dir.clone(),
            managed: config.laya_managed,
            models: config.laya_models.clone(),
        },
        cancel.clone(),
    );
    if config.laya_managed {
        println!(
            "🔮 Laya router at {} (managed; logs in {})",
            router.base_url,
            config.data_dir.join("laya").display()
        );
    } else {
        println!("🔮 Laya router at {} (external)", router.base_url);
    }

    let state = AppState::new(config.clone(), db, tools, broker, router);
    // Saved passwords left in a bot's folder by older versions move into the vault
    crate::server::passwords::migrate_all(&state).await;
    crate::server::passwords::warn_if_exposed(&state).await;
    // Before anything can start a turn: say which replies the last run cut off
    crate::server::routes::interrupt_running_turns(&state).await;
    crate::server::scheduler::spawn_background(state.clone(), cancel.clone());
    crate::server::engine_routes::spawn_startup_check(state.clone());
    crate::server::parallel_routes::spawn_startup_sweep(state.clone());
    crate::server::app_update_routes::spawn_notifier(state.clone());
    // browser containers left by this or another kwant: remove orphans, stop idle ones
    let browser = state.browser_service.clone();
    browser.clone().spawn_reaper(cancel.clone());
    let api_token = state.api_token.clone();
    let app = create_router(state);

    let addr = format!("{}:{}", config.host, config.port);
    crate::mcp::shell_risk::set_serve_port(config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    tracing::info!(
        host = %config.host,
        port = config.port,
        data_dir = %config.data_dir.display(),
        laya_url = %config.laya_url,
        "Starting Kwant..."
    );
    println!(
        "⚡ kwant bots: your team of super-intelligent bots, listening at http://{}",
        addr
    );
    // stdout only: a one-time link, never the API token
    if crate::server::auth::startup_link_needed(&config.host, api_token.require) {
        let code = api_token.signin.create_link(crate::server::signin::LINK_TTL);
        let link = crate::server::auth::one_time_link(&config.host, config.port, api_token.public_base(), &code);
        for line in crate::server::auth::one_time_notice(&link, true) {
            println!("{line}");
        }
    }

    let app = app.into_make_service_with_connect_info::<std::net::SocketAddr>();
    tokio::select! {
        res = axum::serve(listener, app) => res?,
        _ = shutdown => println!("Shutting down…"),
    }
    cancel.cancel();
    browser.shutdown().await;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), sidecar).await;
    Ok(())
}

/// The data dir holds the database (keys, MCP tokens) and profile logins: owner only.
fn restrict_data_dir(dir: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        for name in ["kwnt.db", "kwnt.db-wal", "kwnt.db-shm"] {
            let path = dir.join(name);
            if path.exists() {
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
        }
    }
    #[cfg(not(unix))]
    let _ = dir;
}
