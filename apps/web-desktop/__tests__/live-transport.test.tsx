import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { createLiveTransport } from "../app/live-transport";
import type { LiveEvent } from "@argos/api-client";

class Socket {
  static OPEN = 1;
  static instances: Socket[] = [];
  readyState = 0;
  sent: string[] = [];
  onopen?: () => void;
  onmessage?: (event: { data: string }) => void;
  onclose?: () => void;
  onerror?: () => void;
  constructor(readonly url: URL) {
    Socket.instances.push(this);
  }
  open() {
    this.readyState = 1;
    this.onopen?.();
  }
  message(value: unknown) {
    this.onmessage?.({ data: JSON.stringify(value) });
  }
  send(value: string) {
    this.sent.push(value);
  }
  close() {
    this.readyState = 3;
    this.onclose?.();
  }
}
beforeEach(() => {
  Socket.instances = [];
  vi.useFakeTimers();
  vi.stubGlobal("WebSocket", Socket);
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

it("shares one same-origin socket and replaces the requested channels on navigation", async () => {
  const live = createLiveTransport();
  const metrics: LiveEvent[] = [];
  const brief: LiveEvent[] = [];
  const stopMetrics = live.subscribe({ type: "metrics" }, (event) =>
    metrics.push(event),
  );
  const stopBrief = live.subscribe(
    { type: "brief", run: "r1", id: "a1", watch: "v1" },
    (event) => brief.push(event),
  );
  await Promise.resolve();
  expect(Socket.instances).toHaveLength(1);
  const socket = Socket.instances[0];
  expect(socket.url.host).toBe(window.location.host);
  expect(socket.url.pathname).toBe("/api/v1/live");
  socket.open();
  expect(JSON.parse(socket.sent.at(-1)!)).toEqual({
    type: "subscribe",
    metrics: true,
    brief: { run: "r1", id: "a1", watch: "v1" },
  });
  socket.message({
    type: "metrics",
    data: {
      cpu_percent: 0,
      memory_percent: 25,
      disk_percent: null,
      network_kbps: 0,
    },
  });
  expect(metrics.at(-1)?.type).toBe("metrics");
  expect(brief.at(-1)?.type).toBe("status");
  stopBrief();
  expect(JSON.parse(socket.sent.at(-1)!)).toEqual({
    type: "subscribe",
    metrics: true,
    brief: null,
  });
  stopMetrics();
  await vi.advanceTimersByTimeAsync(60_000);
  expect(Socket.instances).toHaveLength(1);
  expect(socket.readyState).toBe(3);
});

it("backs off failed Rust reconnects even when the Next relay handshake opened", async () => {
  const live = createLiveTransport();
  const stop = live.subscribe({ type: "metrics" }, () => {});
  await Promise.resolve();
  Socket.instances[0].open();
  Socket.instances[0].close();
  await vi.advanceTimersByTimeAsync(1000);
  expect(Socket.instances).toHaveLength(2);
  Socket.instances[1].open();
  Socket.instances[1].close();
  await vi.advanceTimersByTimeAsync(1999);
  expect(Socket.instances).toHaveLength(2);
  await vi.advanceTimersByTimeAsync(1);
  expect(Socket.instances).toHaveLength(3);
  Socket.instances[2].open();
  expect(JSON.parse(Socket.instances[2].sent[0])).toEqual({
    type: "subscribe",
    metrics: true,
    brief: null,
  });
  stop();
});

it("ignores delayed progress from the previous article and rejects malformed metrics", async () => {
  const live = createLiveTransport();
  const metricsStop = live.subscribe({ type: "metrics" }, () => {});
  const firstStop = live.subscribe(
    { type: "brief", run: "r1", id: "a1", watch: "v1" },
    () => {},
  );
  await Promise.resolve();
  const socket = Socket.instances[0];
  socket.open();
  firstStop();
  const events: LiveEvent[] = [];
  const secondStop = live.subscribe(
    { type: "brief", run: "r1", id: "a2", watch: "v2" },
    (event) => events.push(event),
  );
  const event = {
    type: "brief",
    run: "r1",
    id: "a1",
    job_id: null,
    stages: [],
    indexing: null,
    complete: true,
    refresh: true,
  };
  socket.message(event);
  expect(events.filter((event) => event.type === "brief")).toHaveLength(0);
  socket.message({ ...event, id: "a2" });
  expect(events.filter((event) => event.type === "brief")).toHaveLength(1);
  socket.message({ type: "metrics", data: { cpu_percent: "unavailable" } });
  expect(socket.readyState).toBe(3);
  secondStop();
  metricsStop();
});

it("does not open a throwaway connection during immediate Strict Mode cleanup", async () => {
  const live = createLiveTransport();
  const stop = live.subscribe({ type: "metrics" }, () => {});
  stop();
  await Promise.resolve();
  expect(Socket.instances).toHaveLength(0);
});

it("reconnects a silent socket, ignores its late messages and cancels retries on cleanup", async () => {
  const events: LiveEvent[] = [];
  const live = createLiveTransport();
  const stop = live.subscribe({ type: "metrics" }, (event) =>
    events.push(event),
  );
  await Promise.resolve();
  const stale = Socket.instances[0];
  stale.open();
  await vi.advanceTimersByTimeAsync(35_000);
  expect(stale.readyState).toBe(3);
  expect(events.at(-1)).toEqual({ type: "status", connected: false });
  await vi.advanceTimersByTimeAsync(1000);
  const current = Socket.instances[1];
  current.open();
  const count = events.length;
  stale.message({
    type: "metrics",
    data: {
      cpu_percent: 88,
      memory_percent: 0,
      disk_percent: 0,
      network_kbps: 0,
    },
  });
  expect(events).toHaveLength(count);
  current.close();
  stop();
  await vi.advanceTimersByTimeAsync(60_000);
  expect(Socket.instances).toHaveLength(2);
});

it("keeps a healthy heartbeat stream connected without emitting fictitious metrics", async () => {
  const events: LiveEvent[] = [];
  const live = createLiveTransport();
  const stop = live.subscribe({ type: "metrics" }, (event) =>
    events.push(event),
  );
  await Promise.resolve();
  const socket = Socket.instances[0];
  socket.open();
  for (let i = 0; i < 4; i++) {
    await vi.advanceTimersByTimeAsync(15_000);
    socket.message({ type: "heartbeat" });
  }
  expect(socket.readyState).toBe(1);
  expect(events.filter((event) => event.type === "metrics")).toEqual([]);
  expect(Socket.instances).toHaveLength(1);
  stop();
});
