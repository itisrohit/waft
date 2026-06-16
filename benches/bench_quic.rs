#![allow(clippy::unwrap_used)]

use criterion::{Criterion, criterion_group, criterion_main};
use std::fs;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use waft::identity::Identity;
use waft::quic_send::send_file_quic;
use waft::quic_transfer::start_quic_receiver;
use waft::trust::TrustStore;

fn bench_quic_file_transfer(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let size = 1024 * 1024; // 1MB
    let temp_dir = std::env::temp_dir();
    let src_path = temp_dir.join("waft_bench_quic_src");
    let dest_dir = temp_dir.join("waft_bench_quic_dest");

    fs::write(&src_path, vec![0u8; size]).unwrap();
    let _ = fs::remove_dir_all(&dest_dir);
    fs::create_dir_all(&dest_dir).unwrap();

    let trust_path = temp_dir.join("waft_bench_quic_trust");
    if trust_path.exists() {
        let _ = fs::remove_file(&trust_path);
    }
    let trust_store = Arc::new(TrustStore::load_or_create(&trust_path).unwrap());
    let sender_identity = Identity::generate();

    let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let socket = std::net::UdpSocket::bind(bind_addr).unwrap();
    let local_addr = socket.local_addr().unwrap();
    drop(socket);

    // Initialize crypto provider
    waft::quic_transport::init_crypto_provider();

    let trust = Arc::clone(&trust_store);
    let dest = dest_dir.clone();
    let receiver_handle = rt.spawn(async move {
        let _ = start_quic_receiver(local_addr, trust, dest).await;
    });

    // Wait for receiver to bind
    rt.block_on(async {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    });

    c.bench_function("transfer_quic_1mb", |b| {
        b.iter(|| {
            rt.block_on(async {
                send_file_quic(local_addr, &sender_identity, &src_path, None)
                    .await
                    .unwrap();
            });
        });
    });

    receiver_handle.abort();
    let _ = fs::remove_file(&src_path);
    let _ = fs::remove_dir_all(&dest_dir);
    let _ = fs::remove_file(&trust_path);
}

criterion_group!(benches, bench_quic_file_transfer);
criterion_main!(benches);
