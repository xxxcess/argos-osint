import "@testing-library/jest-dom";
import { beforeAll, afterAll, afterEach, vi } from "vitest";
import { setupServer } from "msw/node";
import { http, HttpResponse } from "msw";

// Base URL is configurable so suites can point at another loopback origin
// without editing handlers (see `ARGOS_TEST_BASE`). api-client only accepts
// loopback HTTP origins, so the default keeps matching the local API.
export const testBase = (
  process.env.ARGOS_TEST_BASE ?? "http://127.0.0.1:8080"
).replace(/\/+$/, "");

// jsdom does not implement these browser APIs that React Native Web's layout
// and appearance hooks may touch. Provide honest no-op stubs.
if (
  typeof globalThis.window !== "undefined" &&
  typeof window.matchMedia !== "function"
) {
  const matchMediaStub = (query: string): MediaQueryList => {
    const listeners = new Set<(e: MediaQueryListEvent) => void>();
    return {
      matches: false,
      media: query,
      onchange: null,
      addListener: (cb: (e: MediaQueryListEvent) => void) => {
        listeners.add(cb);
      },
      removeListener: (cb: (e: MediaQueryListEvent) => void) => {
        listeners.delete(cb);
      },
      addEventListener: (_t: string, cb: (e: MediaQueryListEvent) => void) => {
        listeners.add(cb);
      },
      removeEventListener: (
        _t: string,
        cb: (e: MediaQueryListEvent) => void,
      ) => {
        listeners.delete(cb);
      },
      dispatchEvent: () => false,
    } as unknown as MediaQueryList;
  };
  window.matchMedia = matchMediaStub as unknown as typeof window.matchMedia;
}

if (typeof globalThis.ResizeObserver === "undefined") {
  class ResizeObserverStub {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  globalThis.ResizeObserver =
    ResizeObserverStub as unknown as typeof globalThis.ResizeObserver;
}

// jsdom + MSW limitation: passing ANY AbortSignal to `fetch` never resolves
// (mswjs/interceptors + jsdom signal identity mismatch). Component code still
// passes timeout/unmount signals, so wrap fetch at the environment boundary:
// the signal is stripped for transport, but abort is observed so in-flight
// requests reject with a proper AbortError if the signal fires.
// Install after `mswServer.listen` so the captured fetch is MSW-intercepted.
let installedFetchWrapper = false;
function installFetchWrapper() {
  if (installedFetchWrapper || typeof window === "undefined") return;
  installedFetchWrapper = true;
  const envFetch = window.fetch.bind(window);
  const observedFetch: typeof fetch = (input, init) => {
    const signal = init?.signal;
    if (!signal) return envFetch(input, init);
    return new Promise((resolve, reject) => {
      const onAbort = () =>
        reject(new DOMException("The operation was aborted.", "AbortError"));
      if (signal.aborted) {
        onAbort();
        return;
      }
      signal.addEventListener("abort", onAbort, { once: true });
      envFetch(input, { ...init, signal: undefined as never })
        .then((res) => {
          signal.removeEventListener("abort", onAbort);
          resolve(res);
        })
        .catch((err) => {
          signal.removeEventListener("abort", onAbort);
          reject(err);
        });
    });
  };
  window.fetch = observedFetch;
}

// MSW server for API mocking; all handlers use the configurable base URL.
// Default handlers can be overridden per test with `mswServer.use(...)`.
const api = (path: string) => `${testBase}/api/v1/${path}`;
export const mswServer = setupServer(
  http.get(api("health/ready"), () => {
    return HttpResponse.json({
      ready: false,
      storage: false,
      service: "argos-intel",
      api_version: 1,
      instance_id: "test-instance",
    });
  }),
  http.get(api("days"), () => {
    return HttpResponse.json({ days: [] });
  }),
  http.get(api("articles"), () => {
    return HttpResponse.json({ rows: [], next_cursor: null, revision: "" });
  }),
  http.post(api("brief/ensure"), () => {
    return HttpResponse.json({ job_id: "test-job" });
  }),
  http.post(api("brief/reload"), () => {
    return HttpResponse.json({ job_id: "test-job" });
  }),
  http.get(api("articles/detail*"), () => {
    return HttpResponse.json({
      article: {
        id: "test",
        run_id: "test",
        title: "Test Article",
        description: "Test description",
        source_name: "Test Source",
        source_domain: "test.com",
        published_at: "2024-01-15T10:00:00Z",
        category: "geopolitical",
        image_url: "",
        url: "http://test.com",
        author: "Test Author",
      },
      source_revision: "",
      actors: [],
      claims: [],
      body: null,
      output: null,
      lifecycle: "complete",
      status: "done",
      revision: "",
      publication: null,
      relations: [],
      links: [],
      confidence: null,
      indexing: null,
    });
  }),
  http.get(api("topology"), () => {
    return HttpResponse.json({
      root_label: "Test",
      nodes: [],
      edges: [],
      findings: { gaps: [] },
      has_continuation: false,
    });
  }),
  http.get(api("articles/related*"), () => {
    return HttpResponse.json({ rows: [], next_cursor: null });
  }),
);

// Global test setup/teardown
beforeAll(() => {
  mswServer.listen({ onUnhandledRequest: "error" });
  installFetchWrapper();
});
afterAll(() => mswServer.close());
afterEach(() => {
  mswServer.resetHandlers();
  vi.clearAllMocks();
});
