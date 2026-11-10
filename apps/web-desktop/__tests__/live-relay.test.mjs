import assert from "node:assert/strict";
import { once } from "node:events";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { WebSocket, WebSocketServer } from "ws";
import { createLiveRelay } from "../live-relay.mjs";

async function fixture(t, token) {
  const upstream = new WebSocketServer({ host: "127.0.0.1", port: 0 });
  await once(upstream, "listening");
  const environment = new Map(
    [
      "ARGOS_RUST_API_ORIGIN",
      "FLEETWORKS_API_ORIGIN",
      "FLEETWORKS_API_TOKEN",
      "FLEETWORKS_API_TOKEN_FILE",
      "FLEETWORKS_WEB_ORIGIN",
    ].map((key) => [key, process.env[key]]),
  );
  for (const key of environment.keys()) delete process.env[key];
  process.env.ARGOS_RUST_API_ORIGIN = `http://127.0.0.1:${upstream.address().port}`;
  const server = createServer((_request, response) => response.end("ok"));
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const port = server.address().port;
  let origin = `http://127.0.0.1:${port}`;
  if (token) {
    origin = "http://localhost:9000";
    const directory = await mkdtemp(join(tmpdir(), "fleetworks-web-"));
    t.after(() => rm(directory, { recursive: true, force: true }));
    const tokenFile = join(directory, "token");
    await writeFile(tokenFile, `${token}\r\n`, { mode: 0o600 });
    process.env.FLEETWORKS_API_ORIGIN = process.env.ARGOS_RUST_API_ORIGIN;
    process.env.FLEETWORKS_API_TOKEN_FILE = tokenFile;
    process.env.FLEETWORKS_WEB_ORIGIN = origin;
  }
  const relay = createLiveRelay(port);
  server.on("upgrade", relay.upgrade);
  t.after(async () => {
    relay.close();
    for (const client of upstream.clients) client.terminate();
    await new Promise((resolve) => upstream.close(resolve));
    await new Promise((resolve) => server.close(resolve));
    for (const [key, value] of environment) {
      if (value === undefined) delete process.env[key];
      else process.env[key] = value;
    }
  });
  return {
    upstream,
    port,
    url: `ws://127.0.0.1:${port}/api/v1/live`,
    origin,
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

test("adds the server bearer to the upstream upgrade behind a remapped port", async (t) => {
  const token = "fleetworks-fixture-token-123456789012";
  const f = await fixture(t, token);
  const connected = once(f.upstream, "connection");
  const client = new WebSocket(f.url, {
    origin: f.origin,
    headers: {
      host: new URL(f.origin).host,
      authorization: "Bearer browser-controlled",
      cookie: "private=browser-cookie",
    },
  });
  t.after(() => client.terminate());
  const [, request] = await connected;
  assert.equal(request.headers.host, `127.0.0.1:${f.upstream.address().port}`);
  assert.equal(request.headers.authorization, `Bearer ${token}`);
  assert.equal(request.headers.origin, undefined);
  assert.equal(request.headers.cookie, undefined);
  const closed = once(client, "close");
  client.close();
  await closed;
});
