/** Server-only configuration shared by the REST adapter and WebSocket relay. */
export function rustOrigin() {
  const url = new URL(
    process.env.ARGOS_RUST_API_ORIGIN || "http://127.0.0.1:8080",
  );
  if (
    url.protocol !== "http:" ||
    !["127.0.0.1", "localhost"].includes(url.hostname) ||
    url.username ||
    url.password ||
    url.pathname !== "/" ||
    url.search ||
    url.hash
  )
    throw new Error("Invalid server-side Rust API origin");
  return url.origin;
}
