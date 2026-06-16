use criterion::{Criterion, criterion_group, criterion_main};

fn bench_file_transfer(c: &mut Criterion) {
    c.bench_function("transfer_1mb", |b| {
        b.iter(|| {
            // TODO: wire up send_file + start_receiver in loopback
        });
    });
}

criterion_group!(benches, bench_file_transfer);
criterion_main!(benches);
