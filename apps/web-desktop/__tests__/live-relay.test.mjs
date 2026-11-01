import assert from "node:assert/strict";
import { once } from "node:events";
import { createServer } from "node:http";
import test from "node:test";
import { WebSocket, WebSocketServer } from "ws";
import { createLiveRelay } from "../live-relay.mjs";

async function fixture(t) {
  const upstream = new WebSocketServer({ host: "127.0.0.1", port: 0 });
  await once(upstream, "listening");
  const previous = process.env.ARGOS_RUST_API_ORIGIN;
  process.env.ARGOS_RUST_API_ORIGIN = `http://127.0.0.1:${upstream.address().port}`;
  const server = createServer((_request, response) => response.end("ok"));
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const port = server.address().port;
  const relay = createLiveRelay(port);
  server.on("upgrade", relay.upgrade);
  t.after(async () => {
    relay.close();
    for (const client of upstream.clients) client.terminate();
    await new Promise((resolve) => upstream.close(resolve));
    await new Promise((resolve) => server.close(resolve));
    if (previous === undefined) delete process.env.ARGOS_RUST_API_ORIGIN;
    else process.env.ARGOS_RUST_API_ORIGIN = previous;
  });
  return {
    upstream,
    port,
    url: `ws://127.0.0.1:${port}/api/v1/live`,
    origin: `http://127.0.0.1:${port}`,
  };
}

test("relays subscription frames and Rust samples without storing channel state", async (t) => {
  const f = await fixture(t);
  const command = JSON.stringify({
    type: "subscribe",
    metrics: true,
    brief: { run: "fixture-run", id: "fixture-article", watch: "generation-1" },
  });
  const sample = JSON.stringify({
    type: "metrics",
    data: {
      cpu_percent: 12,
      memory_percent: 34,
      disk_percent: null,
      network_kbps: 0,
    },
  });
  const received = new Promise((resolve) =>
    f.upstream.once("connection", (socket, request) => {
      assert.equal(request.url, "/api/v1/live");
      assert.equal(request.headers.origin, undefined);
      socket.once("message", (data) => {
        resolve(data.toString());
        socket.send(sample);
      });
    }),
  );
  const client = new WebSocket(f.url, { origin: f.origin });
  await once(client, "open");
  const reply = once(client, "message");
  client.send(command);
  assert.equal(await received, command);
  assert.equal((await reply)[0].toString(), sample);
  const closed = once(client, "close");
  client.close();
  await closed;
});

test("refuses foreign origins before creating a Rust connection", async (t) => {
  const f = await fixture(t);
  let connections = 0;
  f.upstream.on("connection", () => {
    connections++;
  });
  const client = new WebSocket(f.url, { origin: "https://untrusted.example" });
  client.on("error", () => {});
  const status = await new Promise((resolve) =>
    client.once("unexpected-response", (_request, response) => {
      response.resume();
      resolve(response.statusCode);
      client.terminate();
    }),
  );
  assert.equal(status, 403);
  assert.equal(connections, 0);
});

test("closes the browser stream when its Rust connection drops", async (t) => {
  const f = await fixture(t);
  f.upstream.once("connection", (socket) => socket.close());
  const client = new WebSocket(f.url, { origin: f.origin });
  const [code, reason] = await once(client, "close");
  assert.equal(code, 1012);
  assert.equal(reason.toString(), "Live connection interrupted");
});
