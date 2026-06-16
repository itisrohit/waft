#![allow(clippy::unwrap_used)]

use criterion::{Criterion, criterion_group, criterion_main};
use std::fs;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use waft::identity::Identity;
use waft::send::send_file;
use waft::transfer::start_receiver;
use waft::trust::TrustStore;

fn bench_file_transfer(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let size = 1024 * 1024; // 1MB
    let temp_dir = std::env::temp_dir();
    let src_path = temp_dir.join("waft_bench_src");
    let dest_dir = temp_dir.join("waft_bench_dest");

    fs::write(&src_path, vec![0u8; size]).unwrap();
    let _ = fs::remove_dir_all(&dest_dir);
    fs::create_dir_all(&dest_dir).unwrap();

    let trust_path = temp_dir.join("waft_bench_trust");
    if trust_path.exists() {
        let _ = fs::remove_file(&trust_path);
    }
    let trust_store = Arc::new(TrustStore::load_or_create(&trust_path).unwrap());
    let sender_identity = Identity::generate();

    let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let listener = rt
        .block_on(tokio::net::TcpListener::bind(bind_addr))
        .unwrap();
    let local_addr = listener.local_addr().unwrap();
    drop(listener);

    let trust = Arc::clone(&trust_store);
    let dest = dest_dir.clone();
    let receiver_handle = rt.spawn(async move {
        let _ = start_receiver(local_addr, trust, dest).await;
    });

    // Wait for receiver port to be active
    rt.block_on(async {
        for _ in 0..50 {
            if tokio::net::TcpStream::connect(local_addr).await.is_ok() {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        }
    });

    c.bench_function("transfer_1mb", |b| {
        b.iter(|| {
            rt.block_on(async {
                send_file(local_addr, &sender_identity, &src_path, None)
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

criterion_group!(benches, bench_file_transfer);
criterion_main!(benches);
