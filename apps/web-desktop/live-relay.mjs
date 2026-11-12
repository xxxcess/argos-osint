import { WebSocket, WebSocketServer } from "ws";
import {
  directAgent,
  incomingWebOrigin,
  rustUpstream,
} from "./server-origin.mjs";
import {
  ConnectFailure,
  connectionProofHeaders,
  relativeWebPath,
  requireConnection,
  connectAuthority,
} from "./connect-session.mjs";

/** @tag:api-proxy — Connection-local frame forwarding; no subscriptions or application cache. */
export function createLiveRelay(port) {
  const clients = new WebSocketServer({
    noServer: true,
    maxPayload: 16 * 1024,
    perMessageDeflate: false,
  });
  let closing = false;
  async function upgrade(request, socket, head) {
    const onSocketError = () => socket.destroy();
    socket.once("error", onSocketError);
    let session;
    try {
      const origin = incomingWebOrigin(
        request.headers.host || "",
        "http:",
        port,
      );
      const url = new URL(request.url || "/", origin);
      if (
        request.headers.origin !== origin ||
        relativeWebPath(url.pathname) !== "/api/v1/live" ||
        [...url.searchParams.keys()].some(
          (key) => key !== "context_revision",
        ) ||
        url.searchParams.getAll("context_revision").length !== 1
      )
        throw new Error("Origin refused");
      const rawRevision = url.searchParams.get("context_revision");
      const expectedRevision = Number(rawRevision);
      if (
        !rawRevision ||
        !/^[1-9][0-9]{0,15}$/u.test(rawRevision) ||
        !Number.isSafeInteger(expectedRevision)
      )
        throw new ConnectFailure(409, "context_required");
      session = await requireConnection(request, { expectedRevision });
    } catch (error) {
      const status = error instanceof ConnectFailure ? error.status : 403;
      const reason =
        {
          401: "Unauthorized",
          403: "Forbidden",
          409: "Conflict",
          429: "Too Many Requests",
          503: "Service Unavailable",
        }[status] || "Forbidden";
      socket.end(
        `HTTP/1.1 ${status} ${reason}\r\nCache-Control: private, no-store\r\nConnection: close\r\n\r\n`,
      );
      return;
    }
    let target;
    let headers;
    try {
      const upstream = await rustUpstream();
      target = new URL("/api/v1/live", upstream.origin);
      target.protocol = target.protocol === "https:" ? "wss:" : "ws:";
      if (!upstream.authorization) throw new Error("Missing authority");
      headers = {
        host: target.host,
        authorization: upstream.authorization,
        ...connectionProofHeaders(session),
      };
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
      let stopped = false;
      let checkingAuthority = false;
      let lastVerified = Date.now();
      const expires = Date.parse(session.expires_at);
      const stop = (code = 1012, reason = "Live connection interrupted") => {
        if (stopped) return;
        stopped = true;
        clearInterval(heartbeat);
        clearInterval(authorityCheck);
        clearTimeout(expiry);
        if (upstream.readyState === WebSocket.CONNECTING) upstream.terminate();
        else if (upstream.readyState === WebSocket.OPEN) upstream.close();
        if (browser.readyState === WebSocket.OPEN) browser.close(code, reason);
      };
      const forward = (destination, data, binary) => {
        if (stopped) return;
        if (Date.now() >= expires || Date.now() - lastVerified >= 10_000) {
          stop(1008, "Connection context expired; reconnect");
          return;
        }
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
      // One bounded authority request per live connection, independent of frames.
      const authorityCheck = setInterval(async () => {
        if (checkingAuthority || stopped) return;
        checkingAuthority = true;
        try {
          const current = await connectAuthority.resolve(
            session.token,
            session.context.context_revision,
          );
          if (
            current.context.deployment_id !== session.context.deployment_id ||
            current.context.registration_id !==
              session.context.registration_id ||
            current.context.access_epoch !== session.context.access_epoch ||
            current.context.scope_revision !== session.context.scope_revision
          )
            throw new Error("Context changed");
          lastVerified = Date.now();
        } catch {
          stop(1008, "Connection context expired; reconnect");
        } finally {
          checkingAuthority = false;
        }
      }, 5000);
      const expiry = setTimeout(
        () => stop(1008, "Connection context expired; reconnect"),
        Math.max(0, Math.min(2_147_483_647, expires - Date.now())),
      );
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
      upstream.on("error", () => stop());
      upstream.on("close", () => stop());
      browser.on("error", () => stop());
      browser.on("close", () => stop());
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
