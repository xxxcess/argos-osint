// @vitest-environment node
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { http, passthrough } from "msw";
import { mswServer } from "../../../vitest.setup";
import { proxyRust } from "../app/server-api";
import { CONNECT_COOKIE } from "../connect-session.mjs";
import {
  installGatewayEnvironment,
  startConnectGateway,
} from "./connect-gateway-fixture.mjs";

let gateway: Awaited<ReturnType<typeof startConnectGateway>>;
let restore: () => void;
const web = "http://127.0.0.1:3000";
beforeEach(async () => {
  gateway = await startConnectGateway();
  restore = installGatewayEnvironment(gateway, web);
  mswServer.use(http.all(`${gateway.origin}/*`, () => passthrough()));
});
afterEach(async () => {
  restore();
  await gateway.close();
});

function input(
  path: string,
  options: {
    method?: string;
    headers?: Record<string, string>;
    body?: string;
    cookie?: boolean;
  } = {},
) {
  return new Request(`${web}/api/v1/${path}`, {
    method: options.method || "GET",
    headers: {
      ...(options.cookie === false
        ? {}
        : { cookie: `${CONNECT_COOKIE}=${gateway.sessionToken}` }),
      ...(options.method === "POST"
        ? { origin: web, "content-type": "application/json" }
        : {}),
      ...options.headers,
    },
    ...(options.body === undefined ? {} : { body: options.body }),
  });
}

describe("authoritative registered REST boundary", () => {
  it("refuses absent or forged cookies even when browser supplies trusted-looking context headers", async () => {
    const request = input("articles", {
      cookie: false,
      headers: {
        "x-fleetworks-session": gateway.sessionToken,
        "x-fleetworks-context-revision": "1",
        "x-fleetworks-workstation-id": "forged",
      },
    });
    expect((await proxyRust(request, ["articles"])).status).toBe(401);
    expect(gateway.received).toHaveLength(0);
    const forged = input("articles", {
      cookie: false,
      headers: { cookie: `${CONNECT_COOKIE}=${"b".repeat(64)}` },
    });
    expect((await proxyRust(forged, ["articles"])).status).toBe(401);
    expect(
      gateway.received.every((entry) =>
        entry.path.endsWith("sessions/resolve"),
      ),
    ).toBe(true);
  });

  it("rejects legacy workspace query narrowing before any authority or data request", async () => {
    for (const field of [
      "workspace",
      "workspace_id",
      "workspace_name",
      "workspace_marker",
    ]) {
      const response = await proxyRust(input(`articles?${field}=legacy`), [
        "articles",
      ]);
      expect(response.status).toBe(400);
    }
    expect(gateway.received).toHaveLength(0);
  });

  it("creates backend proof only from current authoritative resolution and never forwards browser credentials", async () => {
    const request = input("articles?cursor=opaque%2B%2F%3D", {
      headers: {
        authorization: "Bearer forged-browser",
        "proxy-authorization": "Basic forged",
        "x-fleetworks-session": "b".repeat(64),
        "x-fleetworks-context-revision": "999",
      },
    });
    const response = await proxyRust(request, ["articles"]);
    expect(response.status).toBe(200);
    const forwarded = gateway.received.find((entry) =>
      entry.path.startsWith("/api/v1/articles"),
    )!;
    expect(
      new URL(forwarded.path, gateway.origin).searchParams.get("cursor"),
    ).toBe("opaque+/=");
    expect(forwarded.headers.authorization).toBe(`Bearer ${gateway.apiToken}`);
    expect(forwarded.headers["x-fleetworks-session"]).toBe(
      gateway.sessionToken,
    );
    expect(forwarded.headers["x-fleetworks-context-revision"]).toBe("1");
    for (const header of [
      "cookie",
      "origin",
      "proxy-authorization",
      "x-fleetworks-workstation-id",
    ])
      expect(forwarded.headers[header]).toBeUndefined();
    expect(response.headers.get("set-cookie")).toBeNull();
    expect(await response.text()).not.toContain(gateway.sessionToken);
  });

  it("rejects stale-tab and missing context revision before any mutation is forwarded", async () => {
    for (const revision of [undefined, "2", "0", "9007199254740993"]) {
      const request = input("brief/ensure", {
        method: "POST",
        headers: revision
          ? { "x-fleetworks-expected-context-revision": revision }
          : {},
        body: JSON.stringify({ run: "fixture", id: "article" }),
      });
      expect((await proxyRust(request, ["brief", "ensure"])).status).toBe(409);
    }
    expect(
      gateway.received.some((entry) => entry.path.includes("brief/ensure")),
    ).toBe(false);
    const valid = input("brief/ensure", {
      method: "POST",
      headers: { "x-fleetworks-expected-context-revision": "1" },
      body: JSON.stringify({ run: "fixture", id: "article" }),
    });
    expect((await proxyRust(valid, ["brief", "ensure"])).status).toBe(200);
    expect(
      gateway.received.find((entry) => entry.path.endsWith("brief/ensure"))
        ?.body,
    ).toEqual({ run: "fixture", id: "article" });
  });

  it("fails closed on registry outage and revocation without exposing configured endpoints", async () => {
    gateway.setUnavailable(true);
    const unavailable = await proxyRust(input("articles"), ["articles"]);
    expect(unavailable.status).toBe(503);
    expect(await unavailable.text()).not.toContain(gateway.origin);
    gateway.setUnavailable(false);
    gateway.revoked.add(gateway.sessionToken);
    expect((await proxyRust(input("articles"), ["articles"])).status).toBe(401);
    expect(
      gateway.received.some((entry) =>
        entry.path.startsWith("/api/v1/articles"),
      ),
    ).toBe(false);
  });

  it("keeps native registration, uploads and maintenance outside the browser generic allowlist", async () => {
    for (const path of [
      "connect/registrations/ensure",
      "actors/seed",
      "classifier/train",
      "search/actors/reindex",
    ]) {
      expect(
        (
          await proxyRust(
            input(path, { method: "POST", body: "{}" }),
            path.split("/"),
          )
        ).status,
      ).toBe(404);
    }
    expect(gateway.received).toHaveLength(0);
  });
});
