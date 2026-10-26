"use client";
import React, {
  createContext,
  useContext,
  useEffect,
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
interface Backend {
  base: string;
  setBase: (s: string) => void;
  instance: string;
  connected: boolean;
  retry: () => void;
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
async function request<T>(
  base: string,
  path: string,
  body?: unknown,
): Promise<T> {
  const response = await fetch(`${base}/api/v1/${path}`, {
    method: body === undefined ? "GET" : "POST",
    headers: body === undefined ? {} : { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(10000),
    credentials: "omit",
    cache: "no-store",
  });
  if (!response.ok) {
    const error = await response
      .json()
      .catch(() => ({ error: `HTTP ${response.status}` }));
    throw new Error(error.error || `HTTP ${response.status}`);
  }
  return response.json();
}
export function BackendProvider({ children }: { children: React.ReactNode }) {
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
      <Connection>{children}</Connection>
    </QueryClientProvider>
  );
}
const READY_POLL_MS = 60_000;

function Connection({ children }: { children: React.ReactNode }) {
  const [base, update] = useState("http://127.0.0.1:8080");
  const [theme, setTheme] = useState<"dark" | "light">("dark");
  const client = useQueryClient();
  useEffect(() => {
    try {
      update(
        safeBackend(
          new URLSearchParams(location.search).get("api") ||
            localStorage.getItem("argos.backend") ||
            base,
        ),
      );
    } catch {
      /* default local endpoint */
    }
  }, []); // Initial configuration only.
  const health = useQuery({
    queryKey: ["health", base],
    queryFn: () => request<Health>(base, "health/ready"),
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
    update(next);
    localStorage.setItem("argos.backend", next);
  };
  const value: Backend = {
    base,
    setBase,
    instance,
    connected,
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
    queryFn: () => request<T>(b.base, path),
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
    queryFn: ({ pageParam }) =>
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
    queryFn: () => request<Topology>(b.base, "topology", { seed }),
  });
}

export function useRelated(article: Article) {
  const b = useBackend();
  return useInfiniteQuery({
    queryKey: [b.base, b.instance, "related", article.run_id, article.id],
    initialPageParam: "",
    enabled: b.connected,
    queryFn: ({ pageParam }) =>
      request<{
        rows: { article: Article; basis: string[]; tentative: boolean }[];
        next_cursor: string | null;
      }>(
        b.base,
        "articles/related?" +
          keyFor(article) +
          (pageParam ? "&" + new URLSearchParams({ cursor: pageParam }) : ""),
      ),
    getNextPageParam: (last) => last.next_cursor || undefined,
    refetchInterval: 5000,
  });
}
