import { get } from "node:http";

const origin = new URL(process.env.FLEETWORKS_WEB_ORIGIN);
const request = get(
  // Process health must not need an arbitrary registered user's cookie. The
  // packaged smoke proves protected readiness through a genuine Connect login.
  "http://127.0.0.1:3000/connect",
  { headers: { host: origin.host }, signal: AbortSignal.timeout(3000) },
  (response) => {
    response.resume();
    if (response.statusCode !== 200) process.exitCode = 1;
  },
);
request.on("error", () => {
  process.exitCode = 1;
});
