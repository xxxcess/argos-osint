import { get } from "node:http";

const origin = new URL(process.env.FLEETWORKS_WEB_ORIGIN);
const request = get(
  "http://127.0.0.1:3000/api/v1/health/ready",
  { headers: { host: origin.host }, signal: AbortSignal.timeout(3000) },
  (response) => {
    response.resume();
    if (response.statusCode !== 200) process.exitCode = 1;
  },
);
request.on("error", () => {
  process.exitCode = 1;
});
