use clap::{Parser, Subcommand};
use kwnt::{
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
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

#[derive(Parser, Debug)]
#[command(name = "kwnt", author, version, about = "kwant bots: your team of AI bots")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    #[arg(short, long, help = "Path to config file")]
    pub config: Option<PathBuf>,

    /// Path to the Kwants data directory. Available on every subcommand.
    #[arg(long, global = true)]
    pub data_dir: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Start the Kwants HTTP/SSE server and embedded web UI
    Serve {
        #[arg(long, help = "Server host interface to bind")]
        host: Option<String>,

        #[arg(short, long, help = "Server port to listen on")]
        port: Option<u16>,

        #[arg(long, help = "Laya router service URL")]
        laya_url: Option<String>,

        #[arg(long, help = "Do not install or start a local Laya router")]
        no_laya: bool,
    },

    /// Print the resolved config and whether local data files exist
    Status,

    /// List kwants stored in the local database
    Agents,

    /// List teams stored in the local database
    Teams,

    /// List channels stored in the local database
    Channels,

    /// Print the API token that browsers and scripts on other machines need
    Token,

    /// Run a tiny MCP test server on stdin/stdout (for tests and smoke checks)
    #[command(hide = true)]
    McpEchoServer,

    /// Run an OAuth-protected mock MCP server over HTTP (for tests and smoke checks)
    #[command(hide = true)]
    McpOauthMockServer {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 7790)]
        port: u16,
        /// expires_in of issued access tokens; under 60 makes kwnt refresh on every use
        #[arg(long, default_value_t = 3600)]
        expires_in: i64,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    // Before logging starts: stdout carries the MCP protocol
    if matches!(cli.command, Some(Commands::McpEchoServer)) {
        // A test helper that holds nothing: left dumpable so tests can find it by its environment
        kwnt::mcp::echo_server::run_stdio().await?;
        return Ok(());
    }
    kwnt::harden_process();
    tracing_subscriber::fmt::init();
    if let Some(Commands::McpOauthMockServer { host, port, expires_in }) = &cli.command {
        let listener = tokio::net::TcpListener::bind((host.as_str(), *port)).await?;
        println!("mock OAuth MCP server: http://{}/mcp", listener.local_addr()?);
        kwnt::mcp::oauth_mock::serve(listener, kwnt::mcp::oauth_mock::MockOptions { expires_in: *expires_in }).await?;
        return Ok(());
    }

    let mut config = match cli.config {
        Some(path) => Config::from_file(path)?,
        None => Config::load()?,
    };

    if let Some(data_dir) = cli.data_dir {
        config.data_dir = data_dir;
    }

    match cli.command {
        Some(Commands::Serve {
            host,
            port,
            laya_url,
            no_laya,
        }) => {
            if let Some(h) = host {
                config.host = h;
            }
            if let Some(p) = port {
                config.port = p;
            }
            if let Some(l) = laya_url {
                config.laya_url = l;
            }
            if no_laya {
                config.laya_managed = false;
            }

            let db = Database::init(&config.data_dir).await?;
            restrict_data_dir(&config.data_dir);
            let tools = ToolRunner::new(config.data_dir.join("workspace"));
            let broker = PermissionBroker::new();
            let router = LayaRouter::new(config.laya_url.clone());
            let shutdown = CancellationToken::new();
            let sidecar = sidecar::spawn(
                router.clone(),
                SidecarOptions {
                    data_dir: config.data_dir.clone(),
                    managed: config.laya_managed,
                    models: config.laya_models.clone(),
                },
                shutdown.clone(),
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
            kwnt::server::passwords::migrate_all(&state).await;
            kwnt::server::scheduler::spawn_background(state.clone(), shutdown.clone());
            kwnt::server::engine_routes::spawn_startup_check(state.clone());
            let app = create_router(state);

            let addr = format!("{}:{}", config.host, config.port);
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
            if !is_loopback_bind(&config.host) {
                println!(
                    "🔒 Requests from other machines need the API token: run `kwnt token` and open http://<this-host>:{}/#kwnt_token=<token>",
                    config.port
                );
            } else {
                println!(
                    "🔒 Saved passwords and approvals need the API token here too: run `kwnt token` and open http://localhost:{}/#kwnt_token=<token> once",
                    config.port
                );
            }

            let app = app.into_make_service_with_connect_info::<std::net::SocketAddr>();
            tokio::select! {
                res = axum::serve(listener, app) => res?,
                _ = shutdown_signal() => println!("Shutting down…"),
            }
            shutdown.cancel();
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), sidecar).await;
        }
        Some(Commands::Status) => print_status(&config),
        Some(Commands::Agents) => print_agents(&config).await?,
        Some(Commands::Teams) => print_teams(&config).await?,
        Some(Commands::Channels) => print_channels(&config).await?,
        Some(Commands::Token) => {
            let token = std::env::var("KWNT_API_TOKEN")
                .ok()
                .map(|t| t.trim().to_string())
                .filter(|t| t.len() >= 16)
                .unwrap_or_else(|| kwnt::server::auth::load_or_create(&config.data_dir));
            println!("{token}");
        }
        Some(Commands::McpEchoServer) => unreachable!("handled before config loading"),
        Some(Commands::McpOauthMockServer { .. }) => unreachable!("handled before config loading"),
        None => {
            println!("⚡ kwant bots: your team of super-intelligent bots");
            println!("Run `kwnt serve` to start the server, or `kwnt --help` for options.");
        }
    }

    Ok(())
}

fn is_loopback_bind(host: &str) -> bool {
    host == "localhost"
        || host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
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

fn print_status(config: &Config) {
    let config_path = config.data_dir.join("config.toml");
    let db_path = config.data_dir.join("kwnt.db");
    let db_label = if db_path.exists() {
        let bytes = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);
        format!("{} ({bytes} bytes)", db_path.display())
    } else {
        "not created yet".to_string()
    };
    let config_label = if config_path.exists() {
        config_path.display().to_string()
    } else {
        format!("{} (absent)", config_path.display())
    };

    println!("kwant bots");
    println!("  host       {}", config.host);
    println!("  port       {}", config.port);
    println!("  data dir   {}", config.data_dir.display());
    println!("  config     {config_label}");
    println!("  database   {db_label}");
    println!(
        "  laya       {} ({}; models {})",
        config.laya_url,
        if config.laya_managed { "managed" } else { "external" },
        config.laya_models
    );
    println!("  fallback   {}", config.default_fallback_model);
}

async fn print_agents(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let db = Database::init(&config.data_dir).await?;
    let kwants = db.list_kwants(&kwnt::db::spaces::SpaceId::default_space()).await?;
    println!(
        "{:<16} {:<32} {:<8} MODEL",
        "NAME", "TITLE", "DRIVER"
    );
    if kwants.is_empty() {
        println!("(no bots)");
        return Ok(());
    }
    for kwant in kwants {
        println!(
            "{:<16} {:<32} {:<8} {}",
            kwant.name, kwant.title, kwant.driver_type, kwant.preferred_model
        );
    }
    Ok(())
}

async fn print_teams(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let db = Database::init(&config.data_dir).await?;
    let teams = db.list_teams(&kwnt::db::spaces::SpaceId::default_space()).await?;
    println!("{:<24} {:<8} WORKSPACE", "TEAM", "MEMBERS");
    if teams.is_empty() {
        println!("(no teams)");
        return Ok(());
    }
    for team in teams {
        let members = serde_json::from_str::<Vec<String>>(&team.member_kwants)
            .map(|ids| ids.len())
            .unwrap_or(0);
        let workspace = team.workspace_path.unwrap_or_else(|| "-".to_string());
        println!("{:<24} {:<8} {workspace}", team.name, members);
    }
    Ok(())
}

async fn print_channels(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let db = Database::init(&config.data_dir).await?;
    let channels = db.list_channels(&kwnt::db::spaces::SpaceId::default_space()).await?;
    println!("{:<24} {:<10} DESCRIPTION", "CHANNEL", "ROUTING");
    if channels.is_empty() {
        println!("(no channels)");
        return Ok(());
    }
    for channel in channels {
        let description = channel.description.unwrap_or_else(|| "-".to_string());
        println!(
            "{:<24} {:<10} {description}",
            channel.name, channel.routing_mode
        );
    }
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut sig) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            sig.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
}
