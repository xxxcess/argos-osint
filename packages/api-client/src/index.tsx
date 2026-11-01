"use client";
import React, {
  createContext,
  useContext,
  useSyncExternalStore,
  useState,
  useEffect,
} from "react";
import {
  QueryClient,
  QueryClientProvider,
  useQuery,
  useMutation,
  useQueryClient,
  useInfiniteQuery,
} from "@tanstack/react-query";
import type { LiveTransport, BriefProgress } from "./live";
export { decodeLiveEvent } from "./live";
export type {
  LiveTransport,
  LiveChannel,
  LiveEvent,
  BriefProgress,
} from "./live";
/** Telemetry from the Rust API host. Null means unavailable or warming up. */
export interface HostMetrics {
  cpu_percent: number | null;
  memory_percent: number | null;
  disk_percent: number | null;
  network_kbps: number | null;
}
export interface Article {
  id: string;
  run_id: string;
  title: string;
  description: string;
  source_name: string;
  source_domain: string;
  published_at: string;
  category: string;
  image_url: string;
  url: string;
  author: string;
}
export interface Brief {
  article: Article;
  source_revision?: string;
  topology_seed?: Seed | null;
  inferences?: { id: string; text: string; topology_seed?: Seed | null }[];
  actors: {
    id: string;
    display_name: string;
    type_hint: string;
    topology_seed?: Seed | null;
  }[];
  claims: {
    id: string;
    text: string;
    classification: string;
    confidence: number | null;
    topology_seed?: Seed | null;
    admiralty: string;
  }[];
  body: { body_markdown: string } | null;
  output: Record<string, unknown> | null;
  lifecycle: string;
  status: string;
  revision: string;
  publication: [number, unknown, unknown] | null;
  relations: [string, string, string][];
  links?: string[];
  confidence?: {
    score: number | null;
    provisional: boolean;
    claims: number;
    inferences: number;
  } | null;
  body_pending?: boolean;
  job_id?: string;
  indexing?: { state: string; reason: string } | null;
}
export interface Seed {
  [key: string]: unknown;
}
export interface Topology {
  root_label: string;
  nodes: {
    id: string;
    label: string;
    kind: string;
    detail: string;
    topology_seed: Seed | null;
    is_root?: boolean;
  }[];
  edges: { id: string; from: string; to: string; label: string }[];
  findings: { gaps: string[] };
  has_continuation: boolean;
}
export interface Health {
  ready: boolean;
  storage: boolean;
  service: string;
  api_version: number;
  instance_id: string;
}
export const DEFAULT_BACKEND = "";

// Each app supplies its platform persistence and request-ID implementation.
export interface BackendSettings {
  subscribe: (onChange: () => void) => () => void;
  getSnapshot: () => string;
  getServerSnapshot: () => string;
  setBase: (base: string) => void;
  createRequestId: () => string;
}

interface Backend {
  base: string;
  setBase: (s: string) => void;
  instance: string;
  connected: boolean;
  live?: LiveTransport;
  connecting: boolean;
  health?: Health;
  connectionError?: string;
  retry: () => void;
  createRequestId: () => string;
  theme: "dark" | "light";
  setTheme: (v: "dark" | "light") => void;
}
const BackendContext = createContext<Backend | null>(null);
export function safeBackend(raw: string) {
  if (raw === "") return "";
  const u = new URL(raw);
  if (
    u.protocol !== "http:" ||
    !["127.0.0.1", "localhost"].includes(u.hostname) ||
    u.username ||
    u.password ||
    u.pathname !== "/" ||
    u.search ||
    u.hash
  )
    throw new Error("Use a loopback HTTP origin for the workspace");
  return u.origin;
}
export class ApiError extends Error {
  status: number;
  payload?: unknown;

  constructor(status: number, message: string, payload?: unknown) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.payload = payload;
  }
}

// Longer than the API's 45s deadline and shorter than the Next relay's 55s
// deadline, so a slow read queued behind a running brief can finish and a
// real server timeout arrives as JSON instead of AbortSignal "signal timed out".
const REQUEST_TIMEOUT_MS = 52_000;

function combineSignals(
  signal?: AbortSignal,
  timeoutMs = REQUEST_TIMEOUT_MS,
): { signal?: AbortSignal; cleanup?: () => void } {
  const timeoutSignal =
    typeof AbortSignal !== "undefined" &&
    typeof AbortSignal.timeout === "function"
      ? AbortSignal.timeout(timeoutMs)
      : undefined;

  if (!signal) {
    return { signal: timeoutSignal };
  }
  if (!timeoutSignal) {
    return { signal };
  }

  if (typeof AbortSignal.any === "function") {
    return { signal: AbortSignal.any([signal, timeoutSignal]) };
  }

  const controller = new AbortController();
  const onAbort = () => {
    controller.abort(signal.aborted ? signal.reason : timeoutSignal.reason);
  };
  if (signal.aborted) {
    controller.abort(signal.reason);
    return { signal: controller.signal };
  }
  if (timeoutSignal.aborted) {
    controller.abort(timeoutSignal.reason);
    return { signal: controller.signal };
  }
  signal.addEventListener("abort", onAbort, { once: true });
  timeoutSignal.addEventListener("abort", onAbort, { once: true });
  return {
    signal: controller.signal,
    cleanup: () => {
      signal.removeEventListener("abort", onAbort);
      timeoutSignal.removeEventListener("abort", onAbort);
    },
  };
}

async function request<T>(
  base: string,
  path: string,
  body?: unknown,
  signal?: AbortSignal,
  timeoutMs = REQUEST_TIMEOUT_MS,
): Promise<T> {
  const combined = combineSignals(signal, timeoutMs);
  let response: Response;
  try {
    response = await fetch(`${base}/api/v1/${path}`, {
      method: body === undefined ? "GET" : "POST",
      headers: body === undefined ? {} : { "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: combined.signal,
      credentials: "omit",
      cache: "no-store",
    });

    if (!response.ok) {
      let payload: unknown = undefined;
      let message = `HTTP ${response.status}`;
      try {
        const text = await response.text();
        try {
          const json = JSON.parse(text);
          payload = json;
          if (json && typeof json === "object") {
            if (
              "error" in json &&
              typeof (json as { error: unknown }).error === "string"
            ) {
              message = (json as { error: string }).error;
            } else if (
              "message" in json &&
              typeof (json as { message: unknown }).message === "string"
            ) {
              message = (json as { message: string }).message;
            }
          }
        } catch {
          if (text.trim().length > 0) {
            message = text.trim();
            payload = text;
          }
        }
      } catch {
        // response reading failed, keep default message
      }
      throw new ApiError(response.status, message, payload);
    }

    if (response.status === 204) {
      return undefined as T;
    }
    const text = await response.text();
    if (!text) {
      return undefined as T;
    }
    try {
      return JSON.parse(text) as T;
    } catch {
      return text as unknown as T;
    }
  } finally {
    combined.cleanup?.();
  }
}

export function BackendProvider({
  children,
  settings,
  live,
}: {
  children: React.ReactNode;
  settings: BackendSettings;
  live?: LiveTransport;
}) {
  const [client] = useState(
    () =>
      new QueryClient({
        defaultOptions: {
          queries: { retry: 1, refetchOnWindowFocus: true },
          mutations: { retry: false },
        },
      }),
  );
  return (
    <QueryClientProvider client={client}>
      <Connection settings={settings} live={live}>
        {children}
      </Connection>
    </QueryClientProvider>
  );
}
const READY_POLL_MS = 60_000;

function Connection({
  children,
  settings,
  live,
}: {
  children: React.ReactNode;
  settings: BackendSettings;
  live?: LiveTransport;
}) {
  const base = useSyncExternalStore(
    settings.subscribe,
    settings.getSnapshot,
    settings.getServerSnapshot,
  );
  const [theme, setTheme] = useState<"dark" | "light">("dark");
  const client = useQueryClient();
  const health = useQuery({
    queryKey: ["health", base],
    queryFn: ({ signal }) =>
      request<Health>(base, "health/ready", undefined, signal),
    refetchInterval: READY_POLL_MS,
    retry: false,
  });
  const connected =
    health.isSuccess &&
    health.data.ready &&
    health.data.storage &&
    health.data.service === "argos-intel" &&
    health.data.api_version === 1;
  const instance = connected ? health.data.instance_id : "";
  const setBase = (raw: string) => {
    const next = safeBackend(raw);
    client.clear();
    settings.setBase(next);
  };
  const value: Backend = {
    base,
    setBase,
    instance,
    connected,
    live,
    connecting: health.isFetching,
    health: health.isSuccess ? health.data : undefined,
    connectionError: health.error?.message,
    createRequestId: settings.createRequestId,
    retry: () => {
      void health.refetch();
    },
    theme,
    setTheme,
  };
  return (
    <BackendContext.Provider value={value}>{children}</BackendContext.Provider>
  );
}
export function useBackend() {
  const v = useContext(BackendContext);
  if (!v) throw new Error("BackendProvider required");
  return v;
}
export function useRead<T>(
  path: string,
  enabled = true,
  timeoutMs = REQUEST_TIMEOUT_MS,
) {
  const b = useBackend();
  return useQuery({
    queryKey: [b.base, b.instance, path],
    queryFn: ({ signal }) =>
      request<T>(b.base, path, undefined, signal, timeoutMs),
    enabled: b.connected && enabled,
    staleTime: 30_000,
    refetchOnWindowFocus: false,
  });
}
// @tag:host-metrics — normal backend/instance keys and readiness gating apply.
export function useHostMetrics() {
  const b = useBackend();
  const client = useQueryClient();
  const query = useQuery({
    queryKey: [b.base, b.instance, "host/metrics"],
    queryFn: ({ signal }) =>
      request<HostMetrics>(b.base, "host/metrics", undefined, signal),
    enabled: b.connected && !b.live,
    staleTime: Infinity,
    refetchOnWindowFocus: false,
  });
  useEffect(() => {
    if (!b.connected || !b.live) return;
    const key = [b.base, b.instance, "host/metrics"];
    return b.live.subscribe({ type: "metrics" }, (event) => {
      if (event.type === "metrics") client.setQueryData(key, event.data);
      else if (
        (event.type === "status" && !event.connected) ||
        event.type === "error"
      )
        client.setQueryData<HostMetrics>(key, {
          cpu_percent: null,
          memory_percent: null,
          disk_percent: null,
          network_kbps: null,
        });
    });
  }, [b.base, b.instance, b.connected, b.live, client]);
  return query;
}

/** REST remains the brief snapshot. Push updates invalidate only this occurrence. */
export function useBriefLive(article: Article, watch: string) {
  const { run_id: run, id } = article;
  const b = useBackend();
  const client = useQueryClient();
  const queryKey = [b.base, b.instance, "brief/live", run, id];
  const progress = useQuery<{ connected: boolean; progress?: BriefProgress }>({
    queryKey,
    enabled: false,
    queryFn: async () => ({ connected: false }),
    staleTime: Infinity,
  });
  useEffect(() => {
    if (!b.connected || !b.live) return;
    const key = [b.base, b.instance, "brief/live", run, id];
    return b.live.subscribe(
      { type: "brief", run: run, id: id, watch },
      (event) => {
        if (event.type === "status")
          client.setQueryData(
            key,
            (old: { progress?: BriefProgress } | undefined) => ({
              ...old,
              connected: event.connected,
            }),
          );
        if (event.type === "brief") {
          client.setQueryData(key, { connected: true, progress: event });
          if (event.refresh)
            void client.invalidateQueries({
              queryKey: [
                b.base,
                b.instance,
                `articles/detail?${keyFor({ run_id: run, id })}`,
              ],
              exact: true,
            });
        }
        if (event.type === "error")
          client.setQueryData(
            key,
            (old: { progress?: BriefProgress } | undefined) => ({
              ...old,
              connected: false,
            }),
          );
      },
    );
  }, [b.base, b.instance, b.connected, b.live, run, id, watch, client]);
  return progress.data;
}

export function useAction<T>(path: string, signal?: AbortSignal) {
  const b = useBackend(),
    client = useQueryClient();
  return useMutation({
    mutationFn: (body: unknown) => {
      if (!b.connected) throw new Error("API offline");
      return request<T>(b.base, path, body, signal);
    },
    onSuccess: (_data, variables) => {
      // Resolving an existing identity is read-only; it changes no cached data.
      if (path === "topology/resolve") return;
      if (path === "brief/ensure" || path === "brief/reload") {
        const source = variables as { run?: string; id?: string };
        const detail = `articles/detail?${keyFor({ run_id: source.run || "", id: source.id || "" })}`;
        return client.invalidateQueries({
          predicate: ({ queryKey }) => {
            if (queryKey[0] !== b.base || queryKey[1] !== b.instance)
              return false;
            return (
              queryKey[2] === detail ||
              queryKey[2] === "articles" ||
              (queryKey[2] === "related" &&
                queryKey[3] === source.run &&
                queryKey[4] === source.id)
            );
          },
        });
      }
      return client.invalidateQueries({ queryKey: [b.base, b.instance] });
    },
  });
}
/** Specific categories supported by v1; "all" selects the server's global feed. */
export const INTEL_CATEGORIES = [
  "geopolitical",
  "economic",
  "military",
  "information",
  "stability",
  "technology",
] as const;
// Native "all" uses this same stream: the server owns order and opaque cursors.
export function useArticles(
  category: string,
  day: string,
  search: string,
  enabled = true,
) {
  const b = useBackend();
  return useInfiniteQuery({
    queryKey: [b.base, b.instance, "articles", category, day, search],
    initialPageParam: "",
    // A visible feed gets one slow freshness check; briefs never poll.
    refetchInterval: enabled ? 60_000 : false,
    staleTime: 30_000,
    refetchOnWindowFocus: false,
    enabled: b.connected && !!day && enabled,
    queryFn: ({ pageParam, signal }) =>
      request<{
        rows: Article[];
        next_cursor: string | null;
        revision: string;
      }>(
        b.base,
        "articles?" +
          new URLSearchParams({
            category,
            day,
            search,
            ...(pageParam ? { cursor: pageParam } : {}),
          }),
        undefined,
        signal,
      ),
    getNextPageParam: (last) => last.next_cursor || undefined,
  });
}
export function keyFor(a: Pick<Article, "run_id" | "id">) {
  return new URLSearchParams({ run: a.run_id, id: a.id }).toString();
}
export function useTopology(seed: string | null) {
  const b = useBackend();
  return useQuery({
    queryKey: [b.base, b.instance, "topology", seed],
    enabled: b.connected && !!seed,
    queryFn: ({ signal }) =>
      request<Topology>(
        b.base,
        `topology?seed=${encodeURIComponent(seed || "")}`,
        undefined,
        signal,
      ),
  });
}

export function useRelated(article: Article) {
  const b = useBackend();
  return useInfiniteQuery({
    queryKey: [b.base, b.instance, "related", article.run_id, article.id],
    initialPageParam: "",
    enabled: b.connected,
    queryFn: ({ pageParam, signal }) =>
      request<{
        rows: { article: Article; basis: string[]; tentative: boolean }[];
        next_cursor: string | null;
      }>(
        b.base,
        "articles/related?" +
          keyFor(article) +
          (pageParam ? "&" + new URLSearchParams({ cursor: pageParam }) : ""),
        undefined,
        signal,
      ),
    getNextPageParam: (last) => last.next_cursor || undefined,
    staleTime: 30_000,
    refetchOnWindowFocus: false,
  });
}
