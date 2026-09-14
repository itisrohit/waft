# Optional cross-network transfers

waft is LAN-first. On the same network it uses UDP discovery and the existing
authenticated TCP transfer, with no server dependency. Cross-network support
is opt-in behind the `internet` Cargo feature.

The internet path has three pieces:

1. A self-hosted WebSocket signaling server introduces peers in a shared room
   and forwards only SDP negotiation messages.
2. WebRTC performs ICE using `WAFT_ICE_SERVERS`, trying direct/STUN paths first
   and TURN when configured.
3. File data is sent over the authenticated WebRTC data channel; the signaling
   server is not a relay.

Enable it with a feature build and configure:

```sh
export WAFT_SIGNALING_URL=wss://waft.example.net/ws
export WAFT_SIGNALING_ROOM='long-random-room-secret'
export WAFT_ICE_SERVERS='stun:stun.example.net:3478,turn:username:password@turn.example.net:3478'
```

If the signaling variables are absent, the daemon remains LAN-only. A room is
required so a public signaling endpoint cannot expose every connected peer to
every other user. Use TLS (`wss://`) in production and restrict the signaling
server at the reverse proxy if it is not intended for public access.

## Two-computer test

Run the signaling binary on a reachable host, set the same room and signaling
URL on both computers, then start the waft daemons. Confirm the two peers are
visible before sending a small test file. For a first test, configure a STUN
server that both networks can reach. If the ICE state cannot become connected
through NAT, add a TURN server and repeat.

This fallback is deliberately optional: LAN traffic keeps the lower-latency
path, and disabling the feature removes the WebRTC dependency from the normal
binary.
