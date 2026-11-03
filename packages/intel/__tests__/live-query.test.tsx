import React from "react";
import { act, render, screen, waitFor } from "@testing-library/react";
import { expect, it } from "vitest";
import { http, HttpResponse } from "msw";
import {
  BackendProvider,
  useBriefLive,
  useAssociationLive,
  useActorDossier,
  useTopology,
  useHostMetrics,
  useRead,
  type Article,
  type BackendSettings,
  type LiveChannel,
  type LiveEvent,
  type LiveTransport,
} from "@argos/api-client";
import { mswServer, testBase } from "../../../vitest.setup";

it("keeps metrics on the stream and refreshes only the selected brief on a meaningful transition", async () => {
  let detailReads = 0;
  let metricsReads = 0;
  let otherReads = 0;
  let finishRefresh!: () => void;
  let abortedRefreshes = 0;
  const refreshGate = new Promise<void>((resolve) => {
    finishRefresh = resolve;
  });
  mswServer.use(
    http.get(`${testBase}/api/v1/health/ready`, () =>
      HttpResponse.json({
        ready: true,
        storage: true,
        service: "argos-intel",
        api_version: 1,
        instance_id: "live-query-fixture",
      }),
    ),
    http.get(`${testBase}/api/v1/articles/detail`, async ({ request }) => {
      if (new URL(request.url).searchParams.get("id") === "other") {
        otherReads++;
        return HttpResponse.json({ revision: "other" });
      }
      const revision = ++detailReads;
      if (revision > 1) {
        request.signal.addEventListener("abort", () => {
          abortedRefreshes++;
        });
        await refreshGate;
      }
      return HttpResponse.json({ revision: String(revision) });
    }),
    http.get(`${testBase}/api/v1/host/metrics`, () => {
      metricsReads++;
      return HttpResponse.json({});
    }),
  );
  const listeners = new Map<LiveChannel["type"], (event: LiveEvent) => void>();
  const live: LiveTransport = {
    subscribe(channel, listener) {
      listeners.set(channel.type, listener);
      return () => listeners.delete(channel.type);
    },
  };
  const settings: BackendSettings = {
    subscribe: () => () => {},
    getSnapshot: () => testBase,
    getServerSnapshot: () => testBase,
    setBase: () => {},
    createRequestId: () => "fixture-request",
  };
  const article = { run_id: "r1", id: "a1" } as Article;
  function Probe() {
    const metrics = useHostMetrics();
    const brief = useRead<{ revision: string }>("articles/detail?run=r1&id=a1");
    const other = useRead<{ revision: string }>(
      "articles/detail?run=r1&id=other",
    );
    const progress = useBriefLive(article, "fixture-watch");
    return (
      <div>
        <span>CPU {String(metrics.data?.cpu_percent)}</span>
        <span>revision {brief.data?.revision}</span>
        <span>{other.data?.revision}</span>
        <span>{String(progress?.connected)}</span>
      </div>
    );
  }
  const view = render(
    <BackendProvider settings={settings} live={live}>
      <Probe />
    </BackendProvider>,
  );
  await screen.findByText("revision 1");
  await waitFor(() => expect(listeners.size).toBe(2));
  act(() =>
    listeners.get("metrics")!({
      type: "metrics",
      data: {
        cpu_percent: 0,
        memory_percent: 20,
        disk_percent: null,
        network_kbps: 0,
      },
    }),
  );
  expect(await screen.findByText("CPU 0")).toBeVisible();
  expect(metricsReads).toBe(0);
  const progress = {
    type: "brief" as const,
    run: "r1",
    id: "a1",
    job_id: "j1",
    stages: [],
    indexing: null,
    complete: false,
    refresh: false,
  };
  act(() => listeners.get("brief")!(progress));
  await screen.findByText("true");
  expect(detailReads).toBe(1);
  act(() =>
    listeners.get("brief")!({ ...progress, refresh: true, complete: true }),
  );
  try {
    await waitFor(() => expect(detailReads).toBe(2));
    await act(async () => {
      listeners.get("brief")!({ ...progress, refresh: true, complete: true });
    });
  } finally {
    finishRefresh();
  }
  await screen.findByText("revision 2");
  expect(abortedRefreshes).toBe(0);
  expect(detailReads).toBe(2);
  expect(otherReads).toBe(1);
  act(() => listeners.get("metrics")!({ type: "status", connected: false }));
  expect(await screen.findByText("CPU null")).toBeVisible();
  view.unmount();
  expect(listeners.size).toBe(0);
});

it("refreshes a zero dossier and actor topology when its source finishes indexing", async () => {
  let revision = 0;
  let dossierReads = 0;
  let topologyReads = 0;
  let listener: ((event: LiveEvent) => void) | undefined;
  mswServer.use(
    http.get(`${testBase}/api/v1/health/ready`, () =>
      HttpResponse.json({
        ready: true,
        storage: true,
        service: "argos-intel",
        api_version: 1,
        instance_id: "association-live",
      }),
    ),
    http.get(`${testBase}/api/v1/actors/detail`, () => {
      dossierReads++;
      return HttpResponse.json({ loaded_claims: revision });
    }),
    http.get(`${testBase}/api/v1/topology`, () => {
      topologyReads++;
      return HttpResponse.json({
        root_label: `graph-${revision}`,
        nodes: [],
        edges: [],
        findings: { gaps: [] },
        has_continuation: false,
      });
    }),
  );
  const live: LiveTransport = {
    subscribe(channel, next) {
      if (channel.type === "brief") listener = next;
      return () => {
        listener = undefined;
      };
    },
  };
  const settings: BackendSettings = {
    subscribe: () => () => {},
    getSnapshot: () => testBase,
    getServerSnapshot: () => testBase,
    setBase: () => {},
    createRequestId: () => "association-live",
  };
  function Probe() {
    useAssociationLive({ run_id: "r", article_id: "a" });
    const dossier = useActorDossier("actor");
    const graph = useTopology("actor");
    return (
      <div>
        <span>Claims {dossier.data?.loaded_claims}</span>
        <span>{graph.data?.root_label}</span>
      </div>
    );
  }
  render(
    <BackendProvider settings={settings} live={live}>
      <Probe />
    </BackendProvider>,
  );
  await screen.findByText("Claims 0");
  await screen.findByText("graph-0");
  await waitFor(() => expect(listener).toBeDefined());
  const progress: Extract<LiveEvent, { type: "brief" }> = {
    type: "brief",
    run: "r",
    id: "a",
    job_id: "job",
    stages: [],
    indexing: { state: "indexed", reason: "" },
    complete: true,
    refresh: false,
  };
  act(() => listener?.(progress));
  expect(dossierReads).toBe(1);
  expect(topologyReads).toBe(1);
  revision = 1;
  act(() => listener?.({ ...progress, refresh: true }));
  await screen.findByText("Claims 1");
  await screen.findByText("graph-1");
  expect(dossierReads).toBe(2);
  expect(topologyReads).toBe(2);
});

it("revalidates after an in-flight pre-index dossier returns an old zero snapshot", async () => {
  let reads = 0;
  let release!: () => void;
  let listener: ((event: LiveEvent) => void) | undefined;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  mswServer.use(
    http.get(`${testBase}/api/v1/health/ready`, () =>
      HttpResponse.json({
        ready: true,
        storage: true,
        service: "argos-intel",
        api_version: 1,
        instance_id: "dossier-race",
      }),
    ),
    http.get(`${testBase}/api/v1/actors/detail`, async () => {
      const claimCount = reads++;
      if (claimCount === 0) await gate;
      return HttpResponse.json({ loaded_claims: claimCount });
    }),
  );
  const live: LiveTransport = {
    subscribe(channel, next) {
      if (channel.type === "brief") listener = next;
      return () => {
        listener = undefined;
      };
    },
  };
  const settings: BackendSettings = {
    subscribe: () => () => {},
    getSnapshot: () => testBase,
    getServerSnapshot: () => testBase,
    setBase: () => {},
    createRequestId: () => "dossier-race",
  };
  function Probe() {
    useAssociationLive({ run_id: "r", article_id: "a" });
    const dossier = useActorDossier("actor");
    return <span>Claims {dossier.data?.loaded_claims}</span>;
  }
  render(
    <BackendProvider settings={settings} live={live}>
      <Probe />
    </BackendProvider>,
  );
  await waitFor(() => expect(reads).toBe(1));
  await waitFor(() => expect(listener).toBeDefined());
  act(() => {
    listener?.({
      type: "brief",
      run: "r",
      id: "a",
      job_id: "job",
      stages: [],
      indexing: { state: "indexed", reason: "" },
      complete: true,
      refresh: true,
    });
    release();
  });
  await screen.findByText("Claims 1");
  expect(reads).toBe(2);
});
