import { request as httpRequest } from "node:http";
import { request as httpsRequest } from "node:https";
import { Readable } from "node:stream";
import { directAgent, rustUpstream } from "../server-origin.mjs";
import {
  connectFailureResponse,
  connectionProofHeaders,
  requireConnection,
  requireWebBoundary,
} from "../connect-session.mjs";
import type { ServerConnection } from "../connect-session.mjs";
/** @tag:api-proxy — Server-only Rust transport. Browser requests can never select its destination. */
const routes = new Map([
  ["health/ready", "GET"],
  ["capabilities", "GET"],
  ["host/metrics", "GET"],
  ["days", "GET"],
  ["actors/detail", "GET"],
  ["articles", "GET"],
  ["articles/counts", "GET"],
  ["articles/detail", "GET"],
  ["articles/related", "GET"],
  ["brief/ensure", "POST"],
  ["brief/reload", "POST"],
  ["brief/stages", "GET"],
  ["jobs", "GET"],
  ["jobs/detail", "GET"],
  ["jobs/retry", "POST"],
  ["topology", "GET"],
  ["topology/resolve", "POST"],
  ["topology/summary", "GET"],
  ["topology/summary/generate", "POST"],
]);
const headers = {
  "cache-control": "private, no-store, max-age=0",
  vary: "Cookie, Origin",
  "x-content-type-options": "nosniff",
};
const failure = (status: number, error: string) =>
  Response.json({ error }, { status, headers });

export async function proxyRust(
  request: Request,
  path: string[],
): Promise<Response> {
  const endpoint = path.join("/");
  const method = routes.get(endpoint);
  if (!method) return failure(404, "Unknown API route");
  if (request.method !== method) return failure(405, "Method not allowed");
  const url = new URL(request.url);
  if (
    [...url.searchParams.keys()].some((key) =>
      [
        "workspace",
        "workspace_id",
        "workspace_name",
        "workspace_marker",
      ].includes(key),
    )
  )
    return failure(
      400,
      "Legacy workspace scope is unsupported; reconnect with a workstation Connect ID",
    );
  try {
    await requireWebBoundary(request, method === "POST");
  } catch (error) {
    return connectFailureResponse(error);
  }
  let body: string | undefined;
  if (method === "POST") {
    if (
      request.headers
        .get("content-type")
        ?.split(";")[0]
        .trim()
        .toLowerCase() !== "application/json"
    )
      return failure(415, "JSON required");
    const reader = request.body?.getReader();
    const chunks: Uint8Array[] = [];
    let size = 0;
    if (reader) {
      while (true) {
        const { value, done } = await reader.read();
        if (done) break;
        size += value.byteLength;
        if (size > 16 * 1024) {
          await reader.cancel();
          return failure(413, "Request too large");
        }
        chunks.push(value);
      }
    }
    const data = new Uint8Array(size);
    let offset = 0;
    for (const chunk of chunks) {
      data.set(chunk, offset);
      offset += chunk.byteLength;
    }
    body = new TextDecoder().decode(data);
  }
  let session: ServerConnection;
  try {
    session = await requireConnection(request, { mutation: method === "POST" });
  } catch (error) {
    return connectFailureResponse(error);
  }
  try {
    const upstream = await rustUpstream();
    const target = new URL(`/api/v1/${endpoint}`, upstream.origin);
    target.search = url.search;
    const signal = AbortSignal.any([
      request.signal,
      AbortSignal.timeout(55000),
    ]);
    if (!upstream.authorization)
      return failure(503, "Connection authority unavailable");
    const response = await directRustRequest(
      target,
      method,
      body,
      signal,
      upstream.authorization,
      connectionProofHeaders(session),
    );
    return new Response(response.body, {
      status: response.status,
      headers: {
        ...headers,
        "content-type": "application/json; charset=utf-8",
      },
    });
  } catch {
    return failure(
      503,
      "Backend service unavailable; retry or inspect server diagnostics",
    );
  }
}

/** Use a dedicated direct HTTP agent when an upstream bearer is present. */
function directRustRequest(
  target: URL,
  method: string,
  body: string | undefined,
  signal: AbortSignal,
  authorization: string,
  contextHeaders: Record<string, string>,
): Promise<Response> {
  return new Promise((resolve, reject) => {
    const request = target.protocol === "https:" ? httpsRequest : httpRequest;
    const upstream = request(
      target,
      {
        method,
        signal,
        agent: directAgent(target),
        headers: {
          host: target.host,
          authorization,
          ...contextHeaders,
          ...(body === undefined ? {} : { "content-type": "application/json" }),
        },
      },
      (reply) => {
        const status = reply.statusCode;
        if (
          status === undefined ||
          status < 200 ||
          status > 599 ||
          (status >= 300 && status < 400)
        ) {
          reply.destroy();
          reject(new Error("Invalid Rust API response"));
          return;
        }
        const data =
          status === 204 || status === 205
            ? null
            : (Readable.toWeb(reply) as ReadableStream<Uint8Array>);
        resolve(new Response(data, { status }));
      },
    );
    upstream.once("error", reject);
    upstream.end(body);
  });
}
