# Optional cross-network transfers

waft is LAN-first. On the same network it uses UDP discovery and the existing
authenticated TCP transfer, with no server dependency. Cross-network support
is opt-in behind the `internet` Cargo feature; the `iroh-internet` feature
enables the preferred cross-network transport.

The default rendezvous deployment uses Cloudflare Durable Objects. A small
self-hosted Rust server is also included for users who want full control. The
internet path has three pieces:

1. A WebSocket rendezvous service introduces peers in a shared room and forwards
   SDP negotiation messages or iroh endpoint metadata. The Cloudflare deployment
   is in `cloudflare/`.
2. The daemon keeps LAN as the first route when local discovery finds the peer.
3. iroh performs authenticated QUIC connectivity with direct and relay paths
   for discovered cross-network peers.
4. WebRTC performs ICE using `WAFT_ICE_SERVERS` as the compatibility fallback.
   The rendezvous server never carries file bytes.

Enable it with a feature build and configure:

```sh
export WAFT_SIGNALING_ROOM='long-random-room-secret'
export WAFT_ICE_SERVERS='stun:stun.example.net:3478,turn:username:password@turn.example.net:3478'
```

The deployed waft rendezvous URL is built in, so `WAFT_SIGNALING_URL` is only
needed to override it for self-hosting or tests. A room is still required so a
public signaling endpoint cannot expose every connected peer to every other
user. Use TLS (`wss://`) in production and restrict the signaling server at
the reverse proxy if it is not intended for public access.

For local testing, the room and optional URL can also be passed as daemon
flags, avoiding shell environment setup:

```sh
cargo run --features iroh-internet --bin waft -- \
  --signaling-room 'long-random-room-secret' \
  daemon
```

Use `--signaling-url wss://your-worker.example.net` only when overriding the
built-in hosted deployment.

## Two-computer test

On both computers, start the daemon with the same room:

```sh
cargo run --features iroh-internet --bin waft -- \
  --signaling-room 'long-random-room-secret' daemon
```

Confirm the peer appears, then send a small file:

```sh
cargo run --features iroh-internet --bin waft -- list
cargo run --features iroh-internet --bin waft -- \
  send '<peer-name>' ./test.txt
```

Use separate networks to exercise iroh. Use the same Wi-Fi to exercise LAN;
the daemon automatically prefers the local TCP address when available. A
separate signaling host is only needed when overriding the built-in deployment.

This fallback is deliberately optional: LAN traffic keeps the lower-latency
path, and disabling internet features removes cross-network dependencies from
the normal binary.
