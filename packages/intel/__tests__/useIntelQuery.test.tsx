import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor, act } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import {
  BackendProvider,
  DEFAULT_BACKEND,
  ApiError,
  keyFor,
  safeBackend,
  useAction,
  useArticles,
  useBackend,
  useActorDossier,
  useRelated,
  useRead,
  useTopology,
  type Article,
  type BackendSettings,
} from "@argos/api-client";
import { mswServer, testBase } from "../../../vitest.setup";

// In-memory persistence adapter standing in for the browser adapter
// (localStorage lives in apps/web-desktop, not in @argos/api-client).
let currentBase = testBase;
const subscribers = new Set<() => void>();
const settings: BackendSettings = {
  subscribe: (onChange) => {
    subscribers.add(onChange);
    return () => {
      subscribers.delete(onChange);
    };
  },
  getSnapshot: () => currentBase,
  getServerSnapshot: () => currentBase,
  setBase: vi.fn((base: string) => {
    currentBase = base;
    subscribers.forEach((fn) => fn());
  }),
  createRequestId: () => "test-request-id",
};

const readyHealth = {
  ready: true,
  storage: true,
  service: "argos-intel",
  api_version: 1,
  instance_id: "inst-test",
};

const articleRow: Article = {
  id: "a-1",
  run_id: "run-9",
  title: "Satellite imagery shifts",
  description: "New orbital pass over the corridor.",
  source_name: "OSINT Digest",
  source_domain: "digest.example",
  published_at: "2026-10-29T08:00:00Z",
  category: "signals",
  image_url: "",
  url: "https://digest.example/shifts",
  author: "Corps Analyst",
};

// Throws until the observed backend reports connected (retry:1 delays settle).
const waitForConnected = (connected: () => boolean) =>
  waitFor(
    () => {
      if (!connected()) throw new Error("not connected yet");
    },
    { timeout: 5000 },
  );

// Wrapper providing the BackendProvider around rendered hooks. The provider
// owns its own QueryClient (retry:1 on queries) — the same as production.
function providerWrapper() {
  return function Wrapper({ children }: { children?: React.ReactNode }) {
    return <BackendProvider settings={settings}>{children}</BackendProvider>;
  };
}

beforeEach(() => {
  currentBase = testBase;
});

describe("useBackend (real connection state)", () => {
  it("reports connected once the health contract passes", async () => {
    mswServer.use(
      http.get(`${testBase}/api/v1/health/ready`, () =>
        HttpResponse.json(readyHealth),
      ),
    );
    const { result } = renderHook(() => useBackend(), {
      wrapper: providerWrapper(),
    });
    expect(result.current.connected).toBe(false);
    await waitForConnected(() => result.current.connected);
    await waitFor(() => expect(result.current.connected).toBe(true));
    expect(result.current.base).toBe(testBase);
    expect(result.current.instance).toBe("inst-test");
    expect(typeof result.current.retry).toBe("function");
    expect(typeof result.current.setBase).toBe("function");
  });

  it("stays offline when health fails the service contract", async () => {
    void waitForConnected;
    mswServer.use(
      http.get(`${testBase}/api/v1/health/ready`, () =>
        // Ready flag false: server up, storage unavailable.
        HttpResponse.json({ ...readyHealth, ready: false, storage: false }),
      ),
    );
    const { result } = renderHook(() => useBackend(), {
      wrapper: providerWrapper(),
    });
    // The health fetch settles despite retry:false — busy flag clears.
    await waitFor(() => expect(result.current.connecting).toBe(false), {
      timeout: 5000,
    });
    expect(result.current.connected).toBe(false);
  });

  it("surfaces the DEFAULT_BACKEND constant", () => {
    expect(DEFAULT_BACKEND).toBe("");
  });
});

describe("safeBackend (real endpoint validation)", () => {
  it("normalizes an accepted loopback HTTP origin", () => {
    expect(safeBackend("http://127.0.0.1:8080")).toBe("http://127.0.0.1:8080");
    expect(safeBackend("http://localhost:7780/")).toBe("http://localhost:7780");
  });

  it.each([
    "https://127.0.0.1:8080",
    "http://argos.internal:8080",
    "http://user:pass@127.0.0.1:8080",
    "http://127.0.0.1:8080/api",
    "http://127.0.0.1:8080/?a=1",
    "http://127.0.0.1:8080/#frag",
  ])("rejects an unsafe origin (%s)", (raw) => {
    expect(() => safeBackend(raw)).toThrow(/loopback HTTP origin/);
  });
});

describe("useRead (real query over MSW)", () => {
  it("fetches the day index from the connected backend", async () => {
    mswServer.use(
      http.get(`${testBase}/api/v1/health/ready`, () =>
        HttpResponse.json(readyHealth),
      ),
      http.get(`${testBase}/api/v1/dashboard/days`, () =>
        HttpResponse.json({ days: ["2026-10-29"], revision: "rev-1" }),
      ),
    );
    const { result } = renderHook(
      () => useRead<{ days: string[]; revision: string }>("dashboard/days"),
      { wrapper: providerWrapper() },
    );
    await waitFor(() => expect(result.current.isSuccess).toBe(true), {
      timeout: 5000,
    });
    expect(result.current.data?.days).toEqual(["2026-10-29"]);
  });

  it("propagates ApiError status and message through the hook", async () => {
    mswServer.use(
      http.get(`${testBase}/api/v1/health/ready`, () =>
        HttpResponse.json(readyHealth),
      ),
      http.get(`${testBase}/api/v1/dashboard/days`, () =>
        HttpResponse.json({ error: "storage cyclic" }, { status: 503 }),
      ),
    );
    const { result } = renderHook(
      () => useRead<{ days: string[]; revision: string }>("dashboard/days"),
      { wrapper: providerWrapper() },
    );
    await waitFor(() => expect(result.current.isError).toBe(true), {
      timeout: 5000,
    });
    expect(result.current.error).toBeInstanceOf(ApiError);
    expect((result.current.error as ApiError).status).toBe(503);
    expect((result.current.error as ApiError).message).toBe("storage cyclic");
  });
});

describe("useArticles (real paginated feed)", () => {
  const days = { days: ["2026-10-29"], revision: "rev-1" };

  it("returns the first page and exposes the key contract", async () => {
    mswServer.use(
      http.get(`${testBase}/api/v1/health/ready`, () =>
        HttpResponse.json(readyHealth),
      ),
      http.get(`${testBase}/api/v1/dashboard/days`, () =>
        HttpResponse.json(days),
      ),
      http.get(`${testBase}/api/v1/articles`, ({ request }) => {
        const url = new URL(request.url);
        expect(url.searchParams.get("category")).toBe("signals");
        expect(url.searchParams.get("day")).toBe("2026-10-29");
        return HttpResponse.json({
          rows: [articleRow],
          next_cursor: null,
          revision: "rev-1",
        });
      }),
    );
    const { result } = renderHook(
      () =>
        useArticles("signals", "2026-10-29", "") as unknown as {
          data: { pages: { rows: Article[] }[] } | undefined;
          isSuccess: boolean;
        },
      { wrapper: providerWrapper() },
    );
    await waitFor(() => expect(result.current.isSuccess).toBe(true), {
      timeout: 5000,
    });
    expect(result.current.data?.pages[0]?.rows).toEqual([articleRow]);
    expect(keyFor(articleRow)).toBe("run=run-9&id=a-1");
  });

  it(
    "paginates through the cursor until the feed is exhausted",
    { timeout: 20000 },
    async () => {
      const page2 = {
        id: "a-2",
        run_id: "run-9",
        title: "Second page row",
        description: "",
        source_name: "OSINT Digest",
        source_domain: "digest.example",
        published_at: "2026-10-29T09:00:00Z",
        category: "signals",
        image_url: "",
        url: "https://digest.example/second",
        author: "Corps Analyst",
      } as Article;
      mswServer.use(
        http.get(`${testBase}/api/v1/health/ready`, () =>
          HttpResponse.json(readyHealth),
        ),
        http.get(`${testBase}/api/v1/dashboard/days`, () =>
          HttpResponse.json(days),
        ),
        http.get(`${testBase}/api/v1/articles`, ({ request }) => {
          const cursor = new URL(request.url).searchParams.get("cursor");
          return HttpResponse.json(
            cursor === null || cursor === ""
              ? { rows: [articleRow], next_cursor: "c2", revision: "rev-1" }
              : { rows: [page2], next_cursor: null, revision: "rev-1" },
          );
        }),
      );
      const { result } = renderHook(
        () => useArticles("signals", "2026-10-29", ""),
        { wrapper: providerWrapper() },
      );
      await waitFor(() => expect(result.current.isSuccess).toBe(true), {
        timeout: 5000,
      });
      const initialPages = result.current.data?.pages?.length ?? 0;
      await result.current.fetchNextPage();
      // Wait for the pages array to grow (React Query updates cache async)
      await waitFor(
        () =>
          expect(result.current.data?.pages?.length ?? 0).toBeGreaterThan(
            initialPages,
          ),
        { timeout: 3000 },
      );
      await waitFor(() => expect(result.current.data?.pages).toHaveLength(2), {
        timeout: 1500,
      });
      expect(result.current.data?.pages[1]?.rows[0].id).toBe("a-2");
    },
  );

  it("does not fetch articles without a selected day even after the backend is connected", async () => {
    let reads = 0;
    mswServer.use(
      http.get(`${testBase}/api/v1/health/ready`, () =>
        HttpResponse.json(readyHealth),
      ),
      http.get(`${testBase}/api/v1/articles`, () => {
        reads++;
        return HttpResponse.json({ rows: [] });
      }),
    );
    const { result } = renderHook(
      () => ({
        connected: useBackend().connected,
        query: useArticles("signals", "", ""),
      }),
      { wrapper: providerWrapper() },
    );
    await waitForConnected(() => result.current.connected);
    expect(result.current.query.fetchStatus).toBe("idle");
    expect(result.current.query.data).toBeUndefined();
    expect(reads).toBe(0);
  });
});

describe("useAction (real mutation over MSW)", () => {
  it("posts the brief body and invalidates instance queries", async () => {
    const seenBodies: unknown[] = [];
    mswServer.use(
      http.get(`${testBase}/api/v1/health/ready`, () =>
        HttpResponse.json(readyHealth),
      ),
      http.get(`${testBase}/api/v1/dashboard/days`, () =>
        HttpResponse.json({ days: [], revision: "rev-1" }),
      ),
      http.post(`${testBase}/api/v1/articles/read`, async ({ request }) => {
        seenBodies.push(await request.json());
        return HttpResponse.json({ status: "start_fetch" });
      }),
    );
    const { result } = renderHook(
      () => ({
        connected: useBackend().connected,
        action: useAction("articles/read"),
      }),
      { wrapper: providerWrapper() },
    );
    await waitForConnected(() => result.current.connected);
    await act(
      async () =>
        void (await result.current.action.mutateAsync({
          run: articleRow.run_id,
          id: articleRow.id,
        })),
    );
    await waitFor(() => expect(result.current.action.isSuccess).toBe(true), {
      timeout: 5000,
    });
    expect(result.current.action.data).toEqual({ status: "start_fetch" });
    expect(seenBodies).toEqual([{ run: "run-9", id: "a-1" }]);
  });

  it("refuses to mutate while the API is offline", async () => {
    mswServer.use(
      http.get(`${testBase}/api/v1/health/ready`, () =>
        // health fetched with signal → wrapper applies; service mismatch keeps
        // the connection offline without relying on request failure.
        HttpResponse.json({
          ready: true,
          storage: true,
          service: "other-service",
          api_version: 1,
          instance_id: "x",
        }),
      ),
    );
    const { result } = renderHook(
      () => ({
        connected: useBackend().connected,
        action: useAction("articles/read"),
      }),
      { wrapper: providerWrapper() },
    );
    await waitFor(() => expect(result.current.connected).toBe(false), {
      timeout: 5000,
    });
    await act(async () => {
      const outcome = await result.current.action.mutateAsync({}).then(
        () => null,
        (error: Error) => error,
      );
      expect(outcome?.message).toBe("API offline");
    });
    await waitFor(() => expect(result.current.action.isError).toBe(true));
  });
});

describe("useTopology (real GET over MSW)", () => {
  it("loads topology from one Brain memory ID in a GET query", async () => {
    const bodies: unknown[] = [];
    mswServer.use(
      http.get(`${testBase}/api/v1/health/ready`, () =>
        HttpResponse.json(readyHealth),
      ),
      http.get(`${testBase}/api/v1/topology`, ({ request }) => {
        bodies.push(new URL(request.url).searchParams.get("seed"));
        return HttpResponse.json({
          root_label: "Evidence topology",
          nodes: [
            {
              id: "n1",
              label: "Actor One",
              kind: "actor",
              detail: "d",
              topology_seed: null,
            },
          ],
          edges: [],
          findings: { gaps: [] },
          has_continuation: false,
        });
      }),
    );
    const { result } = renderHook(
      () => ({
        connected: useBackend().connected,
        topo: useTopology("a-1"),
      }),
      { wrapper: providerWrapper() },
    );
    await waitForConnected(() => result.current.connected);
    await waitFor(() => expect(result.current.topo.isSuccess).toBe(true), {
      timeout: 5000,
    });
    expect(result.current.topo.data?.root_label).toBe("Evidence topology");
    expect(bodies[0]).toBe("a-1");
  });
});

describe("useRelated (real graph page over MSW)", () => {
  it("requests related rows keyed by the article identity", async () => {
    mswServer.use(
      http.get(`${testBase}/api/v1/health/ready`, () =>
        HttpResponse.json(readyHealth),
      ),
      http.get(`${testBase}/api/v1/articles/related`, ({ request }) => {
        const query = new URL(request.url).searchParams;
        expect(query.get("run")).toBe(articleRow.run_id);
        expect(query.get("id")).toBe(articleRow.id);
        return HttpResponse.json({
          rows: [
            { article: articleRow, basis: ["same day"], tentative: false },
          ],
          next_cursor: null,
        });
      }),
    );
    const { result } = renderHook(() => useRelated(articleRow), {
      wrapper: providerWrapper(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true), {
      timeout: 5000,
    });
    expect(result.current.data?.pages[0]?.rows[0]?.basis).toEqual(["same day"]);
    expect(result.current.data?.pages[0]?.rows[0]?.tentative).toBe(false);
  });
});

describe("query regression boundaries", () => {
  it("keeps retained data after a 408 refresh failure and recovers on deliberate retry", async () => {
    let failed = false;
    let revision = "retained";
    mswServer.use(
      http.get(`${testBase}/api/v1/health/ready`, () =>
        HttpResponse.json(readyHealth),
      ),
      http.get(`${testBase}/api/v1/articles/detail`, () =>
        failed
          ? HttpResponse.json({ error: "Request timed out" }, { status: 408 })
          : HttpResponse.json({ revision }),
      ),
    );
    const { result } = renderHook(
      () => {
        const query = useRead<{ revision: string }>(
          "articles/detail?run=r&id=a",
        );
        // Consume error/loading during render, as the real screen does, so
        // Query's property tracking subscribes this observer to those changes.
        return {
          data: query.data,
          error: query.error,
          isLoading: query.isLoading,
          refetch: query.refetch,
        };
      },
      { wrapper: providerWrapper() },
    );
    await waitFor(() => expect(result.current.data?.revision).toBe("retained"));
    failed = true;
    await act(async () => {
      await result.current.refetch();
    });
    expect(result.current.data?.revision).toBe("retained");
    expect(result.current.isLoading).toBe(false);
    await waitFor(() =>
      expect(result.current.error).toMatchObject({
        status: 408,
        message: "Request timed out",
      }),
    );
    failed = false;
    revision = "recovered";
    await act(async () => {
      await result.current.refetch();
    });
    await waitFor(() =>
      expect(result.current.data?.revision).toBe("recovered"),
    );
    expect(result.current.error).toBeNull();
  });

  it("never exposes an old backend's delayed response after switching servers", async () => {
    const nextBase = "http://127.0.0.1:8081";
    let release!: () => void;
    let started!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const received = new Promise<void>((resolve) => {
      started = resolve;
    });
    mswServer.use(
      http.get(`${testBase}/api/v1/health/ready`, () =>
        HttpResponse.json(readyHealth),
      ),
      http.get(`${nextBase}/api/v1/health/ready`, () =>
        HttpResponse.json({ ...readyHealth, instance_id: "next-instance" }),
      ),
      http.get(`${testBase}/api/v1/articles/detail`, async () => {
        started();
        await gate;
        return HttpResponse.json({ revision: "obsolete" });
      }),
      http.get(`${nextBase}/api/v1/articles/detail`, () =>
        HttpResponse.json({ revision: "current" }),
      ),
    );
    const { result } = renderHook(
      () => ({
        backend: useBackend(),
        query: useRead<{ revision: string }>("articles/detail?run=r&id=a"),
      }),
      { wrapper: providerWrapper() },
    );
    await received;
    try {
      act(() => result.current.backend.setBase(nextBase));
      await waitFor(() =>
        expect(result.current.query.data?.revision).toBe("current"),
      );
      expect(result.current.backend.instance).toBe("next-instance");
    } finally {
      await act(async () => release());
    }
    expect(result.current.query.data?.revision).toBe("current");
    expect(result.current.query.error).toBeNull();
  });
});

it("isolates delayed actor reads across actor and backend changes", async () => {
  const nextBase = "http://127.0.0.1:8081";
  let release!: () => void;
  let started!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  const received = new Promise<void>((resolve) => {
    started = resolve;
  });
  mswServer.use(
    http.get(`${testBase}/api/v1/health/ready`, () =>
      HttpResponse.json(readyHealth),
    ),
    http.get(`${nextBase}/api/v1/health/ready`, () =>
      HttpResponse.json({ ...readyHealth, instance_id: "actor-next" }),
    ),
    http.get(`${testBase}/api/v1/actors/detail`, async ({ request }) => {
      const id = new URL(request.url).searchParams.get("id");
      if (id === "old") {
        started();
        await gate;
      }
      return HttpResponse.json({ id });
    }),
    http.get(`${nextBase}/api/v1/actors/detail`, () =>
      HttpResponse.json({ id: "next-backend" }),
    ),
  );
  const { result, rerender } = renderHook(
    ({ id }) => ({ backend: useBackend(), query: useActorDossier(id) }),
    { initialProps: { id: "old" }, wrapper: providerWrapper() },
  );
  await received;
  try {
    rerender({ id: "current" });
    expect(result.current.query.data).toBeUndefined();
    await waitFor(() => expect(result.current.query.data?.id).toBe("current"));
    act(() => result.current.backend.setBase(nextBase));
    await waitFor(() =>
      expect(result.current.query.data?.id).toBe("next-backend"),
    );
  } finally {
    await act(async () => release());
  }
  expect(result.current.query.data?.id).toBe("next-backend");
});
