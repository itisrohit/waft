//! Command-line interface and daemon dispatcher.

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use waft::cli::run_client;
use waft::daemon::{DaemonCommand, start_daemon};
use waft::trust::TrustTier;

#[derive(Parser)]
#[command(name = "waft")]
#[command(version = "0.1.0")]
#[command(about = "Cross-platform file transfer and clipboard sync daemon", long_about = None)]
struct Cli {
    /// Custom path for the waft base directory (defaults to ~/.waft)
    #[arg(long, global = true)]
    dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the waft daemon in the foreground
    Daemon,
    /// Send a file to a peer
    Send {
        /// The peer's name or public key fingerprint
        peer: String,
        /// Path to the file to transfer
        file: String,
        /// Use QUIC transport instead of TCP
        #[arg(long)]
        quic: bool,
    },
    /// List active discovered peers on the LAN
    List,
    /// View or configure peer trust tiers
    Trust {
        /// Peer fingerprint
        fingerprint: Option<String>,
        /// Set trust tier (blocked, ask, trusted, own)
        #[arg(long)]
        set: Option<String>,
    },
}

fn init_logging() {
    let env_filter = std::env::var("WAFT_LOG").unwrap_or_else(|_| "info".to_string());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .try_init();
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    waft::quic_transport::init_crypto_provider();
    let cli = Cli::parse();

    // Determine waft base directory path
    let base_dir = cli.dir.unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(home).join(".waft")
    });
    let socket_path = base_dir.join("daemon.sock");

    match cli.command {
        Commands::Daemon => {
            init_logging();
            start_daemon(&base_dir).await?;
        }
        Commands::Send { peer, file, quic } => {
            let file_path = PathBuf::from(file);
            // Convert to absolute path if possible
            let abs_path = std::fs::canonicalize(&file_path).unwrap_or(file_path);
            let cmd = DaemonCommand::SendFile {
                peer,
                file_path: abs_path.to_string_lossy().to_string(),
                quic,
            };
            run_client(&socket_path, cmd).await?;
        }
        Commands::List => {
            run_client(&socket_path, DaemonCommand::ListPeers).await?;
        }
        Commands::Trust { fingerprint, set } => {
            let cmd = match (fingerprint, set) {
                (None, None) => DaemonCommand::ListTrust,
                (Some(fp), None) => DaemonCommand::GetTrust { fingerprint: fp },
                (Some(fp), Some(tier_str)) => {
                    let tier = match tier_str.to_lowercase().as_str() {
                        "blocked" => TrustTier::Blocked,
                        "ask" => TrustTier::Ask,
                        "trusted" => TrustTier::Trusted,
                        "own" => TrustTier::Own,
                        _ => {
                            eprintln!(
                                "Invalid trust tier. Choose from: blocked, ask, trusted, own"
                            );
                            std::process::exit(1);
                        }
                    };
                    DaemonCommand::SetTrust {
                        fingerprint: fp,
                        tier,
                    }
                }
                (None, Some(_)) => {
                    eprintln!("Must specify a peer fingerprint when setting a trust tier.");
                    std::process::exit(1);
                }
            };
            run_client(&socket_path, cmd).await?;
        }
    }

    Ok(())
}
