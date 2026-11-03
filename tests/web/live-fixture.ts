import { setInterval, clearInterval } from "node:timers";
import type { Page, WebSocketRoute } from "@playwright/test";
import type { BriefProgress, HostMetrics } from "@argos/api-client";

type Subscription = {
  metrics: boolean;
  brief: { run: string; id: string; watch?: string } | null;
};
const fixtures = new WeakMap<Page, Awaited<ReturnType<typeof createFixture>>>();

async function createFixture(page: Page) {
  const sockets = new Map<WebSocketRoute, Subscription>();
  const subscriptions: Subscription[] = [];
  let available = true;
  let metrics: HostMetrics = {
    cpu_percent: null,
    memory_percent: null,
    disk_percent: null,
    network_kbps: null,
  };
  const briefs = new Map<string, BriefProgress>();
  await page.routeWebSocket("**/api/v1/live", (socket) => {
    if (!available) {
      socket.close({ code: 1012, reason: "Fixture unavailable" });
      return;
    }
    const heartbeat = setInterval(
      () => socket.send(JSON.stringify({ type: "heartbeat" })),
      10_000,
    );
    (heartbeat as unknown as { unref?: () => void }).unref?.();
    socket.onClose(() => {
      clearInterval(heartbeat);
      sockets.delete(socket);
    });
    socket.onMessage((raw) => {
      const message = JSON.parse(raw.toString()) as Subscription & {
        type: string;
      };
      if (message.type !== "subscribe") return;
      sockets.set(socket, message);
      subscriptions.push(message);
      if (message.metrics)
        socket.send(JSON.stringify({ type: "metrics", data: metrics }));
      if (message.brief) {
        const { run, id } = message.brief;
        socket.send(
          JSON.stringify({
            type: "brief",
            ...(briefs.get(`${run}/${id}`) || {
              run,
              id,
              job_id: null,
              stages: [],
              indexing: null,
              complete: true,
              refresh: true,
            }),
          }),
        );
      }
    });
  });
  return {
    subscriptions,
    metrics(value: HostMetrics) {
      metrics = value;
      for (const [socket, subscription] of sockets)
        if (subscription.metrics)
          socket.send(JSON.stringify({ type: "metrics", data: value }));
    },
    brief(value: BriefProgress) {
      briefs.set(`${value.run}/${value.id}`, value);
      for (const [socket, subscription] of sockets)
        if (
          subscription.brief?.run === value.run &&
          subscription.brief.id === value.id
        )
          socket.send(JSON.stringify({ type: "brief", ...value }));
    },
    disconnect() {
      available = false;
      for (const socket of sockets.keys())
        socket.close({ code: 1012, reason: "Fixture interrupted" });
      sockets.clear();
    },
    reconnect() {
      available = true;
    },
  };
}

export async function installLiveFixture(page: Page) {
  const existing = fixtures.get(page);
  if (existing) return existing;
  const fixture = await createFixture(page);
  fixtures.set(page, fixture);
  return fixture;
}
