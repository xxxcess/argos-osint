"use client";
import React, {
  createContext,
  useContext,
  useSyncExternalStore,
  useMemo,
  useState,
} from "react";
import {
  QueryClient,
  QueryClientProvider,
  useQuery,
  useMutation,
  useQueryClient,
  useInfiniteQuery,
} from "@tanstack/react-query";
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
  actors: { id: string; display_name: string; type_hint: string }[];
  claims: {
    id: string;
    text: string;
    classification: string;
    confidence: number | null;
    topology_seed?: Seed;
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
  }[];
  edges: { id: string; from: string; to: string; label: string }[];
  findings: { gaps: string[] };
  has_continuation: boolean;
}
interface Health {
  ready: boolean;
  storage: boolean;
  service: string;
  api_version: number;
  instance_id: string;
}
export const DEFAULT_BACKEND = "http://127.0.0.1:8080";

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
  connecting: boolean;
  retry: () => void;
  createRequestId: () => string;
  theme: "dark" | "light";
  setTheme: (v: "dark" | "light") => void;
}
const BackendContext = createContext<Backend | null>(null);
export function safeBackend(raw: string) {
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
    throw new Error("Use a loopback HTTP origin, e.g. http://127.0.0.1:8080");
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

function combineSignals(
  signal?: AbortSignal,
  timeoutMs = 10000,
): { signal?: AbortSignal; cleanup?: () => void } {
  const timeoutSignal =
    typeof AbortSignal !== "undefined" && typeof AbortSignal.timeout === "function"
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
): Promise<T> {
  const combined = combineSignals(signal, 10000);
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
  } finally {
    combined.cleanup?.();
  }

  if (!response.ok) {
    let payload: unknown = undefined;
    let message = `HTTP ${response.status}`;
    try {
      const text = await response.text();
      try {
        const json = JSON.parse(text);
        payload = json;
        if (json && typeof json === "object") {
          if ("error" in json && typeof (json as { error: unknown }).error === "string") {
            message = (json as { error: string }).error;
          } else if ("message" in json && typeof (json as { message: unknown }).message === "string") {
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
}

export function BackendProvider({
  children,
  settings,
}: {
  children: React.ReactNode;
  settings: BackendSettings;
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
      <Connection settings={settings}>{children}</Connection>
    </QueryClientProvider>
  );
}
const READY_POLL_MS = 60_000;

function Connection({
  children,
  settings,
}: {
  children: React.ReactNode;
  settings: BackendSettings;
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
    connecting: health.isFetching,
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
export function useRead<T>(path: string, enabled = true) {
  const b = useBackend();
  return useQuery({
    queryKey: [b.base, b.instance, path],
    queryFn: ({ signal }) => request<T>(b.base, path, undefined, signal),
    enabled: b.connected && enabled,
    refetchInterval: 5000,
  });
}
export function useAction<T>(path: string) {
  const b = useBackend(),
    client = useQueryClient();
  return useMutation({
    mutationFn: (body: unknown) => {
      if (!b.connected) throw new Error("API offline");
      return request<T>(b.base, path, body);
    },
    onSuccess: () => {
      return client.invalidateQueries({ queryKey: [b.base, b.instance] });
    },
  });
}
export function useArticles(category: string, day: string, search: string) {
  const b = useBackend();
  return useInfiniteQuery({
    queryKey: [b.base, b.instance, "articles", category, day, search],
    initialPageParam: "",
    refetchInterval: 5000,
    enabled: b.connected && !!day,
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
export function keyFor(a: Article) {
  return new URLSearchParams({ run: a.run_id, id: a.id }).toString();
}
export function useTopology(seed: Seed | null) {
  const b = useBackend();
  const stable = useMemo(() => JSON.stringify(seed), [seed]);
  return useQuery({
    queryKey: [b.base, b.instance, "topology", stable],
    enabled: b.connected && !!seed,
    queryFn: ({ signal }) =>
      request<Topology>(b.base, "topology", { seed }, signal),
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
    refetchInterval: 5000,
  });
}
