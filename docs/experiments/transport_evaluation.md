# Transport Layer Evaluation: TCP Zero-Copy vs. QUIC vs. Custom UDP

During the v0.2 development cycle, we evaluated multiple transport layers to identify the optimal protocol for local area network (LAN) file transfers. The core objective was to achieve maximum throughput while providing resilience against transient network disconnections.

This document outlines the performance characteristics, engineering challenges, and final design decisions across the three protocols evaluated.

---

## 1. Baseline: TCP Zero-Copy

Our initial implementation relied on a kernel-tuned TCP protocol designed for local network performance.

### Design Features
*   **Zero-Copy I/O**: Native `sendfile(2)` system calls on macOS and Linux, transferring data directly from the OS page cache to the network socket without copying it into user-space.
*   **Memory-Mapped Files**: Using the `memmap2` crate to map files directly into memory, reducing memory copying overhead.
*   **Socket Tuning**: Disabling Nagle's algorithm (`TCP_NODELAY`) and allocating 4MB socket buffers to maximize bandwidth utilization.

### Performance Metrics
*   **1 KB Transfer Latency**: **25.54 ms**
*   **1 MB Transfer Latency**: **31.14 ms** (Throughput: **32.11 MB/s**)
*   **1 MB Criterion Iteration Time**: **1.56 ms**
*   **100 MB Connection Setup Time**: **10.12 ms**
*   **100 MB Total Transfer Time**: **220.00 ms** (Throughput: **446.67 MB/s**)

### Limitations
*   **Brittleness**: Standard TCP connections are tied to a specific IP and port pair. If a device changes network interfaces (e.g., switching from Wi-Fi to Ethernet) or experiences a temporary packet loss drop, the socket disconnects, requiring the transfer to be restarted from the beginning.

---

## 2. Evaluation of QUIC (Quinn)

To address connection instability, we prototyped an alternative transport using the `quinn` crate (a Rust implementation of QUIC).

### Rationale
QUIC offered session-based Connection IDs, allowing connection migration across IP changes, built-in TLS 1.3 encryption, and stream multiplexing to avoid Head-of-Line blocking.

### Performance Metrics
*   **1 MB Criterion Iteration Time**: **9.55 ms** (6.1x slower than TCP)
*   **100 MB Total Transfer Time**: **577.73 ms** (Throughput: **173.09 MB/s**)

### Engineering Bottlenecks
*   **User-Space Cryptography Overhead**: QUIC mandates TLS 1.3 bulk encryption. Performing symmetric decryption on high-throughput file streams in user-space created a CPU bottleneck, capping loopback throughput at **173.09 MB/s** (a 61% reduction compared to TCP Zero-Copy).
*   **System Call Frequency**: Operating in user-space meant QUIC could not take advantage of kernel-level `sendfile` optimizations, resulting in higher system call overhead for UDP socket operations.
*   **Binary Size and Dependencies**: The QUIC stack introduced heavy dependencies (`quinn`, `rustls`, `ring`), significantly increasing compile times and the final binary size.

---

## 3. Evaluation of Custom UDP (WUDP)

To eliminate the cryptography and dependency overhead of QUIC while retaining UDP's flexibility, we designed a custom protocol called **WUDP (Waft UDP Protocol)**.

### Rationale
WUDP implemented a lightweight, plaintext transport over raw UDP, using a 1-RTT handshake, a basic sliding-window algorithm, and direct writes to memory-mapped files out-of-order.

### Performance Metrics
*   **1 MB Criterion Iteration Time**: **8.26 ms** (5.3x slower than TCP)
*   **100 MB Total Transfer Time**: **426.80 ms** (Throughput: **234.30 MB/s**)

### Engineering Bottlenecks
*   **Flow & Congestion Control Complexity**: Bypassing kernel congestion algorithms meant reliability, packet reordering, flow control, and timeout retransmissions had to be implemented entirely in user-space.
*   **High Packet Drop Rates**: Without hardware-level socket offloading, high-speed UDP streams frequently saturated the receiver's socket buffer, leading to packet drops on loopback and requiring frequent retransmissions.
*   **Performance Ceiling**: WUDP achieved **234.30 MB/s** throughput. While faster than QUIC, it remained significantly slower than optimized TCP due to user-space protocol overhead.

---

## 4. Comprehensive Performance Comparison

Below is the consolidated matrix comparing all three implementations on identical hardware (macOS loopback):

| Metric / Transport Layer | TCP Zero-Copy (Final) | QUIC (Quinn / TLS 1.3) | WUDP (Custom Plaintext UDP) |
|:---|:---|:---|:---|
| **1 KB Transfer Latency** | 25.54 ms | — (Not measured) | — (Not measured) |
| **1 MB Transfer Latency** | 31.14 ms | — (Not measured) | — (Not measured) |
| **1 MB Criterion Iteration Time** | **1.56 ms** | 9.55 ms | 8.26 ms |
| **1 MB Measured Throughput** | **32.11 MB/s** | — (Not measured) | — (Not measured) |
| **100 MB Connection Setup Time** | **10.12 ms** | — (Not measured) | — (Not measured) |
| **100 MB Total Transfer Time** | **220.00 ms** | 577.73 ms | 426.80 ms |
| **100 MB Measured Throughput** | **446.67 MB/s** | 173.09 MB/s | 234.30 MB/s |
| **Connection Resiliency** | Resumable via Offset Handshake | Session Migration (CID-based) | Token-based migration |
| **Dependency Impact** | Minimal (Standard library + libc) | High (`quinn` + `rustls` + `ring`) | Medium (Custom memory-mapped I/O) |
| **Implementation Complexity** | Low | Medium | High |

---

## 5. Final Architecture: Resumable TCP Zero-Copy

Based on the evaluation metrics, we chose to maintain the performance of TCP Zero-Copy and resolve connection instability at the application layer.

### Resumable Design
1.  **Atomic Part-Files**: The receiver writes incoming data to a temporary `<BLAKE3_HASH>.part` file.
2.  **Negotiated Handshake**: During connection setup:
    *   If the complete file already exists and matches the BLAKE3 hash, the transfer is skipped (`0x02` Done).
    *   If a `.part` file is found, the receiver queries its size (`offset`) and replies with a `0x03` (Resume) ACK containing the `8-byte big-endian u64` offset.
    *   Otherwise, the transfer starts from `0` (`0x01` Accept).
3.  **Zero-Copy Offset Seeking**: The sender seeks the file pointer to the negotiated offset using `sendfile` arguments or memory-mapped slicing, streaming only the remaining bytes.
4.  **Post-Transfer Verification**: The receiver verifies the full file hash before renaming the `.part` file to the final destination.

This approach delivers the connection resiliency of UDP/QUIC while preserving the raw **446.67 MB/s** throughput of kernel-optimized TCP Zero-Copy. The experimental QUIC and WUDP modules have been completely removed from the codebase to keep it clean and maintainable.
