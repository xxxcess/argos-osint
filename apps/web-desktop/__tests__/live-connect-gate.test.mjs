import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { parseConnectId } from "../connect-id.mjs";
import {
  ConnectFailure,
  CONNECT_COOKIE,
  connectionProofHeaders,
  createConnectAuthority,
  createConnectionResponse,
  currentConnectionResponse,
  isPublicWebPath,
  logoutConnectionResponse,
  publicConnection,
  requireConnection,
  requireDevelopmentUpgrade,
  safeReturnPath,
  sessionTokenFromCookie,
} from "../connect-session.mjs";
import {
  installGatewayEnvironment,
  startConnectGateway,
} from "./connect-gateway-fixture.mjs";

test("browser/Node Connect parser consumes the same Rust corpus", async () => {
  const corpus = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/fleetworks-contracts/tests/fixtures/connect_ids.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  for (const fixture of corpus.valid) {
    assert.deepEqual(parseConnectId(fixture.input), {
      connectId: fixture.canonical,
      workstationId: fixture.canonical,
    });
  }
  for (const fixture of corpus.invalid)
    assert.throws(() => parseConnectId(fixture));
});

test("public allowlist excludes protected pages/RSC/data/export and unsafe return destinations", () => {
  for (const path of [
    "/connect",
    "/api/connect",
    "/api/connect/logout",
    "/_next/static/chunks/example.js",
  ])
    assert.equal(isPublicWebPath(path), true);
  for (const path of [
    "/",
    "/home",
    "/bulletin",
    "/brief",
    "/actor",
    "/topology",
    "/api/v1/articles",
    "/export",
    "/_next/image",
  ])
    assert.equal(isPublicWebPath(path), false);
  for (const value of [
    "//untrusted.test/path",
    "https://untrusted.test",
    "/\\untrusted.test",
    "/connect",
    "/api/connect",
    "/x\nLocation:evil",
  ])
    assert.equal(safeReturnPath(value), "/");
  assert.equal(
    safeReturnPath("/brief?run=fixture&id=article"),
    "/brief?run=fixture&id=article",
  );
  for (const cookie of [
    undefined,
    "",
    `${CONNECT_COOKIE}=forged`,
    `${CONNECT_COOKIE}=${"a".repeat(64)}; ${CONNECT_COOKIE}=${"b".repeat(64)}`,
  ])
    assert.throws(() => sessionTokenFromCookie(cookie));
});

test("same authoritative guard denies forged/revoked/stale requests before protected data", async (t) => {
  const gateway = await startConnectGateway();
  const restore = installGatewayEnvironment(gateway);
  t.after(async () => {
    restore();
    await gateway.close();
  });
  const request = (extra = {}) =>
    new Request("http://127.0.0.1:3000/brief?run=fixture", {
      headers: {
        cookie: `${CONNECT_COOKIE}=${gateway.sessionToken}`,
        ...extra,
      },
    });
  const valid = await requireConnection(request());
  assert.equal(valid.context.workstation_id, gateway.context.workstation_id);
  assert.deepEqual(connectionProofHeaders(valid), {
    "x-fleetworks-session": gateway.sessionToken,
    "x-fleetworks-context-revision": "1",
  });
  assert.equal(
    JSON.stringify(publicConnection(valid)).includes(gateway.sessionToken),
    false,
  );
  await assert.rejects(
    requireConnection(new Request("http://127.0.0.1:3000/")),
    (error) => error instanceof ConnectFailure && error.status === 401,
  );
  await assert.rejects(
    requireConnection(
      request({ "x-fleetworks-expected-context-revision": "2" }),
    ),
    (error) => error instanceof ConnectFailure && error.status === 409,
  );
  await assert.rejects(
    requireConnection(request({ origin: "https://foreign.example" })),
    (error) => error instanceof ConnectFailure && error.status === 403,
  );
  gateway.revoked.add(gateway.sessionToken);
  await assert.rejects(
    requireConnection(request()),
    (error) => error instanceof ConnectFailure && error.status === 401,
  );
  assert.equal(
    gateway.received.some(
      (entry) => !entry.path.startsWith("/api/v1/connect/sessions"),
    ),
    false,
  );
});

test("Connect creates a cookie for existing registration only and never exposes session bearer", async (t) => {
  const gateway = await startConnectGateway();
  const restore = installGatewayEnvironment(gateway);
  t.after(async () => {
    restore();
    await gateway.close();
  });
  const make = (body, origin = "http://127.0.0.1:3000") =>
    new Request("http://127.0.0.1:3000/api/connect", {
      method: "POST",
      headers: { origin, "content-type": "application/json" },
      body: JSON.stringify(body),
    });
  const valid = await createConnectionResponse(
    make({ connect_id: "fixture-workstation-research" }),
  );
  assert.equal(valid.status, 200);
  const cookie = valid.headers.get("set-cookie");
  assert.match(cookie, /HttpOnly/u);
  assert.match(cookie, /SameSite=Strict/u);
  assert.doesNotMatch(cookie, /Domain=/u);
  const body = await valid.json();
  assert.equal(body.session_token, undefined);
  assert.equal(body.token, undefined);
  assert.equal(body.context.connect_id, "fixture-workstation-research");
  assert.equal(
    (
      await createConnectionResponse(
        make({ connect_id: "fixture-workstation-unknown" }),
      )
    ).status,
    401,
  );
  assert.equal(
    (
      await createConnectionResponse(
        make({ connect_id: "fixture-workstation-research", owner: "forged" }),
      )
    ).status,
    400,
  );
  assert.equal(
    (
      await createConnectionResponse(
        make(
          { connect_id: "fixture-workstation-research" },
          "http://untrusted.example",
        ),
      )
    ).status,
    403,
  );
  assert.equal(
    gateway.received.some((entry) => entry.path.includes("registrations")),
    false,
  );
});

test("logout revokes authority; outage is unavailable rather than fake empty success", async (t) => {
  const gateway = await startConnectGateway();
  const restore = installGatewayEnvironment(gateway);
  t.after(async () => {
    restore();
    await gateway.close();
  });
  const request = () =>
    new Request("http://127.0.0.1:3000/api/connect", {
      headers: { cookie: `${CONNECT_COOKIE}=${gateway.sessionToken}` },
    });
  assert.equal((await currentConnectionResponse(request())).status, 200);
  gateway.setUnavailable(true);
  assert.equal((await currentConnectionResponse(request())).status, 503);
  gateway.setUnavailable(false);
  const logout = new Request("http://127.0.0.1:3000/api/connect/logout", {
    method: "POST",
    headers: {
      origin: "http://127.0.0.1:3000",
      "content-type": "application/json",
      cookie: `${CONNECT_COOKIE}=${gateway.sessionToken}`,
    },
    body: "{}",
  });
  assert.equal((await logoutConnectionResponse(logout)).status, 200);
  assert.equal((await currentConnectionResponse(request())).status, 401);
});

test("authority responses reject expired, malformed and secret-bearing extra context", async () => {
  const now = Date.parse("2026-01-01T00:00:00Z");
  const token = "a".repeat(64);
  const gateway = await startConnectGateway();
  try {
    const authority = createConnectAuthority({
      now: () => now,
      request: async () => ({
        session_token: token,
        context: { ...gateway.context, hidden_secret: "must-not-serialize" },
        expires_at: "2026-01-01T01:00:00Z",
      }),
    });
    assert.equal(
      JSON.stringify(
        publicConnection(
          await authority.create("fixture-workstation-research"),
        ),
      ).includes("must-not-serialize"),
      false,
    );
    const expired = createConnectAuthority({
      now: () => now,
      request: async () => ({
        session_token: token,
        context: gateway.context,
        expires_at: "2025-01-01T00:00:00Z",
      }),
    });
    await assert.rejects(expired.create("fixture-workstation-research"));
  } finally {
    await gateway.close();
  }
});

test("remote HTTPS still requires authenticated ingress proof and emits Secure cookies", async (t) => {
  const gateway = await startConnectGateway();
  const restore = installGatewayEnvironment(
    gateway,
    "https://intel.example.test",
  );
  const directory = await mkdtemp(
    join(tmpdir(), "fleetworks-ingress-fixture-"),
  );
  const secret = "synthetic-ingress-forwarding-token-123456789012";
  const file = join(directory, "ingress-token");
  await writeFile(file, `${secret}\n`, { mode: 0o600 });
  process.env.FLEETWORKS_WEB_AUTHENTICATED_INGRESS = "1";
  process.env.FLEETWORKS_WEB_INGRESS_TOKEN_FILE = file;
  t.after(async () => {
    restore();
    await gateway.close();
    await rm(directory, { recursive: true, force: true });
  });
  const input = (proof) =>
    new Request("https://intel.example.test/api/connect", {
      method: "POST",
      headers: {
        origin: "https://intel.example.test",
        "content-type": "application/json",
        ...(proof ? { "x-fleetworks-ingress-token": proof } : {}),
        forwarded: "for=127.0.0.1;proto=https",
      },
      body: JSON.stringify({ connect_id: "fixture-workstation-research" }),
    });
  assert.equal((await createConnectionResponse(input())).status, 403);
  assert.equal((await createConnectionResponse(input("forged"))).status, 403);
  assert.equal(
    gateway.received.length,
    0,
    "forwarded headers cannot unlock remote access",
  );
  const accepted = await createConnectionResponse(input(secret));
  assert.equal(accepted.status, 200);
  assert.match(accepted.headers.get("set-cookie"), /Secure/u);
  assert.equal((await accepted.text()).includes(secret), false);
});

test("base paths, malformed bodies and bounded JSON do not widen public access", async (t) => {
  const gateway = await startConnectGateway();
  const restore = installGatewayEnvironment(gateway);
  process.env.FLEETWORKS_WEB_BASE_PATH = "/intel";
  t.after(async () => {
    restore();
    await gateway.close();
  });
  assert.equal(isPublicWebPath("/intel/connect"), true);
  assert.equal(isPublicWebPath("/connect"), false);
  assert.equal(isPublicWebPath("/intel/api/v1/articles"), false);
  assert.equal(
    safeReturnPath("/intel/brief?run=fixture"),
    "/intel/brief?run=fixture",
  );
  assert.equal(safeReturnPath("/outside-prefix"), "/intel/");
  const input = (body, type = "application/json") =>
    new Request("http://127.0.0.1:3000/intel/api/connect", {
      method: "POST",
      headers: { origin: "http://127.0.0.1:3000", "content-type": type },
      body,
    });
  assert.equal(
    (await createConnectionResponse(input("x".repeat(1025)))).status,
    413,
  );
  assert.equal(
    (await createConnectionResponse(input("{}", "notapplication/json"))).status,
    415,
  );
  assert.equal((await createConnectionResponse(input("{bad"))).status, 400);
  const accepted = await createConnectionResponse(
    input(JSON.stringify({ connect_id: "fixture-workstation-research" })),
  );
  assert.equal(accepted.status, 200);
  assert.match(accepted.headers.get("set-cookie"), /Path=\/intel;/u);
});

test("opaque workstation IDs reject legacy names and never trim or normalize authority", () => {
  for (const value of [
    "192.0.2.42:research",
    "[2001:db8::42]:default",
    "workstation-a\n",
    " workstation-a",
    "workstation-a ",
    "workstation/a",
    "workstation\\a",
    "café",
    "",
    "a".repeat(129),
  ])
    assert.throws(() => parseConnectId(value), value);
  assert.deepEqual(parseConnectId("Node-A_123.4"), {
    connectId: "Node-A_123.4",
    workstationId: "Node-A_123.4",
  });
  assert.throws(() =>
    sessionTokenFromCookie(`fleetworks_connect=${"a".repeat(64)}`),
  );
});

test("workstation authority accepts optional changing IP metadata but rejects legacy context and mismatched identity", async (t) => {
  const gateway = await startConnectGateway();
  t.after(() => gateway.close());
  const id = gateway.context.workstation_id;
  const payload = (context) => ({
    session_token: "b".repeat(64),
    context,
    expires_at: new Date(Date.now() + 60_000).toISOString(),
  });
  for (const workstation_ip of [
    undefined,
    null,
    "198.51.100.9",
    "2001:db8::42",
  ]) {
    const authority = createConnectAuthority({
      request: async () => payload({ ...gateway.context, workstation_ip }),
    });
    const result = await authority.create(id);
    assert.equal(result.context.workstation_id, id);
    assert.equal(result.context.connect_id, id);
    assert.equal(result.context.workstation_ip, workstation_ip);
  }
  for (const change of [
    { workspace_id: "legacy" },
    { workspace_name: "research" },
    { workspace_marker: "legacy-marker" },
    { connect_id: "192.0.2.42:research" },
    { workstation_id: "different-installation" },
    { workstation_ip: "not-an-ip" },
    { workstation_ip: 42 },
  ]) {
    const authority = createConnectAuthority({
      request: async () => payload({ ...gateway.context, ...change }),
    });
    await assert.rejects(
      authority.create(id),
      (error) => error instanceof ConnectFailure && error.status === 503,
    );
  }
  const swapped = createConnectAuthority({
    request: async () =>
      payload({
        ...gateway.context,
        workstation_id: "different-installation",
        connect_id: "different-installation",
      }),
  });
  await assert.rejects(swapped.create(id));
});

test("development HMR permits exact current/legacy framework paths only under the normal origin boundary", async (t) => {
  const gateway = await startConnectGateway();
  const restore = installGatewayEnvironment(gateway);
  t.after(async () => {
    restore();
    await gateway.close();
  });
  const request = (origin = "http://127.0.0.1:3000") => ({
    url: "http://127.0.0.1:3000/_next/hmr",
    headers: { host: "127.0.0.1:3000", origin },
  });
  for (const path of ["/_next/hmr", "/_next/webpack-hmr"]) {
    await requireDevelopmentUpgrade(request(), true, path);
    await assert.rejects(requireDevelopmentUpgrade(request(), false, path));
    await assert.rejects(
      requireDevelopmentUpgrade(request("https://foreign.example"), true, path),
    );
  }
  for (const path of [
    "/_next/hmr/extra",
    "/_next/webpack-hmr-forged",
    "/api/v1/live",
    undefined,
  ])
    await assert.rejects(requireDevelopmentUpgrade(request(), true, path));
  assert.equal(gateway.received.length, 0);
});
