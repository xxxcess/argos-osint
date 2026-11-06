import { rustOrigin } from "../server-origin.mjs";
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
  "cache-control": "no-store",
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
  const host = request.headers.get("host");
  let presented: URL;
  try {
    presented = new URL(`${url.protocol}//${host || url.host}`);
    if (
      !["127.0.0.1", "localhost"].includes(presented.hostname) ||
      presented.port !== url.port
    )
      return failure(403, "Host refused");
  } catch {
    return failure(403, "Host refused");
  }
  const origin = request.headers.get("origin");
  if (
    // Next can reconstruct request.url with its internal hostname. The
    // browser's origin must match the validated incoming Host instead.
    (origin && origin !== presented.origin) ||
    (!origin && request.headers.get("sec-fetch-site") === "cross-site")
  )
    return failure(403, "Origin refused");
  let body: string | undefined;
  if (method === "POST") {
    if (!request.headers.get("content-type")?.startsWith("application/json"))
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
  try {
    const target = new URL(`/api/v1/${endpoint}`, rustOrigin());
    target.search = url.search;
    const response = await fetch(target, {
      method,
      body,
      cache: "no-store",
      redirect: "error",
      headers: body === undefined ? {} : { "content-type": "application/json" },
      signal: AbortSignal.any([request.signal, AbortSignal.timeout(55000)]),
    });
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
      "Workspace service unavailable; retry or inspect server diagnostics",
    );
  }
}
