import { NextRequest, NextResponse } from "next/server";

/** Per-request script nonces protect the server-rendered workspace. */
export function proxy(request: NextRequest) {
  const host = request.headers.get("host") || request.nextUrl.host;
  try {
    if (
      !["localhost", "127.0.0.1"].includes(new URL(`http://${host}`).hostname)
    )
      return new NextResponse("Host refused", { status: 403 });
  } catch {
    return new NextResponse("Host refused", { status: 403 });
  }
  const nonce = Buffer.from(crypto.randomUUID()).toString("base64");
  const isDev = process.env.NODE_ENV === "development";
  const csp = `default-src 'none'; script-src 'self' 'nonce-${nonce}' 'strict-dynamic'${isDev ? " 'unsafe-eval'" : ""}; style-src 'self' 'unsafe-inline'; img-src 'self' https: data:; font-src 'self'; connect-src 'self' ws://${request.nextUrl.host}${isDev ? " ws://127.0.0.1:* ws://localhost:*" : ""}; base-uri 'none'; frame-ancestors 'none'; form-action 'none'`;
  const headers = new Headers(request.headers);
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
