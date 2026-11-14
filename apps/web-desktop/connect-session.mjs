import { timingSafeEqual } from "node:crypto";
import { isIP } from "node:net";
import { request as httpRequest } from "node:http";
import { request as httpsRequest } from "node:https";
import {
  configuredWebBasePath,
  configuredWebOrigin,
  directAgent,
  incomingWebOrigin,
  readProtectedTokenFile,
  rustUpstream,
} from "./server-origin.mjs";
import { parseConnectId } from "./connect-id.mjs";

// A new name forces a fresh Connect instead of reviving a workspace session.
export const CONNECT_COOKIE = "fleetworks_connect_v2";
export const EXPECTED_CONTEXT_HEADER = "x-fleetworks-expected-context-revision";
export const CONNECT_RESPONSE_HEADERS = Object.freeze({
  "cache-control": "private, no-store, max-age=0",
  pragma: "no-cache",
  "x-content-type-options": "nosniff",
  "referrer-policy": "no-referrer",
  vary: "Cookie, Origin",
});

/** Safe public failures never contain backend addresses, IDs or credential bytes. */
export class ConnectFailure extends Error {
  constructor(status, code) {
    const messages = {
      400: "Enter a valid registered Connect ID",
      401: "Connection required or registration no longer available",
      403: "Web access or request origin refused",
      409: "Connection context changed; reconnect this tab before continuing",
      413: "Connect request exceeds the size limit",
      415: "JSON request required",
      429: "Too many connection attempts; retry later",
      503: "Connection authority unavailable; retry without changing context",
    };
    super(messages[status] || messages[503]);
    this.name = "ConnectFailure";
    this.status = status;
    this.code = code;
  }
}

export function connectFailureResponse(error) {
  const failure =
    error instanceof ConnectFailure
      ? error
      : new ConnectFailure(503, "authority_unavailable");
  return Response.json(
    { error: failure.message, code: failure.code },
    { status: failure.status, headers: CONNECT_RESPONSE_HEADERS },
  );
}

export function headerValue(headers, name) {
  if (typeof headers?.get === "function") return headers.get(name);
  const value = headers?.[name.toLowerCase()];
  return typeof value === "string" ? value : undefined;
}

export function safeReturnPath(raw) {
  const base = configuredWebBasePath();
  const fallback = `${base}/`;
  if (
    typeof raw !== "string" ||
    raw.length > 2048 ||
    !raw.startsWith("/") ||
    raw.startsWith("//") ||
    /[\\\r\n\u0000]/u.test(raw)
  )
    return fallback;
  try {
    const parsed = new URL(raw, "https://return.invalid");
    const local = relativeWebPath(parsed.pathname);
    if (
      parsed.origin !== "https://return.invalid" ||
      local === undefined ||
      local === "/connect" ||
      local.startsWith("/api/")
    )
      return fallback;
    return `${parsed.pathname}${parsed.search}${parsed.hash}`;
  } catch {
    return fallback;
  }
}

/** Narrow public surface: no app shell, data/image proxy, export or RSC page. */
export function isPublicWebPath(pathname) {
  const path = relativeWebPath(pathname);
  return (
    path !== undefined &&
    (path === "/connect" ||
      path === "/api/connect" ||
      path === "/api/connect/logout" ||
      path === "/favicon.ico" ||
      path === "/fleetworks-mark.svg" ||
      path === "/fleetworks-logo.svg" ||
      path.startsWith("/_next/static/"))
  );
}

export function relativeWebPath(pathname) {
  const base = configuredWebBasePath();
  if (!base) return pathname;
  if (pathname === base) return "/";
  return pathname.startsWith(`${base}/`)
    ? pathname.slice(base.length)
    : undefined;
}

function presentedOrigin(request) {
  let url;
  try {
    url = new URL(request.url || "/", "http://127.0.0.1");
  } catch {
    throw new ConnectFailure(403, "origin_refused");
  }
  try {
    return incomingWebOrigin(
      headerValue(request.headers, "host") || url.host,
      url.protocol,
    );
  } catch {
    throw new ConnectFailure(403, "origin_refused");
  }
}

/** Remote access remains authenticated by the existing external ingress. */
export async function requireWebBoundary(request, mutation = false) {
  const origin = presentedOrigin(request);
  const presented = headerValue(request.headers, "origin");
  if (
    (presented && presented !== origin) ||
    (mutation && presented !== origin) ||
    headerValue(request.headers, "sec-fetch-site") === "cross-site"
  )
    throw new ConnectFailure(403, "origin_refused");
  const configured = configuredWebOrigin();
  if (
    configured &&
    !["127.0.0.1", "localhost", "[::1]"].includes(new URL(configured).hostname)
  ) {
    const supplied = headerValue(request.headers, "x-fleetworks-ingress-token");
    let expected;
    try {
      expected = await readProtectedTokenFile(
        process.env.FLEETWORKS_WEB_INGRESS_TOKEN_FILE,
      );
    } catch {
      throw new ConnectFailure(503, "ingress_unavailable");
    }
    if (
      expected.length < 32 ||
      !/^[A-Za-z0-9._~+/-]+=*(?![\s\S])/u.test(expected)
    )
      throw new ConnectFailure(503, "ingress_unavailable");
    if (
      !supplied ||
      Buffer.byteLength(supplied) !== Buffer.byteLength(expected) ||
      !timingSafeEqual(Buffer.from(supplied), Buffer.from(expected))
    )
      throw new ConnectFailure(403, "ingress_refused");
  }
  return origin;
}

/** Exact development-only framework sockets retain the ordinary origin boundary. */
export async function requireDevelopmentUpgrade(request, development, path) {
  await requireWebBoundary(request);
  if (!development || !["/_next/webpack-hmr", "/_next/hmr"].includes(path))
    throw new ConnectFailure(403, "upgrade_refused");
}

export function sessionTokenFromCookie(raw) {
  if (typeof raw !== "string" || raw.length > 8192)
    throw new ConnectFailure(401, "connection_required");
  const found = raw
    .split(";")
    .map((part) => part.trim())
    .filter((part) => part.startsWith(`${CONNECT_COOKIE}=`));
  if (found.length !== 1) throw new ConnectFailure(401, "connection_required");
  const token = found[0].slice(CONNECT_COOKIE.length + 1);
  if (token.length !== 64 || !/^[a-f0-9]{64}$/u.test(token))
    throw new ConnectFailure(401, "connection_required");
  return token;
}

function positiveRevision(raw) {
  if (typeof raw !== "string" || !/^[1-9][0-9]{0,15}$/u.test(raw))
    throw new ConnectFailure(409, "context_changed");
  const value = Number(raw);
  if (!Number.isSafeInteger(value))
    throw new ConnectFailure(409, "context_changed");
  return value;
}

function validateContext(context) {
  if (!context || typeof context !== "object")
    throw new ConnectFailure(503, "invalid_authority_response");
  for (const field of [
    "deployment_id",
    "registration_id",
    "workstation_id",
    "configuration_epoch",
    "access_epoch",
  ]) {
    if (
      typeof context[field] !== "string" ||
      !context[field] ||
      context[field].length > 256 ||
      /[\p{Cc}]/u.test(context[field])
    )
      throw new ConnectFailure(503, "invalid_authority_response");
  }
  for (const field of [
    "registration_revision",
    "context_revision",
    "scope_revision",
  ]) {
    if (!Number.isSafeInteger(context[field]) || context[field] <= 0)
      throw new ConnectFailure(503, "invalid_authority_response");
  }
  let parsed;
  try {
    parsed = parseConnectId(context.connect_id);
  } catch {
    throw new ConnectFailure(503, "invalid_authority_response");
  }
  if (
    parsed.connectId !== context.connect_id ||
    parsed.workstationId !== context.workstation_id ||
    ["workspace", "workspace_id", "workspace_name", "workspace_marker"].some(
      (field) => Object.hasOwn(context, field),
    ) ||
    (context.workstation_ip != null &&
      (typeof context.workstation_ip !== "string" ||
        isIP(context.workstation_ip) === 0))
  )
    throw new ConnectFailure(503, "invalid_authority_response");
  // Explicit allowlist prevents accidental future secret-bearing fields entering RSC/JSON.
  return Object.fromEntries(
    [
      "deployment_id",
      "registration_id",
      "workstation_id",
      "workstation_ip",
      "connect_id",
      "registration_revision",
      "context_revision",
      "scope_revision",
      "configuration_epoch",
      "access_epoch",
    ].map((field) => [field, context[field]]),
  );
}

async function authorityRequest(path, body) {
  const upstream = await rustUpstream();
  if (!upstream.authorization)
    throw new ConnectFailure(503, "authority_unavailable");
  const target = new URL(`/api/v1/connect/sessions${path}`, upstream.origin);
  const bytes = JSON.stringify(body);
  return new Promise((resolve, reject) => {
    const send = target.protocol === "https:" ? httpsRequest : httpRequest;
    const request = send(
      target,
      {
        method: "POST",
        agent: directAgent(target),
        signal: AbortSignal.timeout(5000),
        headers: {
          host: target.host,
          authorization: upstream.authorization,
          "content-type": "application/json",
          "content-length": Buffer.byteLength(bytes),
        },
      },
      (response) => {
        const status = response.statusCode || 503;
        if (status < 200 || status >= 300) {
          response.resume();
          reject(
            new ConnectFailure(
              [401, 403, 409, 429].includes(status) ? status : 503,
              status === 409 ? "context_changed" : "connection_unavailable",
            ),
          );
          return;
        }
        let size = 0;
        const chunks = [];
        response.on("data", (chunk) => {
          size += chunk.length;
          if (size > 64 * 1024) {
            response.destroy();
            reject(new ConnectFailure(503, "invalid_authority_response"));
          } else chunks.push(chunk);
        });
        response.on("error", () =>
          reject(new ConnectFailure(503, "authority_unavailable")),
        );
        response.on("end", () => {
          try {
            resolve(JSON.parse(Buffer.concat(chunks).toString("utf8")));
          } catch {
            reject(new ConnectFailure(503, "invalid_authority_response"));
          }
        });
      },
    );
    request.on("error", () =>
      reject(new ConnectFailure(503, "authority_unavailable")),
    );
    request.end(bytes);
  });
}

/** Inject boundary transport/clock for deterministic fixtures; no runtime bypass flag. */
export function createConnectAuthority({
  request = authorityRequest,
  now = Date.now,
} = {}) {
  function validateSession(payload, token) {
    const expires = Date.parse(payload?.expires_at);
    if (!Number.isFinite(expires) || expires <= now())
      throw new ConnectFailure(401, "connection_expired");
    return {
      token,
      context: validateContext(payload.context),
      expires_at: payload.expires_at,
    };
  }
  return {
    async create(connectId) {
      let parsed;
      try {
        parsed = parseConnectId(connectId);
      } catch {
        throw new ConnectFailure(400, "invalid_connect_id");
      }
      const payload = await request("", { connect_id: parsed.connectId });
      if (
        typeof payload?.session_token !== "string" ||
        payload.session_token.length !== 64 ||
        !/^[a-f0-9]{64}$/u.test(payload.session_token)
      )
        throw new ConnectFailure(503, "invalid_authority_response");
      const session = validateSession(payload, payload.session_token);
      if (session.context.connect_id !== parsed.connectId)
        throw new ConnectFailure(503, "invalid_authority_response");
      return session;
    },
    async resolve(token, expectedRevision) {
      const payload = await request("/resolve", {
        session_token: token,
        ...(expectedRevision === undefined
          ? {}
          : { expected_context_revision: expectedRevision }),
      });
      const session = validateSession(payload, token);
      if (
        expectedRevision !== undefined &&
        session.context.context_revision !== expectedRevision
      )
        throw new ConnectFailure(409, "context_changed");
      return session;
    },
    async logout(token) {
      const payload = await request("/logout", { session_token: token });
      if (payload?.revoked !== true)
        throw new ConnectFailure(503, "invalid_authority_response");
    },
  };
}

export const connectAuthority = createConnectAuthority();

/** Used at protected render, REST/actions/export and upgrade boundaries. */
export async function requireConnection(
  request,
  { mutation = false, expectedRevision } = {},
) {
  const origin = await requireWebBoundary(request, mutation);
  const token = sessionTokenFromCookie(headerValue(request.headers, "cookie"));
  const rawExpected = headerValue(request.headers, EXPECTED_CONTEXT_HEADER);
  const revision =
    expectedRevision ??
    (rawExpected == null ? undefined : positiveRevision(rawExpected));
  if (mutation && revision === undefined)
    throw new ConnectFailure(409, "context_required");
  const session = await connectAuthority.resolve(token, revision);
  return { ...session, origin };
}

export function connectionProofHeaders(session) {
  return {
    "x-fleetworks-session": session.token,
    "x-fleetworks-context-revision": String(session.context.context_revision),
  };
}

export function publicConnection(session) {
  return { context: session.context, expires_at: session.expires_at };
}

function cookie(value, expires, origin) {
  return `${CONNECT_COOKIE}=${value}; Path=${configuredWebBasePath() || "/"}; HttpOnly; SameSite=Strict; ${new URL(origin).protocol === "https:" ? "Secure; " : ""}Expires=${new Date(expires).toUTCString()}`;
}

async function boundedJson(request) {
  if (
    request.headers.get("content-type")?.split(";")[0].trim().toLowerCase() !==
    "application/json"
  )
    throw new ConnectFailure(415, "json_required");
  const reader = request.body?.getReader();
  const chunks = [];
  let size = 0;
  if (reader) {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > 1024) {
        await reader.cancel();
        throw new ConnectFailure(413, "body_too_large");
      }
      chunks.push(value);
    }
  }
  try {
    return JSON.parse(Buffer.concat(chunks).toString("utf8"));
  } catch {
    throw new ConnectFailure(400, "invalid_json");
  }
}

export async function createConnectionResponse(request) {
  try {
    const origin = await requireWebBoundary(request, true);
    const body = await boundedJson(request);
    if (
      !body ||
      typeof body !== "object" ||
      Array.isArray(body) ||
      Object.keys(body).length !== 1 ||
      typeof body.connect_id !== "string"
    )
      throw new ConnectFailure(400, "invalid_connect_id");
    const session = await connectAuthority.create(body.connect_id);
    let previous;
    try {
      previous = sessionTokenFromCookie(headerValue(request.headers, "cookie"));
    } catch {
      /* Missing/invalid prior cookies grant no authority and need no reuse. */
    }
    if (previous && previous !== session.token) {
      try {
        await connectAuthority.logout(previous);
      } catch (error) {
        if (!(error instanceof ConnectFailure) || error.status !== 401)
          throw error;
      }
    }
    return Response.json(publicConnection(session), {
      headers: {
        ...CONNECT_RESPONSE_HEADERS,
        "set-cookie": cookie(session.token, session.expires_at, origin),
      },
    });
  } catch (error) {
    return connectFailureResponse(error);
  }
}

export async function currentConnectionResponse(request) {
  try {
    return Response.json(publicConnection(await requireConnection(request)), {
      headers: CONNECT_RESPONSE_HEADERS,
    });
  } catch (error) {
    return connectFailureResponse(error);
  }
}

export async function logoutConnectionResponse(request) {
  let origin;
  try {
    origin = await requireWebBoundary(request, true);
    const body = await boundedJson(request);
    if (
      !body ||
      typeof body !== "object" ||
      Array.isArray(body) ||
      Object.keys(body).length !== 0
    )
      throw new ConnectFailure(400, "invalid_json");
    const token = sessionTokenFromCookie(
      headerValue(request.headers, "cookie"),
    );
    await connectAuthority.logout(token);
    return Response.json(
      { disconnected: true },
      {
        headers: {
          ...CONNECT_RESPONSE_HEADERS,
          "set-cookie": cookie("", 0, origin),
        },
      },
    );
  } catch (error) {
    const response = connectFailureResponse(error);
    if (origin) response.headers.set("set-cookie", cookie("", 0, origin));
    return response;
  }
}
