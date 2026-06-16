use std::fs;
use waft::cli::run_client;
use waft::daemon::{DaemonCommand, start_daemon};
use waft::trust::TrustTier;

#[tokio::test]
async fn test_daemon_cli_ipc() -> Result<(), anyhow::Error> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)?
        .as_micros();
    let base_dir = std::env::temp_dir().join(format!("waft_test_daemon_{ts}"));
    let socket_path = base_dir.join("daemon.sock");

    // Ensure directory is clean
    let _ = fs::remove_dir_all(&base_dir);
    fs::create_dir_all(&base_dir)?;

    let base_dir_clone = base_dir.clone();
    let daemon_handle = tokio::spawn(async move {
        let _ = start_daemon(&base_dir_clone).await;
    });

    // Wait for daemon to create socket and start listening
    let mut retries = 20;
    while !socket_path.exists() && retries > 0 {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        retries -= 1;
    }
    assert!(socket_path.exists(), "Daemon did not start Unix socket");

    // 1. Test List peers command
    let res = run_client(&socket_path, DaemonCommand::ListPeers).await;
    assert!(res.is_ok(), "ListPeers failed: {res:?}");

    // 2. Test GetTrust command
    let res = run_client(
        &socket_path,
        DaemonCommand::GetTrust {
            fingerprint: "test_fingerprint".to_string(),
        },
    )
    .await;
    assert!(res.is_ok(), "GetTrust failed: {res:?}");

    // 3. Test SetTrust command
    let res = run_client(
        &socket_path,
        DaemonCommand::SetTrust {
            fingerprint: "test_fingerprint".to_string(),
            tier: TrustTier::Trusted,
        },
    )
    .await;
    assert!(res.is_ok(), "SetTrust failed: {res:?}");

    // 4. Test ListTrust command
    let res = run_client(&socket_path, DaemonCommand::ListTrust).await;
    assert!(res.is_ok(), "ListTrust failed: {res:?}");

    // Clean up
    daemon_handle.abort();
    let _ = fs::remove_dir_all(&base_dir);

    Ok(())
}
