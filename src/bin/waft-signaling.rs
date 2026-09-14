//! Minimal self-hosted WebSocket signaling server for waft's optional remote path.

#[cfg(feature = "internet")]
use std::net::SocketAddr;

#[cfg(feature = "internet")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let bind = std::env::var("WAFT_SIGNALING_BIND")
        .unwrap_or_else(|_| "0.0.0.0:8787".to_string())
        .parse::<SocketAddr>()?;
    waft::remote::run_signaling_server(bind).await
}

#[cfg(not(feature = "internet"))]
fn main() {
    eprintln!("waft-signaling requires the `internet` feature");
    std::process::exit(2);
}
