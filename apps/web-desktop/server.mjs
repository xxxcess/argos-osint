import { createServer } from "node:http";
import { fileURLToPath } from "node:url";
import next from "next";
import { createLiveRelay } from "./live-relay.mjs";
import {
  configuredWebBasePath,
  configuredWebOrigin,
  rustUpstream,
} from "./server-origin.mjs";
import {
  CONNECT_RESPONSE_HEADERS,
  connectFailureResponse,
  isPublicWebPath,
  relativeWebPath,
  requireConnection,
  requireDevelopmentUpgrade,
  requireWebBoundary,
  safeReturnPath,
} from "./connect-session.mjs";

const dir = fileURLToPath(new URL(".", import.meta.url));
process.chdir(dir);
const portIndex = process.argv.indexOf("--port");
const port = Number(
  portIndex < 0 ? process.env.PORT || 3000 : process.argv[portIndex + 1],
);
if (!Number.isInteger(port) || port < 1 || port > 65535)
  throw new Error("Invalid port");
const bind = process.env.FLEETWORKS_WEB_BIND ?? "127.0.0.1";
const origin = configuredWebOrigin();
if (bind !== "127.0.0.1" && (bind !== "0.0.0.0" || !origin))
  throw new Error(
    "Fleetworks web binding requires an explicit protected public origin",
  );
if (process.env.FLEETWORKS_API_ORIGIN !== undefined) await rustUpstream();
const dev = process.argv.includes("--dev");
const server = createServer();
const app = next({
  dev,
  dir,
  hostname: bind,
  port,
  webpack: true,
  turbopack: false,
  httpServer: server,
});
await app.prepare();
const handle = app.getRequestHandler();
const relay = createLiveRelay(port);
const frameworkUpgrades = new Set();
function requestUrl(raw) {
  try {
    return new URL(raw || "/", "http://request.invalid");
  } catch {
    return undefined;
  }
}
const upgrade = async (request, socket, head) => {
  const url = requestUrl(request.url);
  if (!url) {
    socket.end("HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n");
    return;
  }
  const path = relativeWebPath(url.pathname);
  if (path === "/api/v1/live") relay.upgrade(request, socket, head);
  else {
    try {
      await requireDevelopmentUpgrade(request, dev, path);
      for (const handler of frameworkUpgrades)
        handler.call(server, request, socket, head);
    } catch {
      socket.end("HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n");
    }
  }
};
server.on("upgrade", upgrade);
server.on("request", async (request, response) => {
  const url = requestUrl(request.url);
  if (!url) {
    response.writeHead(400).end("Invalid request");
    return;
  }
  // Browser context headers are untrusted. Only the relay creates backend proof.
  for (const name of Object.keys(request.headers)) {
    if (
      name === "x-fleetworks-session" ||
      name.startsWith("x-fleetworks-context-") ||
      name.startsWith("x-fleetworks-origin-") ||
      name.startsWith("x-fleetworks-request-")
    )
      delete request.headers[name];
  }
  try {
    await requireWebBoundary(request);
    if (!isPublicWebPath(url.pathname))
      await requireConnection(request, {
        mutation: !["GET", "HEAD"].includes(request.method || "GET"),
      });
  } catch (error) {
    if (response.destroyed || response.writableEnded) return;
    const failure = connectFailureResponse(error);
    for (const [name, value] of failure.headers)
      response.setHeader(name, value);
    const page =
      !relativeWebPath(url.pathname)?.startsWith("/api/") &&
      ["GET", "HEAD"].includes(request.method || "GET");
    if (page && failure.status === 401) {
      response
        .writeHead(307, {
          location: `${configuredWebBasePath()}/connect?return=${encodeURIComponent(safeReturnPath(request.url))}`,
        })
        .end();
    } else {
      response.writeHead(failure.status).end(await failure.text());
    }
    return;
  }
  if (response.destroyed || response.writableEnded) return;
  for (const [name, value] of Object.entries(CONNECT_RESPONSE_HEADERS))
    response.setHeader(name, value);
  const result = handle(request, response);
  // Next 16 installs its upgrade listener on its first HTTP request. Route
  // framework upgrades separately so its API catch-all cannot consume /live.
  for (const listener of server.listeners("upgrade")) {
    if (listener !== upgrade) {
      frameworkUpgrades.add(listener);
      server.removeListener("upgrade", listener);
    }
  }
  result.catch(() => {
    if (response.destroyed || response.writableEnded) return;
    if (!response.headersSent) response.writeHead(500);
    response.end();
  });
});
await new Promise((resolve, reject) => {
  server.once("error", reject);
  server.listen(port, bind, resolve);
});
console.log(
  `Argos Next server ready at ${origin ?? `http://127.0.0.1:${port}`}`,
);
let closing = false;
async function shutdown() {
  if (closing) return;
  closing = true;
  relay.close();
  server.close();
  server.closeAllConnections();
  await app.close();
}
for (const signal of ["SIGINT", "SIGTERM"])
  process.on(signal, () => {
    void shutdown();
  });
