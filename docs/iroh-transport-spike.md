# iroh transport spike

This is an isolated experiment for cross-network file transfer. It does not
replace the LAN daemon or the existing optional internet transport.

With the `iroh-internet` feature, the daemon can also announce its iroh
endpoint address through the existing rendezvous connection. This removes the
manual endpoint exchange for discovery experiments; it does not yet switch the
daemon's file-send command to iroh.

To enable that announcement on a daemon, pass the same private room on both
devices and build with the combined feature:

```sh
cargo build --features iroh-internet
cargo run --features iroh-internet --bin waft -- \
  --signaling-room 'long-random-room-secret' daemon
```

When both daemons are running, `waft list` includes discovered remote peers
whose entries contain an `iroh:` address. WebRTC remains available through the
existing `internet` feature.

iroh gives each endpoint a public-key identity and uses QUIC connectivity
attempts that can become direct when possible, with relay fallback when NATs
prevent a direct path. The relay carries encrypted traffic but is not the
application's file protocol. The spike uses one bidirectional QUIC stream and
the existing file-name/size framing, so it does not add a blob store or a
server-side file cache.

## Two-computer smoke test

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

The command is intentionally a validation harness, not a production daemon
integration. It currently accepts one incoming stream and does not yet connect
waft's persisted identity, trust tiers, discovery UI, resume protocol, or
automatic peer selection to iroh. Before making it the default internet path,
we need a two-network test matrix, explicit receiver authorization, persisted
endpoint identity, and a decision on relay operations. Public relays are
appropriate for development/testing; production deployment should evaluate a
dedicated relay or another operational policy.
