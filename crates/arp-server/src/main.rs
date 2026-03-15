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

    let log_level = if args.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level)),
        )
        .init();

    info!("ARP Server v{}", env!("CARGO_PKG_VERSION"));

    let config = ServerConfig::from_file(&args.config)?;
    config.validate()?;

    info!("Loading config from: {}", args.config);
    info!("Bind address: {}:{}", config.bind_addr, config.bind_port);

    let service = service::Service::new(config).await?;
    service.run().await?;

    Ok(())
}
