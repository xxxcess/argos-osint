import type { BackendSettings } from "@argos/api-client";

/** The browser only knows Next's same-origin API. Rust configuration is server-only. */
export function createWebBackendSettings(basePath = ""): BackendSettings {
  return {
    subscribe: () => {
      const url = new URL(location.href);
      if (url.searchParams.has("api")) {
        url.searchParams.delete("api");
        history.replaceState(null, "", url);
      }
      try {
        localStorage.removeItem("argos.backend");
      } catch {
        /* Blocked storage still permits the same-origin API. */
      }
      return () => {};
    },
    getSnapshot: () => basePath,
    getServerSnapshot: () => basePath,
    setBase: () => {
      throw new Error("Workspace service configuration belongs to the server");
    },
    createRequestId: () => crypto.randomUUID(),
  };
}

export const webBackendSettings = createWebBackendSettings();
