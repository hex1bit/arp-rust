mod admin;
mod cli;
mod control;
mod proxy;
mod service;

use clap::{Parser, Subcommand};
use tracing::info;

use arp_common::config::ClientConfig;
use arp_common::logging::{init_logging, LogConfig};
use arp_common::Result;

#[derive(Parser)]
#[command(name = "arpc")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "ARP Client - Fast Reverse Proxy Client", long_about = None)]
struct Cli {
    #[arg(short, long, default_value = "client.toml")]
    config: String,

    #[arg(short, long)]
    verbose: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the client (default if no subcommand)
    Run,
    /// Show proxy status
    Status,
    /// Diagnose proxy connectivity
    Check {
        /// Proxy name to check (all if omitted)
        name: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let config = ClientConfig::from_file(&cli.config)?;
    config.validate()?;

    match cli.command {
        Some(Commands::Status) => {
            cli::run_status(&config).await;
            return Ok(());
        }
        Some(Commands::Check { name }) => {
            cli::run_check(&config, name).await;
            return Ok(());
        }
        _ => {} // Run (default)
    }

    let log_level = if cli.verbose {
        "debug"
    } else if config.log_level.is_empty() {
        "info"
    } else {
        &config.log_level
    };

    let _log_guard = init_logging(LogConfig {
        log_level,
        log_file: &config.log_file,
        log_max_days: config.log_max_days,
    });

    info!("ARP Client v{}", env!("CARGO_PKG_VERSION"));
    info!("Loading config from: {}", cli.config);
    info!(
        "Server address: {}:{}",
        config.server_addr, config.server_port
    );
    info!("Proxies: {}", config.proxies.len());

    let service = service::Service::new(config, cli.config).await?;

    tokio::select! {
        res = service.run() => { res? }
        _ = tokio::signal::ctrl_c() => {
            info!("Received shutdown signal, exiting...");
        }
    }

    Ok(())
}
