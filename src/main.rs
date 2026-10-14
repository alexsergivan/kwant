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
#[command(name = "kwnt", author, version, about = "Kwants: 8-Bit AI Agent Platform in Rust")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    #[arg(short, long, help = "Path to config file")]
    pub config: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Start the Kwants HTTP/SSE server and embedded 8-bit arcade UI
    Serve {
        #[arg(long, help = "Server host interface to bind")]
        host: Option<String>,

        #[arg(short, long, help = "Server port to listen on")]
        port: Option<u16>,

        #[arg(long, help = "Path to Kwants data directory")]
        data_dir: Option<PathBuf>,

        #[arg(long, help = "Laya router service URL")]
        laya_url: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();
    let mut config = match cli.config {
        Some(path) => Config::from_file(path)?,
        None => Config::load()?,
    };

    match cli.command {
        Some(Commands::Serve {
            host,
            port,
            data_dir,
            laya_url,
        }) => {
            if let Some(h) = host {
                config.host = h;
            }
            if let Some(p) = port {
                config.port = p;
            }
            if let Some(d) = data_dir {
                config.data_dir = d;
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
                "🕹️ Kwants 8-Bit AI Agent Platform listening at http://{}",
                addr
            );

            axum::serve(listener, app).await?;
        }
        None => {
            println!("🕹️ Kwants: 8-Bit AI Agent Platform in Rust");
            println!("Run `kwnt serve` to start the server, or `kwnt --help` for options.");
        }
    }

    Ok(())
}
