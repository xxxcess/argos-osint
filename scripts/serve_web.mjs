// Local-only static export host. Never use next start for output: export.
import { createServer } from "node:http";
import { readFile, stat, readdir } from "node:fs/promises";
import { resolve, extname, sep } from "node:path";
import { createHash } from "node:crypto";
const root = resolve(import.meta.dirname, "../apps/web-desktop/out");
const index = process.argv.indexOf("--port");
const port = Number(index < 0 ? 3000 : process.argv[index + 1]);
if (!Number.isInteger(port) || port < 1 || port > 65535)
  throw new Error("Invalid port");
const types = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json",
  ".txt": "text/plain",
  ".svg": "image/svg+xml",
  ".ico": "image/x-icon",
  ".png": "image/png",
  ".woff2": "font/woff2",
};
// Hash the immutable Next inline bootstrap; no arbitrary inline scripts.
const scriptHashes = new Set();
async function collectHashes(dir) {
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const path = resolve(dir, entry.name);
    if (entry.isDirectory()) await collectHashes(path);
    else if (entry.name.endsWith(".html")) {
      const html = await readFile(path, "utf8");
      for (const [, attributes, script] of html.matchAll(
        /<script([^>]*)>([\s\S]*?)<\/script>/g,
      )) {
        if (!/\bsrc=/.test(attributes) && script)
          scriptHashes.add(
            `'sha256-${createHash("sha256").update(script).digest("base64")}'`,
          );
      }
    }
  }
}
await collectHashes(root);
export const csp = `default-src 'none'; script-src 'self' ${[...scriptHashes].join(" ")}; style-src 'self' 'unsafe-inline'; img-src 'self' https: data:; font-src 'self'; connect-src http://127.0.0.1:* http://localhost:*; base-uri 'none'; frame-ancestors 'none'; form-action 'none'`;
const server = createServer(async (req, res) => {
  const started = performance.now();
  let finished = false;
  res.once("finish", () => {
    finished = true;
    // Access logs deliberately exclude query strings, headers and bodies.
    let path = "[invalid path]";
    try {
      path = new URL(req.url, `http://127.0.0.1:${port}`).pathname;
    } catch {}
    console.log(
      `${req.method} ${path} ${res.statusCode} ${Math.round(performance.now() - started)} ms`,
    );
  });
  res.once("close", () => {
    if (!finished && !res.writableEnded) {
      let path = "[invalid path]";
      try {
        path = new URL(req.url, `http://127.0.0.1:${port}`).pathname;
      } catch {}
      console.log(
        `${req.method} ${path} ${res.statusCode} ABORTED ${Math.round(
          performance.now() - started,
        )} ms`,
      );
    }
  });
  const hosts = [
    `localhost:${port}`,
    `127.0.0.1:${port}`,
    ...(port === 80 ? ["localhost", "127.0.0.1"] : []),
  ];
  if (!hosts.includes(req.headers.host)) {
    res.writeHead(403).end();
    return;
  }
  res.setHeader("Content-Security-Policy", csp);
  res.setHeader("X-Content-Type-Options", "nosniff");
  res.setHeader("Referrer-Policy", "no-referrer");
  res.setHeader("Cache-Control", "no-store");
  if (!["GET", "HEAD"].includes(req.method)) {
    res.writeHead(405).end();
    return;
  }
  try {
    const path = decodeURIComponent(
      new URL(req.url, `http://127.0.0.1:${port}`).pathname,
    );
    let file = resolve(root, "." + path);
    if (file !== root && !file.startsWith(root + sep)) {
      res.writeHead(403).end();
      return;
    }
    if ((await stat(file)).isDirectory()) file = resolve(file, "index.html");
    const bytes = await readFile(file);
    res.setHeader(
      "Content-Type",
      types[extname(file)] || "application/octet-stream",
    );
    res.writeHead(200).end(req.method === "HEAD" ? undefined : bytes);
  } catch {
    res.writeHead(404).end("Not found");
  }
});
server.listen(port, "127.0.0.1", () =>
  console.log(`Static Intel: http://localhost:${port}`),
);
