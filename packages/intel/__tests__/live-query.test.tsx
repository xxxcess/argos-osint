import React from "react";
import { act, render, screen, waitFor } from "@testing-library/react";
import { expect, it } from "vitest";
import { http, HttpResponse } from "msw";
import {
  BackendProvider,
  useBriefLive,
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
    http.get(`${testBase}/api/v1/articles/detail`, ({ request }) => {
      if (new URL(request.url).searchParams.get("id") === "other") {
        otherReads++;
        return HttpResponse.json({ revision: "other" });
      }
      detailReads++;
      return HttpResponse.json({ revision: String(detailReads) });
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
  await screen.findByText("revision 2");
  expect(detailReads).toBe(2);
  expect(otherReads).toBe(1);
  act(() => listeners.get("metrics")!({ type: "status", connected: false }));
  expect(await screen.findByText("CPU null")).toBeVisible();
  view.unmount();
  expect(listeners.size).toBe(0);
});
