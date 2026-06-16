# waft Benchmark Results

This document details the performance benchmarks of the current production-grade transport layer implementation: **Resumable TCP Zero-Copy**.

---

## 1. Loopback Performance Matrix

Measurements were taken on a macOS Sequoia system running loopback file transfers in release mode. Under the hood, this implementation uses native macOS `sendfile` zero-copy system calls combined with application-layer offset resumption.

| File Size | Latency / Setup Time | Total Transfer Time | Measured Throughput | Target Limit / Floor |
|---|---|---|---|---|
| **1 KB** | 25.54 ms | 25.54 ms | — | < 50 ms total |
| **1 MB** | 31.14 ms | 31.14 ms | 32.11 MB/s | < 80 ms total |
| **100 MB** | 10.12 ms | 220.00 ms | **446.67 MB/s** | < 1.2 s total |

*   **Criterion Benchmarks**: The Criterion loopback benchmark for 1MB transfers reported an average iteration time of **1.56 ms**.
*   **BLAKE3 Overhead**: Post-transfer BLAKE3 integrity checks take less than **5 ms** for 100MB, presenting negligible CPU overhead.

---

## 2. Key Performance Factors

Our Resumable TCP Zero-Copy transport achieves peak throughput on loopback and local networks due to three core design choices:

1.  **OS-Level Zero-Copy (`sendfile`/`splice`)**: Bypasses user-space read/write buffers. The kernel streams file data directly from the OS page cache to the TCP socket.
2.  **Socket Buffer Tuning**: Bypasses Nagle's delay (`TCP_NODELAY`) and configures send/recv socket buffers to **4MB** to fully saturate local Gigabit and Wi-Fi networks.
3.  **Stateless Resumption**: Uses a 1-RTT offset negotiation handshake. If a transfer is interrupted, it resumes from the last successfully written block stored in the receiver's temporary `.part` file, saving both time and bandwidth.
