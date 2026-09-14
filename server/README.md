# waft signaling server

This is an optional, self-hosted WebSocket rendezvous service. It forwards
small SDP messages between peers in the same room and never handles file
contents. Clients can exchange WebRTC negotiation messages or iroh endpoint
metadata through the room; file data remains peer-to-peer.

Run it locally:

```sh
cargo run --features internet --bin waft-signaling
```

For deployment, put it behind a TLS reverse proxy and expose the resulting
`wss://` URL to clients. The server itself listens on `0.0.0.0:8787` by
default; override it with `WAFT_SIGNALING_BIND`.

Each client must set the same high-entropy `WAFT_SIGNALING_ROOM` value. Rooms
are isolated in memory and disappear when their last client disconnects.

Example client environment:

```sh
export WAFT_SIGNALING_URL=wss://waft.example.net/ws
export WAFT_SIGNALING_ROOM='replace-with-a-long-random-secret'
export WAFT_ICE_SERVERS='stun:stun.example.net:3478,turn:turn.example.net:3478'
```

STUN is sufficient when WebRTC NAT traversal allows a direct UDP path. TURN is
needed for networks that block or do not permit a direct WebRTC path; it should
be supplied as a credentialed URL and hosted separately from this signaling
service. iroh can use its direct or relay connectivity independently.
