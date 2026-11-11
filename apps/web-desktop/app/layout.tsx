import "./style.css";
import type { CSSProperties } from "react";
import { WEB_UI_SCALE } from "./ui-scale";
import { headers } from "next/headers";
import { WebWorkspace } from "./workspace";
import { Inter, JetBrains_Mono } from "next/font/google";

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
  icons: { icon: "/fleetworks-mark.svg", apple: "/fleetworks-mark.svg" },
};
export default async function Layout({
  children,
}: {
  children: React.ReactNode;
}) {
  // Request rendering lets Next attach the CSP nonce to every bootstrap script.
  await headers();
  return (
    <html
      lang="en"
      className={`${inter.variable} ${mono.variable}`}
      style={{ "--intel-ui-scale": WEB_UI_SCALE } as CSSProperties}
    >
      <body>
        <WebWorkspace />
        {children}
      </body>
    </html>
  );
}
