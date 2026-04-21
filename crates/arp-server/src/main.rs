mod audit;
mod control;
mod metrics;
mod nathole;
mod proxy;
mod resource;
mod service;
mod web;

use clap::Parser;
use tracing::info;

use arp_common::config::ServerConfig;
use arp_common::logging::{init_logging, LogConfig};
use arp_common::Result;

#[derive(Parser)]
#[command(name = "arps")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "ARP Server - Fast Reverse Proxy Server", long_about = None)]
struct Args {
    #[arg(short, long, default_value = "server.toml")]
    config: String,

    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Load config first so we can read log_file / log_max_days.
    let config = ServerConfig::from_file(&args.config)?;
    config.validate()?;

    let log_level = if args.verbose {
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

    info!("ARP Server v{}", env!("CARGO_PKG_VERSION"));
    info!("Loading config from: {}", args.config);
    info!("Bind address: {}:{}", config.bind_addr, config.bind_port);

    let service = service::Service::new(config).await?;
    let service_for_shutdown = service.clone();

    tokio::select! {
        res = service.run() => { res? }
        _ = tokio::signal::ctrl_c() => {
            info!("Received shutdown signal, draining connections...");
            service_for_shutdown.graceful_shutdown().await;
        }
    }

    Ok(())
}
