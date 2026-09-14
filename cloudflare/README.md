# Cloudflare signaling deployment

This Worker is the recommended hosted signaling service for waft. It uses one
Durable Object per room, forwards SDP negotiation messages and optional iroh
endpoint metadata, and never receives file bytes.

Install Wrangler, authenticate with Cloudflare, then deploy:

```sh
npm install -g wrangler
wrangler login
cd cloudflare
wrangler deploy
```

The resulting Worker URL is configured as waft's default rendezvous URL. For a
custom deployment, override it on clients along with a private room:

```sh
export WAFT_SIGNALING_URL=wss://waft-signaling.example.workers.dev
export WAFT_SIGNALING_ROOM='use-a-long-random-secret'
export WAFT_ICE_SERVERS='stun:stun.example.net:3478'
```

The Rust client adds the room to the WebSocket URL automatically. Deploy a
TURN server separately if direct WebRTC connectivity fails for some users. The
same rendezvous endpoint can also exchange iroh endpoint addresses when clients
use the `iroh-internet` feature.
