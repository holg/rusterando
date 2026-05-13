mod client;
mod config;
mod idempotency;
mod printer;

use anyhow::Context;
use tracing::info;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "pizzeria_printer=info,kitchen_protocol=info".into()),
        )
        .init();

    let cfg = config::Config::from_env_or_file().context("load config")?;
    info!(
        server = %cfg.server_addr,
        shop = %cfg.shop_slug,
        printer = %cfg.printer_path.display(),
        state = %cfg.state_dir.display(),
        version = %cfg.version,
        "pizzeria-printer starting",
    );

    client::run(cfg).await
}
