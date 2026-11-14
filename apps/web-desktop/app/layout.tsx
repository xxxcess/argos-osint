import "./style.css";
import type { CSSProperties } from "react";
import { WEB_UI_SCALE } from "./ui-scale";
import { headers } from "next/headers";
import { redirect } from "next/navigation";
import { WebWorkspace } from "./workspace";
import { Inter, JetBrains_Mono } from "next/font/google";
import { configuredWebBasePath } from "../server-origin.mjs";
import {
  ConnectFailure,
  relativeWebPath,
  requireConnection,
  requireWebBoundary,
  safeReturnPath,
} from "../connect-session.mjs";
import type { RegisteredWebContext } from "@argos/api-client";

const inter = Inter({
  subsets: ["latin"],
  display: "swap",
  variable: "--intel-body",
});
const mono = JetBrains_Mono({
  subsets: ["latin"],
  display: "swap",
  variable: "--intel-mono",
});
export const metadata = {
  title: "Fleetworks · Intel",
  description: "Local intelligence bulletin and evidence workspace",
  icons: {
    icon: `${configuredWebBasePath()}/fleetworks-mark.svg`,
    apple: `${configuredWebBasePath()}/fleetworks-mark.svg`,
  },
};
export default async function Layout({
  children,
}: {
  children: React.ReactNode;
}) {
  // Request rendering lets Next attach the CSP nonce to every bootstrap script.
  const incoming = await headers();
  const path =
    incoming.get("x-fleetworks-request-path") || `${configuredWebBasePath()}/`;
  const method = incoming.get("x-fleetworks-request-method") || "GET";
  const request = { headers: incoming, url: path, method };
  const publicConnect = relativeWebPath(path.split("?")[0]) === "/connect";
  let connection: RegisteredWebContext | undefined;
  let expiresAt: string | undefined;
  let connectionError: string | undefined;
  let needsConnect = false;
  try {
    if (publicConnect) await requireWebBoundary(request);
    else {
      const session = await requireConnection(request, {
        mutation: !["GET", "HEAD"].includes(method),
      });
      connection = session.context;
      expiresAt = session.expires_at;
    }
  } catch (error) {
    if (error instanceof ConnectFailure && error.status === 401)
      needsConnect = true;
    else
      connectionError =
        error instanceof ConnectFailure
          ? error.message
          : "Connection authority unavailable; retry";
  }
  if (needsConnect)
    redirect(
      `${configuredWebBasePath()}/connect?return=${encodeURIComponent(safeReturnPath(path))}`,
    );
  return (
    <html
      lang="en"
      className={`${inter.variable} ${mono.variable}`}
      style={{ "--intel-ui-scale": WEB_UI_SCALE } as CSSProperties}
    >
      <body>
        {connection && expiresAt ? (
          <WebWorkspace
            connection={connection}
            expiresAt={expiresAt}
            basePath={configuredWebBasePath()}
          />
        ) : null}
        {connectionError ? (
          <main role="alert">
            <p>{connectionError}</p>
            <a href={`${configuredWebBasePath()}/connect`}>Reconnect</a>
          </main>
        ) : (
          children
        )}
      </body>
    </html>
  );
}
