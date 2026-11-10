import { WebSocket, WebSocketServer } from "ws";
import {
  directAgent,
  incomingWebOrigin,
  rustUpstream,
} from "./server-origin.mjs";

/** @tag:api-proxy — Connection-local frame forwarding; no subscriptions or application cache. */
export function createLiveRelay(port) {
  const clients = new WebSocketServer({
    noServer: true,
    maxPayload: 16 * 1024,
    perMessageDeflate: false,
  });
  let closing = false;
  async function upgrade(request, socket, head) {
    try {
      const origin = incomingWebOrigin(
        request.headers.host || "",
        "http:",
        port,
      );
      if (request.headers.origin !== origin || request.url !== "/api/v1/live")
        throw new Error("Origin refused");
    } catch {
      socket.end("HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n");
      return;
    }
    let target;
    let headers;
    const onSocketError = () => socket.destroy();
    socket.once("error", onSocketError);
    try {
      const upstream = await rustUpstream();
      target = new URL("/api/v1/live", upstream.origin);
      target.protocol = target.protocol === "https:" ? "wss:" : "ws:";
      if (upstream.authorization)
        headers = { host: target.host, authorization: upstream.authorization };
    } catch {
      socket.end(
        "HTTP/1.1 503 Service Unavailable\r\nConnection: close\r\n\r\n",
      );
      return;
    }
    socket.off("error", onSocketError);
    if (closing || socket.destroyed) {
      socket.destroy();
      return;
    }
    clients.handleUpgrade(request, socket, head, (browser) => {
      const upstream = new WebSocket(target, {
        maxPayload: 64 * 1024,
        perMessageDeflate: false,
        handshakeTimeout: 10_000,
        followRedirects: false,
        ...(headers ? { headers, agent: directAgent(target) } : {}),
      });
      const pending = [];
      let pendingBytes = 0;
      let alive = true;
      const stop = () => {
        clearInterval(heartbeat);
        if (upstream.readyState === WebSocket.CONNECTING) upstream.terminate();
        else if (upstream.readyState === WebSocket.OPEN) upstream.close();
        if (browser.readyState === WebSocket.OPEN)
          browser.close(1012, "Live connection interrupted");
      };
      const forward = (destination, data, binary) => {
        if (destination.readyState !== WebSocket.OPEN) return;
        if (destination.bufferedAmount > 256 * 1024) {
          stop();
          return;
        }
        destination.send(data, { binary });
      };
      const heartbeat = setInterval(() => {
        if (!alive) {
          browser.terminate();
          stop();
          return;
        }
        alive = false;
        if (browser.readyState === WebSocket.OPEN) browser.ping();
      }, 20_000);
      browser.on("pong", () => {
        alive = true;
      });
      browser.on("message", (data, binary) => {
        if (upstream.readyState === WebSocket.CONNECTING) {
          pendingBytes += data.length;
          if (pendingBytes > 16 * 1024) {
            stop();
            return;
          }
          pending.push([data, binary]);
        } else forward(upstream, data, binary);
      });
      upstream.on("open", () => {
        for (const [data, binary] of pending) forward(upstream, data, binary);
        pending.length = 0;
      });
      upstream.on("message", (data, binary) => forward(browser, data, binary));
      upstream.on("error", stop);
      upstream.on("close", stop);
      browser.on("error", stop);
      browser.on("close", stop);
    });
  }
  return {
    upgrade,
    close() {
      closing = true;
      for (const client of clients.clients) client.terminate();
      clients.close();
    },
  };
}
