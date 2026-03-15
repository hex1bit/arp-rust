mod control;
mod proxy;
mod service;

use clap::Parser;
use tracing::info;

use arp_common::config::ClientConfig;
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

    let log_level = if args.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level)),
        )
        .init();

    info!("ARP Client v{}", env!("CARGO_PKG_VERSION"));

    let config = ClientConfig::from_file(&args.config)?;
    config.validate()?;

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
