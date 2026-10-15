mod client;
mod config;
mod idempotency;
#[cfg(feature = "kds")]
mod kds;
mod link;
mod printer;
mod raster_cache;
mod svg_raster;
mod test_print;
mod update;
mod usb;

use anyhow::Context;
use tracing::info;

/// The client's runtime. Built by hand rather than `#[tokio::main]` because
/// the kitchen display needs the main thread for Slint and runs this on a
/// background thread instead.
fn runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
}

fn main() -> anyhow::Result<()> {
    // Handle --version/--help BEFORE doing anything else. Without this the
    // flags were silently ignored and the binary booted the full client —
    // opening the printer device and running forever. A stray
    // `rusterando-printer --version` then wedged the single-open
    // /dev/usb/lp0 and blocked the real service from printing.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--test-print") {
        // One sample receipt, then exit (see test_print.rs).
        let path = args.get(1).map(String::as_str).unwrap_or("usb");
        return test_print::run(path).map_err(|e| anyhow::anyhow!("test print failed: {e:#}"));
    }
    for arg in &args {
        match arg.as_str() {
            "--version" | "-V" => {
                println!("rusterando-printer {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--help" | "-h" => {
                println!(
                    "rusterando-printer {}\n\nKitchen receipt-printer client. \
                     Configuration comes from env vars or $PRINTER_CONFIG \
                     (see config.rs). Flags: --version, --help, and \
                     --test-print [PATH] (one sample receipt to PATH, \
                     default `usb`, then exit).",
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
                .unwrap_or_else(|_| "rusterando_printer=info,kitchen_protocol=info".into()),
        )
        .init();

    let cfg = config::Config::from_env_or_file().context("load config")?;
    info!(
        server = %cfg.server_addr,
        shop = %cfg.shop_slug,
        printer = %cfg.printer_path.display(),
        state = %cfg.state_dir.display(),
        version = %cfg.version,
        display = cfg!(feature = "kds"),
        "pizzeria-printer starting",
    );

    #[cfg(feature = "kds")]
    return kds::run(cfg);

    #[cfg(not(feature = "kds"))]
    runtime()?.block_on(client::run(cfg, None))
}
