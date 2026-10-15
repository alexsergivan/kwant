use clap::{Parser, Subcommand};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::Database,
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "kwnt", author, version, about = "Kwants: Multi-Agent AI Platform in Rust")]
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
    },

    /// Print the resolved config and whether local data files exist
    Status,

    /// List kwants stored in the local database
    Agents,

    /// List teams stored in the local database
    Teams,

    /// List channels stored in the local database
    Channels,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();
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

            let db = Database::init(&config.data_dir).await?;
            let tools = ToolRunner::new(config.data_dir.join("workspace"));
            let broker = PermissionBroker::new();
            let router = LayaRouter::new(config.laya_url.clone());

            let state = AppState::new(config.clone(), db, tools, broker, router);
            let app = create_router(state);

            let addr = format!("{}:{}", config.host, config.port);
            let listener = tokio::net::TcpListener::bind(&addr).await?;

            tracing::info!(
                host = %config.host,
                port = config.port,
                data_dir = %config.data_dir.display(),
                laya_url = %config.laya_url,
                "Starting Kwants engine..."
            );
            println!(
                "⚡ Kwants Multi-Agent AI Platform listening at http://{}",
                addr
            );

            axum::serve(listener, app).await?;
        }
        Some(Commands::Status) => print_status(&config),
        Some(Commands::Agents) => print_agents(&config).await?,
        Some(Commands::Teams) => print_teams(&config).await?,
        Some(Commands::Channels) => print_channels(&config).await?,
        None => {
            println!("⚡ Kwants: Multi-Agent AI Platform in Rust");
            println!("Run `kwnt serve` to start the server, or `kwnt --help` for options.");
        }
    }

    Ok(())
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

    println!("Kwants");
    println!("  host       {}", config.host);
    println!("  port       {}", config.port);
    println!("  data dir   {}", config.data_dir.display());
    println!("  config     {config_label}");
    println!("  database   {db_label}");
    println!("  laya       {}", config.laya_url);
    println!("  fallback   {}", config.default_fallback_model);
}

async fn print_agents(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let db = Database::init(&config.data_dir).await?;
    let kwants = db.list_kwants().await?;
    println!(
        "{:<16} {:<32} {:<8} {}",
        "NAME", "TITLE", "DRIVER", "MODEL"
    );
    if kwants.is_empty() {
        println!("(no kwants)");
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
    let teams = db.list_teams().await?;
    println!("{:<24} {:<8} {}", "TEAM", "MEMBERS", "WORKSPACE");
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
    let channels = db.list_channels().await?;
    println!("{:<24} {:<10} {}", "CHANNEL", "ROUTING", "DESCRIPTION");
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
