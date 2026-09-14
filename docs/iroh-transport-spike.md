# iroh transport

This covers the iroh transport and its daemon integration. It does not replace
the LAN transport: when a peer is on the local network, the daemon continues
to use the LAN path first.

With the `iroh-internet` feature, the daemon can also announce its iroh
endpoint address through the existing rendezvous connection. This removes
manual endpoint exchange and lets the daemon select iroh for peers discovered
through the internet rendezvous.

To enable that announcement on a daemon, pass the same private room on both
devices and build with the combined feature:

```sh
cargo build --features iroh-internet
cargo run --features iroh-internet --bin waft -- \
  --signaling-room 'long-random-room-secret' daemon
```

When both daemons are running, `waft list` includes discovered remote peers
whose entries contain an `iroh:` address. `waft send` uses LAN when available,
then authenticated iroh QUIC for a peer with an iroh address, and retains the
existing WebRTC path as fallback.

iroh gives each endpoint a public-key identity and uses QUIC connectivity
attempts that can become direct when possible, with relay fallback when NATs
prevent a direct path. The relay carries encrypted traffic but is not the
application's file protocol. The spike uses one bidirectional QUIC stream and
the existing file-name/size framing, so it does not add a blob store or a
server-side file cache.

## Two-computer daemon test

On either computer, list peers and send a file by the displayed name:

```sh
cargo run --features iroh-internet --bin waft -- list
cargo run --features iroh-internet --bin waft -- send '<peer-name>' ./test.txt
```

For an internet-path test, use two different networks, such as home Wi-Fi and
a phone hotspot. For a LAN-path test, put both computers on the same Wi-Fi and
confirm the peer has a local address in `waft list`; the daemon will prefer
that route automatically.

## Low-level harness smoke test

Build and run the receiver on computer B:

```sh
cargo run --features iroh-spike --bin waft-iroh -- \
  --receive "$HOME/Downloads/waft-iroh-test"
```

Copy the complete `WAFT_IROH_ENDPOINT=...` value printed by computer B. On
computer A, send a small file using that JSON endpoint address:

```sh
cargo run --features iroh-spike --bin waft-iroh -- \
  --send ./test.txt \
  --peer '<paste-the-complete-endpoint-json-here>'
```

The receiver exits after one file. Run the commands on separate computers and
separate networks to exercise iroh's cross-network path. The endpoint may use
a direct connection or a public development relay depending on the networks.
No account or paid service is required for this smoke test.

## Scope and follow-up

The low-level command remains a validation harness. The daemon path adds the
persisted identity, trust store, atomic temporary files, and BLAKE3 verification,
but does not yet provide resume support over iroh. Public relays are suitable
for development/testing; production deployment should evaluate relay
operations separately.
