import {
  DEFAULT_BACKEND,
  safeBackend,
  type BackendSettings,
} from "@argos/api-client";

// Browser state is initialized only by the client snapshot. The static export
// always renders DEFAULT_BACKEND, then useSyncExternalStore hydrates the choice.
let selected: string | undefined;
const listeners = new Set<() => void>();

export const webBackendSettings: BackendSettings = {
  subscribe(onChange) {
    listeners.add(onChange);
    return () => {
      listeners.delete(onChange);
    };
  },
  getSnapshot() {
    if (selected === undefined) {
      selected = DEFAULT_BACKEND;
      try {
        selected = safeBackend(
          new URLSearchParams(location.search).get("api") ||
            localStorage.getItem("argos.backend") ||
            DEFAULT_BACKEND,
        );
      } catch {
        // Invalid/unavailable persisted settings keep the loopback default.
      }
    }
    return selected;
  },
  getServerSnapshot: () => DEFAULT_BACKEND,
  setBase(base) {
    selected = base;
    try {
      localStorage.setItem("argos.backend", base);
    } catch {
      // A blocked browser store must not prevent a connection for this session.
    }
    for (const onChange of listeners) onChange();
  },
  createRequestId: () => crypto.randomUUID(),
};
