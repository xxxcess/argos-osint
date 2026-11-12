import { decodeLiveEvent } from "@argos/api-client";
import type { LiveChannel, LiveEvent, LiveTransport } from "@argos/api-client";

/** One connection per UI instance. Next relays; Rust owns the requested channels. */
export function createLiveTransport(
  options: {
    basePath?: string;
    contextRevision?: number;
    onInvalidated?: () => void;
  } = {},
): LiveTransport & { close: () => void } {
  const subscriptions = new Map<
    symbol,
    { channel: LiveChannel; listener: (event: LiveEvent) => void }
  >();
  let socket: WebSocket | undefined;
  let retry: ReturnType<typeof setTimeout> | undefined;
  let watchdog: ReturnType<typeof setInterval> | undefined;
  let attempts = 0;
  let lastMessage = 0;
  let closed = false;
  const publish = (event: LiveEvent) => {
    for (const { channel, listener } of subscriptions.values()) {
      if (
        event.type === "status" ||
        (event.type === "metrics" && channel.type === "metrics") ||
        (event.type === "brief" &&
          channel.type === "brief" &&
          channel.run === event.run &&
          channel.id === event.id) ||
        (event.type === "error" && channel.type === event.channel)
      )
        listener(event);
    }
  };
  const sendSubscriptions = () => {
    if (socket?.readyState !== WebSocket.OPEN) return;
    const channels = [...subscriptions.values()].map(({ channel }) => channel);
    const brief = channels.find((channel) => channel.type === "brief");
    socket.send(
      JSON.stringify({
        type: "subscribe",
        metrics: channels.some((channel) => channel.type === "metrics"),
        brief:
          brief?.type === "brief"
            ? { run: brief.run, id: brief.id, watch: brief.watch }
            : null,
      }),
    );
  };
  const connect = () => {
    if (closed || !subscriptions.size || socket) return;
    const url = new URL(
      `${options.basePath || ""}/api/v1/live`,
      window.location.href,
    );
    if (options.contextRevision !== undefined)
      url.searchParams.set("context_revision", String(options.contextRevision));
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    clearTimeout(retry);
    const current = new WebSocket(url);
    socket = current;
    current.onopen = () => {
      if (socket !== current) return;
      lastMessage = Date.now();
      publish({ type: "status", connected: true });
      sendSubscriptions();
      watchdog = setInterval(() => {
        if (Date.now() - lastMessage > 30_000) current.close();
      }, 5000);
    };
    current.onmessage = (message) => {
      if (socket !== current || typeof message.data !== "string") return;
      lastMessage = Date.now();
      try {
        const event = decodeLiveEvent(message.data);
        attempts = 0;
        if (event) publish(event);
      } catch {
        current.close(1007, "Invalid live message");
      }
    };
    current.onerror = () => {
      current.close();
    };
    current.onclose = (event) => {
      if (socket !== current) return;
      socket = undefined;
      clearInterval(watchdog);
      publish({ type: "status", connected: false });
      if (event.code === 1008) {
        closed = true;
        options.onInvalidated?.();
        return;
      }
      if (subscriptions.size)
        retry = setTimeout(connect, Math.min(30_000, 1000 * 2 ** attempts++));
    };
  };
  return {
    close() {
      closed = true;
      clearTimeout(retry);
      clearInterval(watchdog);
      const previous = socket;
      socket = undefined;
      previous?.close();
      publish({ type: "status", connected: false });
      subscriptions.clear();
    },
    subscribe(channel, listener) {
      if (closed) {
        listener({ type: "status", connected: false });
        return () => {};
      }
      const key = Symbol();
      subscriptions.set(key, { channel, listener });
      listener({
        type: "status",
        connected: socket?.readyState === WebSocket.OPEN,
      });
      if (socket?.readyState === WebSocket.OPEN) sendSubscriptions();
      else queueMicrotask(connect);
      return () => {
        subscriptions.delete(key);
        if (subscriptions.size) {
          sendSubscriptions();
          return;
        }
        clearTimeout(retry);
        clearInterval(watchdog);
        const previous = socket;
        socket = undefined;
        previous?.close();
      };
    },
  };
}
