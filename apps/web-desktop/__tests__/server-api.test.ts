// @vitest-environment node
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { http, passthrough } from "msw";
import { once } from "node:events";
import { createServer, type IncomingHttpHeaders } from "node:http";
import { mswServer } from "../../../vitest.setup";
import { proxyRust } from "../app/server-api";

// Transport-only fixtures inject a previously verified context. The separate
// server-api-auth suite exercises real cookies, registry calls and denial.
vi.mock("../connect-session.mjs", async (importOriginal) => {
  const actual =
    await importOriginal<typeof import("../connect-session.mjs")>();
  return {
    ...actual,
    requireConnection: vi.fn(async () => ({
      token: "a".repeat(64),
      context: { context_revision: 1 },
      expires_at: "2099-01-01T00:00:00Z",
      origin: "http://127.0.0.1:3000",
    })),
  };
});

const next = "http://127.0.0.1:3000";
const rust = "http://127.0.0.1:9188";
beforeEach(() => {
  vi.stubEnv("FLEETWORKS_API_ORIGIN", rust);
  vi.stubEnv("FLEETWORKS_API_TOKEN", "fleetworks-fixture-token-123456789012");
  vi.stubEnv("FLEETWORKS_WEB_ORIGIN", undefined);
});
afterEach(() => vi.unstubAllEnvs());

type CapturedRequest = {
  url: URL;
  method: string;
  headers: IncomingHttpHeaders;
  body: string;
};

/** Exercise the production node:http stream adapter with IncomingMessage. */
async function withUpstream(
  body: unknown,
  check: (fixture: {
    origin: string;
    received: CapturedRequest[];
  }) => Promise<void>,
) {
  const received: CapturedRequest[] = [];
  const failures: unknown[] = [];
  const upstream = createServer(async (request, reply) => {
    try {
      const chunks: Buffer[] = [];
      let size = 0;
      for await (const chunk of request) {
        const bytes = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
        size += bytes.length;
        if (size > 16 * 1024) throw new Error("Fixture request exceeds bound");
        chunks.push(bytes);
      }
      received.push({
        url: new URL(request.url || "/", `http://${request.headers.host}`),
        method: request.method || "GET",
        headers: request.headers,
        body: Buffer.concat(chunks).toString("utf8"),
      });
      reply.writeHead(200, { "content-type": "application/json" });
      reply.end(JSON.stringify(body));
    } catch (error) {
      failures.push(error);
      reply.writeHead(500).end();
    }
  });
  upstream.listen(0, "127.0.0.1");
  await once(upstream, "listening");
  try {
    const address = upstream.address();
    if (!address || typeof address === "string")
      throw new Error("Missing fixture port");
    const origin = `http://127.0.0.1:${address.port}`;
    vi.stubEnv("FLEETWORKS_API_ORIGIN", origin);
    mswServer.use(http.all(`${origin}/*`, () => passthrough()));
    await check({ origin, received });
    expect(failures).toEqual([]);
  } finally {
    upstream.closeAllConnections();
    await new Promise<void>((resolve, reject) =>
      upstream.close((error) => (error ? reject(error) : resolve())),
    );
  }
}

describe("Next same-origin API → Rust transport", () => {
  it("keeps classifier setup local to Venture", async () => {
    for (const [method, endpoint] of [
      ["GET", "classifier/status"],
      ["POST", "classifier/train"],
      ["GET", "classifier/jobs/detail"],
      ["POST", "classifier/cancel"],
    ]) {
      const response = await proxyRust(
        new Request(`${next}/api/v1/${endpoint}`, { method }),
        endpoint.split("/"),
      );
      expect(response.status).toBe(404);
    }
  });
  it("uses only the server origin and forwards opaque queries/status without browser headers", async () => {
    await withUpstream(
      { rows: [], next_cursor: "opaque:2" },
      async ({ origin, received }) => {
        // The retired variable cannot override the configured server destination.
        vi.stubEnv("ARGOS_RUST_API_ORIGIN", "http://retired.example.test:9188");
        const response = await proxyRust(
          new Request(`${next}/api/v1/articles?cursor=a%2Bb%2F%3D`, {
            headers: { origin: next, cookie: "private=x" },
          }),
          ["articles"],
        );
        expect(response.status).toBe(200);
        expect(received).toHaveLength(1);
        expect(received[0].url.origin).toBe(origin);
        expect(received[0].url.pathname).toBe("/api/v1/articles");
        expect(received[0].url.searchParams.get("cursor")).toBe("a+b/=");
        expect(received[0].headers.origin).toBeUndefined();
        expect(received[0].headers.cookie).toBeUndefined();
        expect(await response.json()).toEqual({
          rows: [],
          next_cursor: "opaque:2",
        });
        expect(response.headers.get("cache-control")).toBe(
          "private, no-store, max-age=0",
        );
      },
    );
  });

  it("injects only the server bearer and refuses upstream redirects", async () => {
    const token = "fleetworks-fixture-token-123456789012";
    const received: { path: string; headers: IncomingHttpHeaders }[] = [];
    const upstream = createServer((request, reply) => {
      received.push({ path: request.url || "", headers: request.headers });
      if (request.url?.startsWith("/api/v1/days")) {
        reply.writeHead(302, { location: "/must-not-be-requested" }).end();
        return;
      }
      reply.writeHead(200, { "content-type": "application/json" });
      reply.end(JSON.stringify({ ready: true }));
    });
    upstream.listen(0, "127.0.0.1");
    await once(upstream, "listening");
    try {
      const address = upstream.address();
      if (!address || typeof address === "string")
        throw new Error("Missing fixture port");
      const backend = `http://127.0.0.1:${address.port}`;
      const publicOrigin = "http://localhost:9000";
      vi.stubEnv("FLEETWORKS_API_ORIGIN", backend);
      vi.stubEnv("FLEETWORKS_API_TOKEN", token);
      vi.stubEnv("FLEETWORKS_WEB_ORIGIN", publicOrigin);
      mswServer.use(http.all(`${backend}/*`, () => passthrough()));
      const make = (path: string, origin = publicOrigin) =>
        new Request(`http://127.0.0.1:3000/api/v1/${path}`, {
          headers: {
            host: "localhost:9000",
            origin,
            cookie: "private=browser-cookie",
            authorization: "Bearer browser-controlled",
            "proxy-authorization": "Basic browser-controlled",
          },
        });
      const response = await proxyRust(make("health/ready"), [
        "health",
        "ready",
      ]);
      expect(response.status).toBe(200);
      expect(await response.json()).toEqual({ ready: true });
      expect(response.headers.get("authorization")).toBeNull();
      expect(received[0]).toMatchObject({
        path: "/api/v1/health/ready",
        headers: {
          host: `127.0.0.1:${address.port}`,
          authorization: `Bearer ${token}`,
        },
      });
      for (const key of ["origin", "cookie", "proxy-authorization"])
        expect(received[0].headers[key]).toBeUndefined();
      expect((await proxyRust(make("days", next), ["days"])).status).toBe(403);
      const redirect = await proxyRust(make("days"), ["days"]);
      expect(redirect.status).toBe(503);
      expect(await redirect.text()).not.toContain(token);
      expect(received.map(({ path }) => path)).toEqual([
        "/api/v1/health/ready",
        "/api/v1/days",
      ]);
    } finally {
      upstream.closeAllConnections();
      await new Promise<void>((resolve) => upstream.close(() => resolve()));
    }
  });

  it("forwards JSON mutations to the fixed Rust destination", async () => {
    await withUpstream(
      { nodes: [{ kind: "inference" }] },
      async ({ origin, received }) => {
        const response = await proxyRust(
          new Request(`${next}/api/v1/topology/resolve`, {
            method: "POST",
            headers: { origin: next, "content-type": "application/json" },
            body: JSON.stringify({ seed: { Inference: "fixture-inference" } }),
          }),
          ["topology", "resolve"],
        );
        expect(await response.json()).toEqual({
          nodes: [{ kind: "inference" }],
        });
        expect(response.status).toBe(200);
        expect(received).toHaveLength(1);
        expect(received[0].url.origin).toBe(origin);
        expect(received[0].url.pathname).toBe("/api/v1/topology/resolve");
        expect(received[0].method).toBe("POST");
        expect(JSON.parse(received[0].body)).toEqual({
          seed: { Inference: "fixture-inference" },
        });
      },
    );
  });

  it.each([
    ["http://localhost:3000", "127.0.0.1:3000"],
    ["http://127.0.0.1:3000", "localhost:3000"],
  ])(
    "admits same-origin ensure through Host when Next reconstructs %s",
    async (internal, host) => {
      await withUpstream({ job_id: "existing-job" }, async ({ received }) => {
        const input = (origin: string) =>
          new Request(`${internal}/api/v1/brief/ensure`, {
            method: "POST",
            headers: { host, origin, "content-type": "application/json" },
            body: JSON.stringify({ run: "r1", id: "a1" }),
          });
        const response = await proxyRust(input(`http://${host}`), [
          "brief",
          "ensure",
        ]);
        expect(response.status).toBe(200);
        expect(await response.json()).toEqual({ job_id: "existing-job" });
        expect(received).toHaveLength(1);
        expect(received[0].url.pathname).toBe("/api/v1/brief/ensure");
        expect(received[0].headers.origin).toBeUndefined();
        expect(JSON.parse(received[0].body)).toEqual({ run: "r1", id: "a1" });
        // Loopback aliases are distinct browser origins, as are different ports.
        expect(
          (await proxyRust(input(internal), ["brief", "ensure"])).status,
        ).toBe(403);
        expect(
          (
            await proxyRust(input(`http://${host.replace(":3000", ":3001")}`), [
              "brief",
              "ensure",
            ])
          ).status,
        ).toBe(403);
        expect(received).toHaveLength(1);
      });
    },
  );

  it("rejects cross-origin writes, unknown routes, wrong methods and oversized bodies before forwarding", async () => {
    const input = (body: string, origin = next) =>
      new Request(`${next}/api/v1/topology/resolve`, {
        method: "POST",
        headers: { origin, "content-type": "application/json" },
        body,
      });
    expect(
      (
        await proxyRust(input("{}", "http://untrusted.test"), [
          "topology",
          "resolve",
        ])
      ).status,
    ).toBe(403);
    expect(
      (
        await proxyRust(
          new Request(`${next}/api/v1/days`, {
            headers: { host: "untrusted.test:3000" },
          }),
          ["days"],
        )
      ).status,
    ).toBe(403);
    expect(
      (await proxyRust(new Request(`${next}/api/v1/other`), ["other"])).status,
    ).toBe(404);
    expect(
      (
        await proxyRust(new Request(`${next}/api/v1/topology/resolve`), [
          "topology",
          "resolve",
        ])
      ).status,
    ).toBe(405);
    expect(
      (await proxyRust(input("x".repeat(16385)), ["topology", "resolve"]))
        .status,
    ).toBe(413);
  });

  it("hides server configuration and upstream connection failures", async () => {
    vi.stubEnv("ARGOS_RUST_API_ORIGIN", "http://private.example.test:9188");
    vi.stubEnv("FLEETWORKS_API_ORIGIN", "invalid-private-origin");
    const response = await proxyRust(
      new Request(`${next}/api/v1/health/ready`),
      ["health", "ready"],
    );
    expect(response.status).toBe(503);
    const text = await response.text();
    expect(text).not.toContain("9188");
    expect(text).not.toContain("private.example.test");
  });
});

it("proxies actor dossier GET and refuses mutations", async () => {
  await withUpstream({ id: "canonical" }, async ({ received }) => {
    const url = `${next}/api/v1/actors/detail?id=actor%20%2F%26`;
    expect(
      await (await proxyRust(new Request(url), ["actors", "detail"])).json(),
    ).toEqual({ id: "canonical" });
    expect(
      (
        await proxyRust(new Request(url, { method: "POST" }), [
          "actors",
          "detail",
        ])
      ).status,
    ).toBe(405);
    expect(received).toHaveLength(1);
    expect(received[0].url.pathname).toBe("/api/v1/actors/detail");
    expect(received[0].url.searchParams.get("id")).toBe("actor /&");
  });
});
