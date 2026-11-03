import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { createServer } from "node:http";
import { resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import test from "node:test";
import { WebSocket, WebSocketServer } from "ws";

test(
  "the real Next runtime relays live frames after REST routing and across a stateless restart",
  { timeout: 60_000 },
  async (t) => {
    const app = resolve(import.meta.dirname, "..");
    const backend = createServer((_request, response) => {
      response.setHeader("content-type", "application/json");
      response.end(
        JSON.stringify({
          ready: true,
          storage: true,
          service: "argos-intel",
          api_version: 1,
          instance_id: "offline-live-fixture",
        }),
      );
    });
    const rustSockets = new WebSocketServer({ noServer: true });
    const commands = [];
    backend.on("upgrade", (request, socket, head) =>
      rustSockets.handleUpgrade(request, socket, head, (client) => {
        client.on("message", (data) => {
          commands.push(data.toString());
          client.send(
            JSON.stringify({
              type: "metrics",
              data: {
                cpu_percent: commands.length,
                memory_percent: 20,
                disk_percent: null,
                network_kbps: 0,
              },
            }),
          );
        });
      }),
    );
    backend.listen(0, "127.0.0.1");
    await once(backend, "listening");
    const rustOrigin = `http://127.0.0.1:${backend.address().port}`;
    const reservation = createServer();
    reservation.listen(0, "127.0.0.1");
    await once(reservation, "listening");
    const port = reservation.address().port;
    await new Promise((done) => reservation.close(done));
    const origin = `http://127.0.0.1:${port}`;
    let child;
    let output = "";
    async function stop() {
      if (!child || child.exitCode !== null || child.signalCode) return;
      const current = child;
      const exited = once(current, "exit");
      current.kill("SIGTERM");
      const timeout = setTimeout(() => current.kill("SIGKILL"), 5000);
      const [code, signal] = await exited;
      clearTimeout(timeout);
      assert.equal(
        code,
        0,
        `Next failed to shut down cleanly (${signal}): ${output}`,
      );
    }
    t.after(async () => {
      try {
        await stop();
      } finally {
        for (const client of rustSockets.clients) client.terminate();
        rustSockets.close();
        await new Promise((done) => backend.close(done));
      }
    });
    async function start() {
      child = spawn(
        process.execPath,
        [resolve(app, "server.mjs"), "--port", String(port)],
        {
          cwd: app,
          env: {
            ...process.env,
            NODE_ENV: "production",
            ARGOS_RUST_API_ORIGIN: rustOrigin,
          },
          stdio: ["ignore", "pipe", "pipe"],
        },
      );
      for (const pipe of [child.stdout, child.stderr])
        pipe.on("data", (chunk) => {
          output = (output + chunk.toString()).slice(-32_000);
        });
      const deadline = Date.now() + 30_000;
      while (Date.now() < deadline) {
        assert.equal(child.exitCode, null, output);
        try {
          const response = await fetch(`${origin}/api/v1/health/ready`, {
            signal: AbortSignal.timeout(1000),
          });
          if (
            response.ok &&
            (await response.json()).instance_id === "offline-live-fixture"
          )
            return;
        } catch {
          /* The process has not bound its listener yet. */
        }
        await delay(100);
      }
      assert.fail(`Next did not become ready: ${output}`);
    }
    const command = JSON.stringify({
      type: "subscribe",
      metrics: true,
      brief: null,
    });
    async function connect() {
      const client = new WebSocket(
        `${origin.replace("http:", "ws:")}/api/v1/live`,
        { origin },
      );
      await once(client, "open");
      const frame = once(client, "message");
      client.send(command);
      const data = JSON.parse((await frame)[0].toString());
      return { client, data };
    }
    await start();
    const first = await connect();
    assert.equal(first.data.data.cpu_percent, 1);
    const closed = once(first.client, "close");
    await stop();
    await closed;
    await start();
    const second = await connect();
    assert.equal(second.data.data.cpu_percent, 2);
    assert.deepEqual(commands, [command, command]);
    const lastClosed = once(second.client, "close");
    second.client.close();
    await lastClosed;
  },
);
