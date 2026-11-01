// @vitest-environment node
import { afterEach, describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { mswServer } from "../../../vitest.setup";
import { proxyRust } from "../app/server-api";

const next = "http://127.0.0.1:3000";
const rust = "http://127.0.0.1:9188";
afterEach(() => vi.unstubAllEnvs());

describe("Next same-origin API → Rust transport", () => {
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
