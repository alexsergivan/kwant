use clap::{Parser, Subcommand};
use kwnt::{config::Config, db::Database};
use std::path::PathBuf;

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

    /// Check everything kwant needs here (Laya, browser, AI CLIs, data folder, access) and print
    /// the command that fixes each missing part on this platform. Exits 1 if a required part is missing
    Doctor,

    /// Install what kwant needs on this Mac or Linux machine: Laya, a browser and an AI CLI
    /// (Claude Code unless you pick with --clis). Asks before each part; never runs sudo (it
    /// prints those commands for you). Safe to run again
    Setup {
        /// Install without asking
        #[arg(long, short = 'y')]
        yes: bool,
        /// Only these parts: laya, browser, claude, codex, grok, docker (comma list)
        #[arg(long)]
        only: Option<String>,
        /// AI CLIs to install: claude, codex, grok (comma list; default: claude if none is installed)
        #[arg(long)]
        clis: Option<String>,
        /// Also install Docker, for VM mode (optional; never needed locally)
        #[arg(long)]
        docker: bool,
        /// Show what would be installed; install nothing
        #[arg(long)]
        dry_run: bool,
    },

    /// List kwants stored in the local database
    Agents,

    /// List teams stored in the local database
    Teams,

    /// List channels stored in the local database
    Channels,

    /// Print a new one-time sign-in link for another device, like `kwnt signin-link` (`--raw`: just
    /// the API token, for scripts). Only reads the existing token; never creates one
    Token {
        /// Print only the token, for `Authorization: Bearer` in scripts
        #[arg(long)]
        raw: bool,
    },

    /// Print a new one-time sign-in link and QR code for another device (works once, for 15
    /// minutes). Asks the running server, so run it where `kwnt serve` runs (Docker:
    /// `docker compose exec kwnt kwnt signin-link`)
    SigninLink {
        /// kwnt's address as browsers open it (e.g. https://kwant.example.com); defaults to the
        /// first KWNT_ALLOWED_ORIGINS entry
        #[arg(long)]
        url: Option<String>,
        /// Leave out the QR code
        #[arg(long)]
        no_qr: bool,
    },

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

            kwnt::serve::run(config, shutdown_signal())
                .await
                .map_err(|e| e as Box<dyn std::error::Error>)?;
        }
        Some(Commands::Status) => print_status(&config),
        Some(Commands::Doctor) => {
            let platform = kwnt::doctor::Platform::detect();
            let checks = kwnt::doctor::checks(&kwnt::doctor::gather(&config).await, platform);
            print!("{}", kwnt::doctor::render(&checks, platform));
            if checks.iter().any(|c| c.status == kwnt::doctor::Status::Fail) {
                std::process::exit(1);
            }
        }
        Some(Commands::Setup { yes, only, clis, docker, dry_run }) => {
            let parse = |v: Option<String>| -> Result<Option<Vec<kwnt::setup::ItemId>>, String> {
                v.map(|v| kwnt::setup::parse_list(&v)).transpose()
            };
            let (only, clis) = match (parse(only), parse(clis)) {
                (Ok(o), Ok(c)) => (o, c),
                (Err(e), _) | (_, Err(e)) => {
                    eprintln!("kwnt setup: {e}");
                    std::process::exit(2);
                }
            };
            let opts = kwnt::setup::Options { yes, only, clis, docker, dry_run };
            std::process::exit(kwnt::setup::run_cli(&config, opts).await);
        }
        Some(Commands::Agents) => print_agents(&config).await?,
        Some(Commands::Teams) => print_teams(&config).await?,
        Some(Commands::Channels) => print_channels(&config).await?,
        Some(Commands::Token { raw }) => {
            // Read only: a token created here (wrong user, wrong data dir) would be rejected.
            let token = match kwnt::server::auth::env_token()
                .map(Ok)
                .unwrap_or_else(|| kwnt::server::auth::read_existing(&config.data_dir))
            {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("kwnt token: {e}");
                    eprintln!("kwnt token only reads the token of a kwant install; it never makes one.");
                    eprintln!("  - Run it as the user that runs `kwnt serve`, with the same KWNT_DATA_DIR");
                    eprintln!("    (systemd setup from the README: sudo cat /var/lib/kwnt/api-token).");
                    eprintln!("  - Docker: docker compose exec kwnt kwnt token --raw");
                    eprintln!("  - If KWNT_API_TOKEN is set for the server, that value is the token.");
                    eprintln!("  - Never started yet? `kwnt serve` creates the token on first run.");
                    std::process::exit(1);
                }
            };
            if raw {
                println!("{token}");
            } else {
                // A one-time link, never the token itself: the token would make a link that works
                // forever for whoever sees this output.
                let where_token = if kwnt::server::auth::env_token().is_some() {
                    "the KWNT_API_TOKEN value".to_string()
                } else {
                    config.data_dir.join("api-token").display().to_string()
                };
                if let Err(e) = kwnt::server::signin::cli_link(&config, None, true).await {
                    eprintln!("kwnt token: {e}");
                    eprintln!("The API token still works on the sign-in screen: it's {where_token} (`kwnt token --raw` prints it).");
                    std::process::exit(1);
                }
                println!("   Prefer typing the token? It's {where_token} (`kwnt token --raw` prints it).");
            }
        }
        Some(Commands::SigninLink { url, no_qr }) => {
            if let Err(e) = kwnt::server::signin::cli_link(&config, url.as_deref(), !no_qr).await {
                eprintln!("kwnt signin-link: {e}");
                std::process::exit(1);
            }
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
