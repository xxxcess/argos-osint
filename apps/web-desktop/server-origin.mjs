import { open } from "node:fs/promises";
import { Agent as HttpAgent } from "node:http";
import { Agent as HttpsAgent } from "node:https";

const httpAgent = new HttpAgent({ keepAlive: true });
const httpsAgent = new HttpsAgent({ keepAlive: true });
const loopback = new Set(["127.0.0.1", "localhost"]);
const maxTokenBytes = 256;

function originUrl(raw) {
  let url;
  try {
    url = new URL(raw);
  } catch {
    throw new Error("Invalid server-side origin");
  }
  if (
    !["http:", "https:"].includes(url.protocol) ||
    !url.hostname ||
    url.username ||
    url.password ||
    url.pathname !== "/" ||
    url.search ||
    url.hash
  )
    throw new Error("Invalid server-side origin");
  return url;
}

/** Server-only configuration shared by the REST adapter and WebSocket relay. */
export function rustOrigin() {
  const network = process.env.FLEETWORKS_API_ORIGIN;
  if (network !== undefined) return originUrl(network).origin;
  const url = originUrl(
    process.env.ARGOS_RUST_API_ORIGIN || "http://127.0.0.1:8080",
  );
  if (url.protocol !== "http:" || !loopback.has(url.hostname))
    throw new Error("Invalid server-side Rust API origin");
  return url.origin;
}

async function readTokenFile(path) {
  if (!path) throw new Error("Missing Fleetworks API credential");
  try {
    const file = await open(path, "r");
    try {
      if (!(await file.stat()).isFile()) throw new Error("Invalid token file");
      const buffer = Buffer.alloc(maxTokenBytes + 3);
      let length = 0;
      while (length < buffer.length) {
        const { bytesRead } = await file.read(
          buffer,
          length,
          buffer.length - length,
          null,
        );
        if (!bytesRead) break;
        length += bytesRead;
      }
      if (length === buffer.length) throw new Error("Invalid token file");
      return buffer.subarray(0, length).toString("utf8").replace(/\r?\n$/u, "");
    } finally {
      await file.close();
    }
  } catch {
    throw new Error("Could not read Fleetworks API credential");
  }
}

export async function rustUpstream() {
  const origin = rustOrigin();
  if (process.env.FLEETWORKS_API_ORIGIN === undefined) return { origin };
  const token =
    process.env.FLEETWORKS_API_TOKEN ??
    (await readTokenFile(process.env.FLEETWORKS_API_TOKEN_FILE));
  if (
    token.length < 32 ||
    token.length > maxTokenBytes ||
    !/^[A-Za-z0-9._~+/-]+=*(?![\s\S])/u.test(token)
  )
    throw new Error("Invalid Fleetworks API credential");
  return { origin, authorization: `Bearer ${token}` };
}

/** Dedicated direct agents avoid inheriting any process-global proxy agent. */
export function directAgent(target) {
  return ["https:", "wss:"].includes(target.protocol) ? httpsAgent : httpAgent;
}

export function configuredWebOrigin() {
  const raw = process.env.FLEETWORKS_WEB_ORIGIN;
  if (raw === undefined) return undefined;
  const url = originUrl(raw);
  if (
    process.env.FLEETWORKS_API_ORIGIN === undefined ||
    url.protocol !== "http:" ||
    !loopback.has(url.hostname)
  )
    throw new Error("Invalid Fleetworks web origin");
  return url.origin;
}

export function incomingWebOrigin(host, protocol, legacyPort) {
  if (!host || /[/\\@?#\s,]/u.test(host)) throw new Error("Host refused");
  const url = originUrl(`${protocol}//${host}`);
  const configured = configuredWebOrigin();
  const port = Number(url.port || (url.protocol === "https:" ? 443 : 80));
  if (
    !loopback.has(url.hostname) ||
    (configured !== undefined
      ? url.origin !== configured
      : legacyPort !== undefined && port !== legacyPort)
  )
    throw new Error("Host refused");
  return url.origin;
}
