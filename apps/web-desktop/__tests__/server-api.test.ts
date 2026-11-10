// @vitest-environment node
import { afterEach, describe, expect, it, vi } from "vitest";
import { http, HttpResponse, passthrough } from "msw";
import { once } from "node:events";
import { createServer, type IncomingHttpHeaders } from "node:http";
import { mswServer } from "../../../vitest.setup";
import { proxyRust } from "../app/server-api";

const next = "http://127.0.0.1:3000";
const rust = "http://127.0.0.1:9188";
afterEach(() => vi.unstubAllEnvs());

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
    vi.stubEnv("ARGOS_RUST_API_ORIGIN", rust);
    let target: URL | undefined;
    mswServer.use(
      http.get(`${rust}/api/v1/articles`, ({ request }) => {
        target = new URL(request.url);
        expect(request.headers.get("origin")).toBeNull();
        expect(request.headers.get("cookie")).toBeNull();
        return HttpResponse.json(
          { rows: [], next_cursor: "opaque:2" },
          { status: 200 },
        );
      }),
    );
    const response = await proxyRust(
      new Request(`${next}/api/v1/articles?cursor=a%2Bb%2F%3D`, {
        headers: { origin: next, cookie: "private=x" },
      }),
      ["articles"],
    );
    expect(response.status).toBe(200);
    expect(target?.origin).toBe(rust);
    expect(target?.searchParams.get("cursor")).toBe("a+b/=");
    expect(await response.json()).toEqual({
      rows: [],
      next_cursor: "opaque:2",
    });
    expect(response.headers.get("cache-control")).toBe("no-store");
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
    vi.stubEnv("ARGOS_RUST_API_ORIGIN", rust);
    mswServer.use(
      http.post(`${rust}/api/v1/topology/resolve`, async ({ request }) => {
        expect(await request.json()).toEqual({
          seed: { Inference: "fixture-inference" },
        });
        return HttpResponse.json({ nodes: [{ kind: "inference" }] });
      }),
    );
    const response = await proxyRust(
      new Request(`${next}/api/v1/topology/resolve`, {
        method: "POST",
        headers: { origin: next, "content-type": "application/json" },
        body: JSON.stringify({ seed: { Inference: "fixture-inference" } }),
      }),
      ["topology", "resolve"],
    );
    expect(await response.json()).toEqual({ nodes: [{ kind: "inference" }] });
  });

  it.each([
    ["http://localhost:3000", "127.0.0.1:3000"],
    ["http://127.0.0.1:3000", "localhost:3000"],
  ])(
    "admits same-origin ensure through Host when Next reconstructs %s",
    async (internal, host) => {
      vi.stubEnv("ARGOS_RUST_API_ORIGIN", rust);
      mswServer.use(
        http.post(`${rust}/api/v1/brief/ensure`, async ({ request }) => {
          expect(request.headers.get("origin")).toBeNull();
          expect(await request.json()).toEqual({ run: "r1", id: "a1" });
          return HttpResponse.json({ job_id: "existing-job" });
        }),
      );
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
  vi.stubEnv("ARGOS_RUST_API_ORIGIN", rust);
  mswServer.use(
    http.get(`${rust}/api/v1/actors/detail`, ({ request }) => {
      expect(new URL(request.url).searchParams.get("id")).toBe("actor /&");
      return HttpResponse.json({ id: "canonical" });
    }),
  );
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
});
