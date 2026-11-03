import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import {
  cp,
  mkdir,
  mkdtemp,
  readFile,
  symlink,
  writeFile,
} from "node:fs/promises";
import { createServer } from "node:http";
import { resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import test from "node:test";
import { chromium, expect } from "@playwright/test";
import { WebSocket, WebSocketServer } from "ws";

test(
  "custom development runtime preserves HMR alongside the stateless live relay",
  { timeout: 120_000 },
  async (t) => {
    const app = resolve(import.meta.dirname, "..");
    const root = resolve(app, "../..");
    const scratch = resolve(root, ".agent-scratch/topology-reference");
    await mkdir(scratch, { recursive: true });
    const fixture = await mkdtemp(resolve(scratch, "live-dev-"));
    for (const name of [
      "server.mjs",
      "server-origin.mjs",
      "live-relay.mjs",
      "next.config.mjs",
      "proxy.ts",
      "package.json",
      "babel.config.cjs",
    ]) {
      try {
        await cp(resolve(app, name), resolve(fixture, name));
      } catch (error) {
        if (name !== "babel.config.cjs" || error.code !== "ENOENT") throw error;
      }
    }
    await cp(resolve(app, "app"), resolve(fixture, "app"), { recursive: true });
    await symlink(
      resolve(app, "node_modules"),
      resolve(fixture, "node_modules"),
      "dir",
    );
    const config = JSON.parse(
      await readFile(resolve(app, "tsconfig.json"), "utf8"),
    );
    config.extends = resolve(root, "tsconfig.json");
    config.include = [
      "next-env.d.ts",
      "**/*.ts",
      "**/*.tsx",
      resolve(root, "packages/**/*.d.ts"),
    ];
    await writeFile(resolve(fixture, "tsconfig.json"), JSON.stringify(config));
    // This isolated smoke does not download fonts. Production visual fixtures
    // cover the real layout; the same strict compiler and RN components run here.
    const layoutPath = resolve(fixture, "app/layout.tsx");
    let layout = await readFile(layoutPath, "utf8");
    layout = layout.replace(
      /import \{ Inter, JetBrains_Mono \} from "next\/font\/google";\n/,
      "",
    );
    layout =
      layout.slice(0, layout.indexOf("const inter =")) +
      layout.slice(layout.indexOf("export const metadata"));
    layout = layout.replace(
      "className={`${inter.variable} ${mono.variable}`}",
      'className=""',
    );
    await writeFile(layoutPath, layout);
    const marker = resolve(fixture, "app/home/page.tsx");
    const pageSource = (text) =>
      `export default function Page() { return <div data-testid="hmr-marker">${text}</div>; }`;
    await writeFile(marker, pageSource("Before HMR"));

    const backend = createServer((_request, response) => response.end("{}"));
    const upstream = new WebSocketServer({ noServer: true });
    backend.on("upgrade", (request, socket, head) =>
      upstream.handleUpgrade(request, socket, head, (client) => {
        client.on("message", () =>
          client.send(
            JSON.stringify({
              type: "metrics",
              data: {
                cpu_percent: 7,
                memory_percent: 20,
                disk_percent: null,
                network_kbps: 0,
              },
            }),
          ),
        );
      }),
    );
    backend.listen(0, "127.0.0.1");
    await once(backend, "listening");
    const reservation = createServer();
    reservation.listen(0, "127.0.0.1");
    await once(reservation, "listening");
    const port = reservation.address().port;
    await new Promise((done) => reservation.close(done));
    const origin = `http://127.0.0.1:${port}`;
    let output = "";
    const child = spawn(
      process.execPath,
      [resolve(fixture, "server.mjs"), "--dev", "--port", String(port)],
      {
        cwd: fixture,
        env: {
          ...process.env,
          NODE_ENV: "development",
          ARGOS_RUST_API_ORIGIN: `http://127.0.0.1:${backend.address().port}`,
        },
        stdio: ["ignore", "pipe", "pipe"],
      },
    );
    for (const pipe of [child.stdout, child.stderr])
      pipe.on("data", (chunk) => {
        output = (output + chunk.toString()).slice(-64_000);
      });
    let browser;
    t.after(async () => {
      await writeFile(resolve(fixture, "runtime.log"), output);
      await browser?.close();
      if (child.exitCode === null && !child.signalCode) {
        const exited = once(child, "exit");
        child.kill("SIGTERM");
        const timeout = setTimeout(() => child.kill("SIGKILL"), 5000);
        await exited;
        clearTimeout(timeout);
      }
      for (const client of upstream.clients) client.terminate();
      upstream.close();
      await new Promise((done) => backend.close(done));
    });
    const deadline = Date.now() + 30_000;
    while (
      !output.includes("Argos Next server ready") &&
      Date.now() < deadline
    ) {
      assert.equal(child.exitCode, null, output);
      await delay(100);
    }
    assert.ok(output.includes("Argos Next server ready"), output);
    browser = await chromium.launch({ headless: true });
    const page = await browser.newPage();
    const errors = [];
    const sockets = [];
    let documents = 0;
    page.on("pageerror", (error) => errors.push(error.message));
    page.on("websocket", (socket) => sockets.push(socket.url()));
    page.on("request", (request) => {
      if (
        request.isNavigationRequest() &&
        request.resourceType() === "document"
      )
        documents++;
    });
    await page.route("**/api/v1/**", (route) =>
      route.fulfill({ status: 503, json: { error: "Offline fixture" } }),
    );
    await page.goto(`${origin}/home`, { timeout: 60_000 });
    await expect(page.getByTestId("hmr-marker")).toHaveText("Before HMR");
    await expect(
      page.getByRole("button", {
        name: "Retry connection checks",
        exact: true,
      }),
    ).toBeEnabled();
    await expect
      .poll(() => sockets.some((url) => url.includes("/_next/")))
      .toBe(true);
    await writeFile(marker, pageSource("After HMR"));
    await expect(page.getByTestId("hmr-marker")).toHaveText("After HMR", {
      timeout: 30_000,
    });
    assert.equal(documents, 1, "HMR must update without a document reload");
    const socket = new WebSocket(
      `${origin.replace("http:", "ws:")}/api/v1/live`,
      { origin },
    );
    await once(socket, "open");
    const frame = once(socket, "message");
    socket.send(
      JSON.stringify({ type: "subscribe", metrics: true, brief: null }),
    );
    assert.equal(JSON.parse((await frame)[0].toString()).data.cpu_percent, 7);
    const closed = once(socket, "close");
    socket.close();
    await closed;
    assert.deepEqual(errors, []);
  },
);
