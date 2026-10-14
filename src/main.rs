use clap::{Parser, Subcommand};
use kwnt::config::Config;
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
        Some(Commands::Serve { host, port, data_dir, laya_url }) => {
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

            tracing::info!(
                host = %config.host,
                port = config.port,
                data_dir = %config.data_dir.display(),
                laya_url = %config.laya_url,
                "Starting Kwants engine..."
            );
            println!(
                "🕹️ Kwants 8-Bit AI Agent Platform listening at http://{}:{}",
                config.host, config.port
            );
        }
        None => {
            println!("🕹️ Kwants: 8-Bit AI Agent Platform in Rust");
            println!("Run `kwnt serve` to start the server, or `kwnt --help` for options.");
        }
    }

    Ok(())
}
