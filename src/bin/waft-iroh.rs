//! Command-line harness for the iroh transport spike.

#[cfg(feature = "iroh-spike")]
use clap::Parser;
#[cfg(feature = "iroh-spike")]
use std::path::PathBuf;

#[cfg(feature = "iroh-spike")]
#[derive(Parser)]
struct Cli {
    /// Wait for one incoming file.
    #[arg(long)]
    receive: Option<PathBuf>,
    /// Send this file to the endpoint address supplied with --peer.
    #[arg(long)]
    send: Option<PathBuf>,
    /// JSON endpoint address printed by another waft-iroh process.
    #[arg(long)]
    peer: Option<String>,
}

#[cfg(feature = "iroh-spike")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let endpoint = waft::iroh_transport::bind().await?;
    if let Some(downloads) = cli.receive {
        waft::iroh_transport::receive(&endpoint, &downloads).await?;
    } else if let Some(path) = cli.send {
        let peer = cli
            .peer
            .ok_or_else(|| anyhow::anyhow!("--peer is required with --send"))?;
        let address = serde_json::from_str(&peer)?;
        waft::iroh_transport::send(&endpoint, address, &path).await?;
    } else {
        anyhow::bail!("choose --receive or --send");
    }
    endpoint.close().await;
    Ok(())
}

#[cfg(not(feature = "iroh-spike"))]
fn main() {
    eprintln!("waft-iroh requires the `iroh-spike` feature");
    std::process::exit(2);
}
