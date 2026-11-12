import { spawn } from "node:child_process";
import { once } from "node:events";
import { fileURLToPath } from "node:url";
import { startConnectGateway } from "./connect-gateway-fixture.mjs";

// The test harness owns infrastructure; the production client/server gains no bypass.
const gateway = await startConnectGateway();
const portIndex = process.argv.indexOf("--port");
const port = portIndex < 0 ? "3100" : process.argv[portIndex + 1];
const app = fileURLToPath(new URL("..", import.meta.url));
const child = spawn(
  process.execPath,
  [
    fileURLToPath(new URL("../server.mjs", import.meta.url)),
    "--port",
    port,
    ...(process.argv.includes("--dev") ? ["--dev"] : []),
  ],
  {
    cwd: app,
    stdio: "inherit",
    env: {
      ...process.env,
      FLEETWORKS_API_ORIGIN: gateway.origin,
      FLEETWORKS_API_TOKEN: gateway.apiToken,
      FLEETWORKS_WEB_ORIGIN: `http://127.0.0.1:${port}`,
      FLEETWORKS_WEB_BASE_PATH: "",
      FLEETWORKS_WEB_AUTHENTICATED_INGRESS: "",
      FLEETWORKS_WEB_INGRESS_TOKEN_FILE: "",
    },
  },
);
let stopping = false;
for (const signal of ["SIGINT", "SIGTERM"])
  process.on(signal, () => {
    if (!stopping) {
      stopping = true;
      child.kill(signal);
    }
  });
child.on("error", async () => {
  await gateway.close();
  process.exitCode = 1;
});
const [code, signal] = await once(child, "exit");
await gateway.close();
process.exitCode = code ?? (signal ? 1 : 0);
