import { NextRequest, NextResponse } from "next/server";
import { incomingWebOrigin } from "./server-origin.mjs";
import { configuredWebBasePath } from "./server-origin.mjs";

/** Per-request script nonces protect the server-rendered workspace. */
export function proxy(request: NextRequest) {
  const host = request.headers.get("host") || request.nextUrl.host;
  let origin: URL;
  try {
    origin = new URL(incomingWebOrigin(host, request.nextUrl.protocol));
  } catch {
    return new NextResponse("Host refused", { status: 403 });
  }
  const nonce = Buffer.from(crypto.randomUUID()).toString("base64");
  const isDev = process.env.NODE_ENV === "development";
  const wsProtocol = origin.protocol === "https:" ? "wss:" : "ws:";
  const csp = `default-src 'none'; script-src 'self' 'nonce-${nonce}' 'strict-dynamic'${isDev ? " 'unsafe-eval'" : ""}; style-src 'self' 'unsafe-inline'; img-src 'self' http: https: data: blob:; font-src 'self'; connect-src 'self' ${wsProtocol}//${origin.host}${isDev ? " ws://127.0.0.1:* ws://localhost:*" : ""}; base-uri 'none'; frame-ancestors 'none'; form-action 'none'`;
  const headers = new Headers(request.headers);
  for (const name of [...headers.keys()]) {
    if (
      name === "x-fleetworks-session" ||
      name.startsWith("x-fleetworks-context-") ||
      name.startsWith("x-fleetworks-origin-") ||
      name.startsWith("x-fleetworks-request-")
    )
      headers.delete(name);
  }
  const basePath = configuredWebBasePath();
  const pathname = request.nextUrl.pathname;
  const fullPath =
    basePath && !(pathname === basePath || pathname.startsWith(`${basePath}/`))
      ? `${basePath}${pathname}`
      : pathname;
  headers.set(
    "x-fleetworks-request-path",
    `${fullPath}${request.nextUrl.search}`,
  );
  headers.set("x-fleetworks-request-method", request.method);
  headers.set("x-nonce", nonce);
  headers.set("Content-Security-Policy", csp);
  const response = NextResponse.next({ request: { headers } });
  response.headers.set("Content-Security-Policy", csp);
  response.headers.set("X-Content-Type-Options", "nosniff");
  response.headers.set("Referrer-Policy", "no-referrer");
  response.headers.set("Cache-Control", "no-store");
  return response;
}
export const config = {
  matcher: ["/((?!api|_next/static|_next/image|favicon.ico).*)"],
};
