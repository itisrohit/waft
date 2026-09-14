/** Cloudflare Durable Object signaling service for waft. */

interface Peer { id: string; name: string; fingerprint: string; iroh_endpoint?: string; }
interface SignalMessage {
  type: string; room?: string; peer?: Peer; from?: string; to?: string;
  sdp?: string; iroh_endpoint?: string; id?: string; message?: string;
}

const MAX_MESSAGE_BYTES = 256 * 1024;

export class WaftRoom {
  constructor(private readonly state: DurableObjectState) {}

  async fetch(request: Request): Promise<Response> {
    if (request.headers.get("Upgrade")?.toLowerCase() !== "websocket") {
      return new Response("WebSocket endpoint", { status: 426 });
    }
    const pair = new WebSocketPair();
    this.state.acceptWebSocket(pair[1]);
    return new Response(null, { status: 101, webSocket: pair[0] });
  }

  webSocketMessage(socket: WebSocket, raw: string | ArrayBuffer): void {
    if (typeof raw !== "string" || raw.length > MAX_MESSAGE_BYTES) {
      socket.close(1009, "message too large");
      return;
    }
    let message: SignalMessage;
    try { message = JSON.parse(raw) as SignalMessage; }
    catch { socket.close(1003, "invalid JSON"); return; }

    if (message.type === "register" && message.peer) {
      if (!message.room || !validPeer(message.peer)) {
        socket.close(1008, "invalid registration");
        return;
      }
      socket.serializeAttachment(message.peer);
      const peers = this.state.getWebSockets()
        .filter((candidate) => candidate !== socket)
        .map((candidate) => candidate.deserializeAttachment() as Peer | null)
        .filter((peer): peer is Peer => peer !== null);
      send(socket, { type: "welcome", peers });
      this.broadcast(socket, { type: "peer_joined", peer: message.peer });
      return;
    }

    if ((message.type === "offer" || message.type === "answer") && message.to) {
      const target = this.state.getWebSockets().find((candidate) => {
        const peer = candidate.deserializeAttachment() as Peer | null;
        return peer?.id === message.to;
      });
      if (target) send(target, message);
      else send(socket, { type: "error", message: "target peer is offline" });
    }
  }

  webSocketClose(socket: WebSocket): void {
    const peer = socket.deserializeAttachment() as Peer | null;
    if (peer) this.broadcast(socket, { type: "peer_left", id: peer.id });
  }

  webSocketError(socket: WebSocket): void { this.webSocketClose(socket); }

  private broadcast(sender: WebSocket, message: SignalMessage): void {
    for (const socket of this.state.getWebSockets()) {
      if (socket !== sender) send(socket, message);
    }
  }
}

function validPeer(peer: Peer): boolean {
  return typeof peer.id === "string" && peer.id.length > 0
    && typeof peer.name === "string" && peer.name.length > 0 && peer.name.length <= 63
    && typeof peer.fingerprint === "string" && /^[0-9a-f]{64}$/.test(peer.fingerprint)
    && (peer.iroh_endpoint === undefined
      || (typeof peer.iroh_endpoint === "string" && peer.iroh_endpoint.length <= 16384));
}

function send(socket: WebSocket, message: SignalMessage): void {
  try { socket.send(JSON.stringify(message)); }
  catch { socket.close(1011, "send failed"); }
}

export default {
  async fetch(request: Request, env: { WAFT_ROOMS: DurableObjectNamespace }): Promise<Response> {
    const url = new URL(request.url);
    const encodedRoom = url.searchParams.get("room");
    if (!encodedRoom || encodedRoom.length > 256) return new Response("missing room", { status: 400 });
    let room: string;
    try {
      const normalized = encodedRoom.replace(/-/g, "+").replace(/_/g, "/");
      const padded = normalized + "=".repeat((4 - normalized.length % 4) % 4);
      room = new TextDecoder().decode(Uint8Array.from(atob(padded), (char) => char.charCodeAt(0)));
    } catch { return new Response("invalid room", { status: 400 }); }
    if (room.length === 0 || room.length > 128) return new Response("invalid room", { status: 400 });
    return env.WAFT_ROOMS.get(env.WAFT_ROOMS.idFromName(room)).fetch(request);
  },
};
