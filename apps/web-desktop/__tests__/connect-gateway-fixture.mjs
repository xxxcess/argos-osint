import { createServer } from "node:http";
import { once } from "node:events";
import { randomBytes } from "node:crypto";
import { parseConnectId } from "../connect-id.mjs";

/** External synthetic gateway, never imported by production app/server modules. */
export async function startConnectGateway({
  dataHandler,
  apiToken = "fleetworks-web-fixture-purpose-token-1234567890",
} = {}) {
  const sessionToken = "a".repeat(64);
  let revision = 1;
  const contextFor = (id, contextRevision) => {
    const parsed = parseConnectId(id);
    return {
      deployment_id: "fixture-deployment",
      registration_id: `fixture-registration-${parsed.workspaceName}`,
      workstation_id: `fixture-node-${parsed.ip}`,
      workstation_ip: parsed.ip,
      workspace_id: `fixture-workspace-${parsed.workspaceName}`,
      workspace_name: parsed.workspaceName,
      connect_id: parsed.connectId,
      registration_revision: 1,
      context_revision: contextRevision,
      scope_revision: 1,
      configuration_epoch: "fixture-reset-1",
      access_epoch: "fixture-web-access-1",
    };
  };
  const context = contextFor("192.0.2.42:research", revision);
  const sessions = new Map([[sessionToken, context]]);
  const registered = new Set([
    "192.0.2.42:research",
    "192.0.2.42:review",
    "192.0.2.42:default",
    "192.0.2.43:research",
  ]);
  const revoked = new Set();
  const received = [];
  let unavailable = false;
  const server = createServer(async (request, response) => {
    const respond = (status, body) =>
      response
        .writeHead(status, {
          "content-type": "application/json",
          "cache-control": "private, no-store",
        })
        .end(JSON.stringify(body));
    try {
      const chunks = [];
      let size = 0;
      for await (const chunk of request) {
        size += chunk.length;
        if (size > 32 * 1024) {
          respond(413, { error: "fixture body bound" });
          return;
        }
        chunks.push(chunk);
      }
      let body;
      try {
        body = chunks.length
          ? JSON.parse(Buffer.concat(chunks).toString("utf8"))
          : undefined;
      } catch {
        respond(400, { error: "invalid fixture JSON" });
        return;
      }
      received.push({
        path: request.url,
        method: request.method,
        headers: { ...request.headers },
        body,
      });
      if (request.headers.authorization !== `Bearer ${apiToken}`) {
        respond(403, { error: "wrong fixture purpose credential" });
        return;
      }
      if (unavailable) {
        respond(503, { error: "fixture unavailable" });
        return;
      }
      const pathname = new URL(request.url, "http://fixture.invalid").pathname;
      if (
        pathname === "/api/v1/connect/sessions" &&
        request.method === "POST"
      ) {
        if (!registered.has(body?.connect_id)) {
          respond(401, { error: "connection unavailable" });
          return;
        }
        const token = randomBytes(32).toString("hex");
        const next = contextFor(body.connect_id, ++revision);
        sessions.set(token, next);
        respond(201, {
          session_token: token,
          context: next,
          expires_at: new Date(Date.now() + 60 * 60 * 1000).toISOString(),
        });
        return;
      }
      if (
        pathname === "/api/v1/connect/sessions/resolve" &&
        request.method === "POST"
      ) {
        const current = sessions.get(body?.session_token);
        if (!current || revoked.has(body.session_token)) {
          respond(401, { error: "connection unavailable" });
          return;
        }
        if (
          body.expected_context_revision !== undefined &&
          body.expected_context_revision !== current.context_revision
        ) {
          respond(409, { error: "context changed" });
          return;
        }
        respond(200, {
          context: current,
          expires_at: new Date(Date.now() + 60 * 60 * 1000).toISOString(),
        });
        return;
      }
      if (
        pathname === "/api/v1/connect/sessions/logout" &&
        request.method === "POST"
      ) {
        revoked.add(body?.session_token);
        respond(200, { revoked: true });
        return;
      }
      const current = sessions.get(request.headers["x-fleetworks-session"]);
      if (!current || revoked.has(request.headers["x-fleetworks-session"])) {
        respond(401, { error: "missing validated fixture context" });
        return;
      }
      if (
        request.headers["x-fleetworks-context-revision"] !==
        String(current.context_revision)
      ) {
        respond(409, { error: "wrong context revision" });
        return;
      }
      if (dataHandler && (await dataHandler(request, response, body))) return;
      if (pathname === "/api/v1/health/ready") {
        respond(200, {
          service: "argos-intel",
          api_version: 1,
          ready: true,
          storage: true,
          instance_id: "connect-gateway-fixture",
        });
        return;
      }
      respond(200, { rows: [], days: [], fixture: true });
    } catch {
      if (!response.headersSent) respond(500, { error: "fixture failure" });
      else response.destroy();
    }
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const origin = `http://127.0.0.1:${server.address().port}`;
  return {
    server,
    origin,
    apiToken,
    sessionToken,
    context,
    received,
    sessions,
    revoked,
    setUnavailable(value) {
      unavailable = value;
    },
    async close() {
      server.closeAllConnections();
      await new Promise((resolve) => server.close(resolve));
    },
  };
}

export function installGatewayEnvironment(
  gateway,
  webOrigin = "http://127.0.0.1:3000",
) {
  const keys = [
    "ARGOS_RUST_API_ORIGIN",
    "FLEETWORKS_API_ORIGIN",
    "FLEETWORKS_API_TOKEN",
    "FLEETWORKS_API_TOKEN_FILE",
    "FLEETWORKS_WEB_ORIGIN",
    "FLEETWORKS_WEB_BASE_PATH",
    "FLEETWORKS_WEB_AUTHENTICATED_INGRESS",
    "FLEETWORKS_WEB_INGRESS_TOKEN_FILE",
  ];
  const prior = new Map(keys.map((key) => [key, process.env[key]]));
  for (const key of keys) delete process.env[key];
  process.env.FLEETWORKS_API_ORIGIN = gateway.origin;
  process.env.FLEETWORKS_API_TOKEN = gateway.apiToken;
  process.env.FLEETWORKS_WEB_ORIGIN = webOrigin;
  return () => {
    for (const [key, value] of prior) {
      if (value === undefined) delete process.env[key];
      else process.env[key] = value;
    }
  };
}
