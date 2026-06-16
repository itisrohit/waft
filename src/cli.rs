//! CLI command parsing and thin client logic.

use crate::daemon::DaemonCommand;
use anyhow::Result;
use std::path::Path;

#[cfg(unix)]
use crate::daemon::DaemonResponse;
#[cfg(unix)]
use anyhow::Context;
#[cfg(unix)]
use std::io::Write;
#[cfg(unix)]
use std::time::Duration;
#[cfg(unix)]
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
#[cfg(unix)]
use tokio::net::UnixStream;

const BOLD: &str = "\x1b[1m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const RESET: &str = "\x1b[0m";

/// Formats a byte size into a human-readable string.
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * 1024 * 1024;
    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(unix)]
/// Connects to the daemon Unix socket, auto-starting the daemon if not running.
async fn connect_to_daemon(socket_path: &Path) -> Result<UnixStream> {
    if !socket_path.exists() {
        start_daemon_process()?;
    }

    let mut retries = 10;
    loop {
        match UnixStream::connect(socket_path).await {
            Ok(stream) => return Ok(stream),
            Err(e) => {
                if retries == 0 {
                    return Err(e).context(format!(
                        "Could not connect to daemon socket at {socket_path:?}"
                    ));
                }
                if retries == 10 {
                    println!("{YELLOW}Daemon not running. Starting waft daemon...{RESET}");
                    start_daemon_process()?;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
                retries -= 1;
            }
        }
    }
}

#[cfg(unix)]
/// Spawns the daemon process detached.
fn start_daemon_process() -> Result<()> {
    let current_exe = std::env::current_exe().context("Failed to get current executable path")?;
    std::process::Command::new(current_exe)
        .arg("daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("Failed to spawn daemon process")?;
    Ok(())
}

#[cfg(unix)]
/// Runs the IPC client to send a command to the daemon and handle the response.
pub async fn run_client(socket_path: &Path, command: DaemonCommand) -> Result<()> {
    let mut stream = connect_to_daemon(socket_path).await?;
    let (reader, mut writer) = stream.split();
    let mut buf_reader = BufReader::new(reader);

    // Send command
    let serialized = serde_json::to_string(&command).context("Failed to serialize command")?;
    writer
        .write_all(format!("{serialized}\n").as_bytes())
        .await?;
    writer.flush().await?;

    // Read response(s)
    let mut line = String::new();
    loop {
        line.clear();
        if buf_reader.read_line(&mut line).await? == 0 {
            break;
        }

        let response: DaemonResponse =
            serde_json::from_str(&line).context("Failed to parse daemon response")?;

        match response {
            DaemonResponse::Ok(msg) => {
                println!("\n{GREEN}✅ Success: {msg}{RESET}");
                break;
            }
            DaemonResponse::Error(err) => {
                println!("\n{RED}❌ Error: {err}{RESET}");
                break;
            }
            DaemonResponse::PeerList(peers) => {
                println!("\n✨ {BOLD}Active LAN Peers:{RESET}");
                println!("{:-<72}", "");
                println!(
                    " {:<18} | {:<24} | {:<20}",
                    "NAME", "FINGERPRINT", "ADDRESS"
                );
                println!("{:-<72}", "");
                for peer in peers {
                    let fp_slice = if peer.fingerprint.len() > 24 {
                        &peer.fingerprint[..24]
                    } else {
                        &peer.fingerprint
                    };
                    println!(" {:<18} | {:<24} | {:<20}", peer.name, fp_slice, peer.addr);
                }
                println!("{:-<72}\n", "");
                break;
            }
            DaemonResponse::TrustList(trusts) => {
                println!("\n🛡️  {BOLD}Peer Trust Configurations:{RESET}");
                println!("{:-<72}", "");
                println!(" {:<44} | {:<20}", "FINGERPRINT", "TRUST TIER");
                println!("{:-<72}", "");
                for (fingerprint, tier) in trusts {
                    println!(" {fingerprint:<44} | {tier:?}");
                }
                println!("{:-<72}\n", "");
                break;
            }
            DaemonResponse::TrustStatus(tier) => {
                println!("Trust tier: {tier:?}");
                break;
            }
            DaemonResponse::Progress {
                bytes_sent,
                total_bytes,
            } => {
                let pct = if total_bytes > 0 {
                    (bytes_sent as f64 / total_bytes as f64) * 100.0
                } else {
                    0.0
                };
                let width = 30;
                let progress = if total_bytes > 0 {
                    ((bytes_sent as f64 / total_bytes as f64) * width as f64) as usize
                } else {
                    0
                };
                let bar: String = std::iter::repeat_n("=", progress)
                    .chain(std::iter::repeat_n(" ", width - progress))
                    .collect();
                print!(
                    "\r[{} {:.1}%] {} / {} bytes",
                    bar,
                    pct,
                    format_size(bytes_sent),
                    format_size(total_bytes)
                );
                let _ = std::io::stdout().flush();
            }
        }
    }

    Ok(())
}

#[cfg(not(unix))]
#[allow(clippy::unused_async)]
pub async fn run_client(_socket_path: &Path, _command: DaemonCommand) -> Result<()> {
    anyhow::bail!("CLI client mode is not supported on this platform.");
}
