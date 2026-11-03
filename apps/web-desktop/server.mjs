import { createServer } from "node:http";
import { fileURLToPath } from "node:url";
import next from "next";
import { createLiveRelay } from "./live-relay.mjs";

const dir = fileURLToPath(new URL(".", import.meta.url));
process.chdir(dir);
const portIndex = process.argv.indexOf("--port");
const port = Number(
  portIndex < 0 ? process.env.PORT || 3000 : process.argv[portIndex + 1],
);
if (!Number.isInteger(port) || port < 1 || port > 65535)
  throw new Error("Invalid port");
const dev = process.argv.includes("--dev");
const server = createServer();
const app = next({
  dev,
  dir,
  hostname: "127.0.0.1",
  port,
  webpack: true,
  turbopack: false,
  httpServer: server,
});
await app.prepare();
const handle = app.getRequestHandler();
const relay = createLiveRelay(port);
const frameworkUpgrades = new Set();
const upgrade = (request, socket, head) => {
  if (request.url?.split("?")[0] === "/api/v1/live")
    relay.upgrade(request, socket, head);
  else
    for (const handler of frameworkUpgrades)
      handler.call(server, request, socket, head);
};
server.on("upgrade", upgrade);
server.on("request", (request, response) => {
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
  server.listen(port, "127.0.0.1", resolve);
});
console.log(`Argos Next server ready at http://127.0.0.1:${port}`);
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
