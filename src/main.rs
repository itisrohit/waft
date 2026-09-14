//! Command-line interface and daemon dispatcher.

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use waft::cli::run_client;
use waft::daemon::{DaemonCommand, ipc_endpoint, start_daemon};
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use waft::startup;
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
    /// Manage the waft daemon
    Daemon {
        #[command(subcommand)]
        action: Option<DaemonAction>,
    },
    /// Send a file to a peer
    Send {
        /// The peer's name or public key fingerprint
        peer: String,
        /// Path to the file to transfer
        file: String,
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

#[derive(Subcommand)]
enum DaemonAction {
    /// Install and start the daemon at login
    Install,
    /// Remove the daemon login agent
    Uninstall,
}

fn init_logging() {
    let env_filter = std::env::var("WAFT_LOG").unwrap_or_else(|_| "info".to_string());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .try_init();
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let cli = Cli::parse();

    // Determine waft base directory path
    let base_dir = cli.dir.unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(home).join(".waft")
    });
    let socket_path = ipc_endpoint(&base_dir);

    match cli.command {
        Commands::Daemon { action } => {
            #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
            if let Some(action) = action {
                match action {
                    DaemonAction::Install => startup::install(&base_dir)?,
                    DaemonAction::Uninstall => startup::uninstall()?,
                }
                return Ok(());
            }
            #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
            if action.is_some() {
                anyhow::bail!(
                    "Daemon login management is currently supported on Linux, macOS, and Windows."
                );
            }
            init_logging();
            start_daemon(&base_dir).await?;
        }
        Commands::Send { peer, file } => {
            let file_path = PathBuf::from(file);
            // Convert to absolute path if possible
            let abs_path = std::fs::canonicalize(&file_path).unwrap_or(file_path);
            let cmd = DaemonCommand::SendFile {
                peer,
                file_path: abs_path.to_string_lossy().to_string(),
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
