mod client;
mod config;
mod idempotency;
mod printer;

use anyhow::Context;
use tracing::info;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    // Handle --version/--help BEFORE doing anything else. Without this the
    // flags were silently ignored and the binary booted the full client —
    // opening the printer device and running forever. A stray
    // `rusterando-printer --version` then wedged the single-open
    // /dev/usb/lp0 and blocked the real service from printing.
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--version" | "-V" => {
                println!("rusterando-printer {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--help" | "-h" => {
                println!(
                    "rusterando-printer {}\n\nKitchen receipt-printer client. \
                     Configuration comes from env vars or $PRINTER_CONFIG \
                     (see config.rs); there are no runtime flags besides \
                     --version and --help.",
                    env!("CARGO_PKG_VERSION")
                );
                return Ok(());
            }
            _ => {}
        }
    }

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
