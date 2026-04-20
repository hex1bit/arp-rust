mod control;
mod proxy;
mod service;

use clap::Parser;
use tracing::info;

use arp_common::config::ClientConfig;
use arp_common::logging::{init_logging, LogConfig};
use arp_common::Result;

#[derive(Parser)]
#[command(name = "arpc")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "ARP Client - Fast Reverse Proxy Client", long_about = None)]
struct Args {
    #[arg(short, long, default_value = "client.toml")]
    config: String,

    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Load config first so we can read log_file / log_max_days.
    let config = ClientConfig::from_file(&args.config)?;
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

    info!("ARP Client v{}", env!("CARGO_PKG_VERSION"));
    info!("Loading config from: {}", args.config);
    info!(
        "Server address: {}:{}",
        config.server_addr, config.server_port
    );
    info!("Proxies: {}", config.proxies.len());

    let service = service::Service::new(config).await?;
    service.run().await?;

    Ok(())
}
