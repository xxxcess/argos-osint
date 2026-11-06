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
  keepPreviousData,
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
  /** Host OS name. Absent or null when the API did not report one. */
  platform?: string | null;
  chipset?: string | null;
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
  /** Ordered memberships; scalar category remains the primary legacy-compatible value. */
  categories?: string[];
  taxonomy_version?: string;
  /** Current publication authority; historical categories alone do not identify domains. */
  admission_state?: string;
  publication_version?: string;
  image_url: string;
  url: string;
  author: string;
  /** Article heat from 0 (cold) to 1 (hot). */
  temperature?: number;
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
  confidence_grade?: string;
  confidence_basis?: "evidence" | "publisher_prior" | "unavailable";
  confidence_reason?: string;
  confidence?: {
    score: number | null;
    provisional: boolean;
    claims: number;
    inferences: number;
  } | null;
  body_pending?: boolean;
  actors_pending?: boolean;
  actors_body_pending?: boolean;
  actors_body_unavailable?: boolean;
  bindings_pending?: boolean;
  job_id?: string;
  indexing?: { state: string; reason: string } | null;
}
export interface Seed {
  [key: string]: unknown;
}
export const BRIEF_FACETS = [
  { id: "current_state", name: "Current State" },
  { id: "significant_developments", name: "Significant Developments" },
  { id: "actor_relationships", name: "Actor Relationships" },
  { id: "cross_domain_implications", name: "Cross-Domain Implications" },
  { id: "emerging_signals", name: "Emerging Signals" },
  { id: "conditional_outlook", name: "Conditional Outlook" },
  { id: "evidence", name: "Evidence" },
  { id: "limitations", name: "Limitations" },
] as const;
export type BriefFacetId = (typeof BRIEF_FACETS)[number]["id"];
export interface NeighborhoodBriefItem {
  text: string;
  epistemic: "recorded" | "stored_inference" | "tentative" | "coverage";
  node_ids: string[];
  edge_ids: string[];
  member_ids: string[];
  receipts: string[];
  basis: number[];
}
/** Presentation adapter from Rust NeighborhoodBrief; counts remain host owned. */
export interface NeighborhoodBrief {
  format_version: number;
  basis_key: string;
  seed: Seed;
  seed_label: string;
  overview: NeighborhoodBriefItem | null;
  connections: NeighborhoodBriefItem[];
  interpretation: NeighborhoodBriefItem[];
  limits: NeighborhoodBriefItem[];
  references: {
    n: number;
    target: { Node: string } | { Edge: string };
    label: string;
  }[];
  coverage: {
    status: "ready" | "limited" | "isolated" | "missing";
    depth: number;
    cycle_scope: "AllRetained" | { ThisCycle: string };
    admitted_nodes: number;
    admitted_edges: number;
    discovered_nodes: number;
    discovered_edges: number;
    omitted_nodes: number;
    omitted_edges: number;
    unresolved_endpoints: number;
    exploration_exhausted: boolean;
    payload_reads_skipped: number;
  };
  facets?: {
    facet: BriefFacetId;
    items: NeighborhoodBriefItem[];
    empty_reason: string | null;
  }[];
}
export type TopologySummaryView =
  | { kind: "recorded" | "pending"; brief: NeighborhoodBrief }
  | {
      kind: "generated";
      brief: NeighborhoodBrief;
      route: string;
      host_notes: string[];
    }
  | {
      kind: "failed";
      brief: NeighborhoodBrief;
      diagnostic: { kind: string; message: string };
    };
export interface TopologySummaryKey {
  seed: string;
  basis: string;
}
export interface Topology {
  root_label: string;
  neighborhood_brief?: NeighborhoodBrief | null;
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
          platform: null,
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
            void client.invalidateQueries(
              {
                queryKey: [
                  b.base,
                  b.instance,
                  `articles/detail?${keyFor({ run_id: run, id })}`,
                ],
                exact: true,
              },
              { cancelRefetch: false },
            );
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
  "governance",
  "security",
  "economic",
  "human",
  "environmental",
  "scientific",
] as const;
export type StrategicDomain = (typeof INTEL_CATEGORIES)[number];
export const INTEL_DOMAIN_NAMES: Record<StrategicDomain, string> = {
  governance: "Governance & Political Stability",
  security: "Security & Threat Environment",
  economic: "Economic & Supply Chain Resilience",
  human: "Human Security & Public Welfare",
  environmental: "Environmental & Resource Security",
  scientific: "Scientific & Technological Capability",
};
// Native "all" uses this same stream: the server owns order and opaque cursors.
export function useArticles(
  category: string,
  day: string,
  search: string,
  enabled = true,
  sort: "latest" | "hottest" = "latest",
) {
  const b = useBackend();
  return useInfiniteQuery({
    queryKey: [b.base, b.instance, "articles", category, day, search, sort],
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
            ...(sort === "hottest" ? { sort } : {}),
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
    staleTime: 30_000,
    refetchInterval: seed ? 60_000 : false,
    refetchOnWindowFocus: false,
    placeholderData: keepPreviousData,
    queryFn: ({ signal }) =>
      request<Topology>(
        b.base,
        `topology?seed=${encodeURIComponent(seed || "")}`,
        undefined,
        signal,
      ),
  });
}

/** A delivery read never creates a task; only an already pending identity polls. */
export function useTopologySummary(seed: string | null, basis: string | null) {
  const b = useBackend();
  return useQuery({
    queryKey: [b.base, b.instance, "topology/summary", seed, basis],
    enabled: b.connected && !!seed && !!basis,
    queryFn: ({ signal }) =>
      request<TopologySummaryView>(
        b.base,
        `topology/summary?${new URLSearchParams({ seed: seed || "", basis: basis || "" })}`,
        undefined,
        signal,
      ),
    staleTime: 30_000,
    retry: false,
    refetchOnWindowFocus: false,
    refetchInterval: (query) =>
      !query.state.error &&
      query.state.data?.kind === "pending" &&
      query.state.dataUpdateCount < 60
        ? 1000
        : false,
  });
}
/** An explicit button action is the sole client generation authorization. */
export function useGenerateTopologySummary() {
  const b = useBackend();
  const client = useQueryClient();
  return useMutation({
    mutationFn: (key: TopologySummaryKey) =>
      request<TopologySummaryView>(b.base, "topology/summary/generate", key),
    retry: false,
    onSuccess: (view, key) =>
      client.setQueryData(
        [b.base, b.instance, "topology/summary", key.seed, key.basis],
        view,
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

export type { BriefStage } from "./live";

/** Read-only, bounded recorded actor neighborhood. */
export interface ActorAssociation {
  node: {
    id: string;
    label: string;
    revision: string;
    topology_seed: Seed | null;
  };
  relationship: {
    id: string;
    rationale: string;
    status: string;
    receipts: string[];
    actor_reference?: { role: string; binding_state: string };
  };
  recorded_at: string | null;
  article: Article | null;
}
export interface ActorDossier {
  id: string;
  display_name: string;
  subtype: string;
  identity_state: string;
  catalog_source_id: string | null;
  catalog_type: string | null;
  description: string | null;
  aliases: string[];
  metadata: { key: string; value: string | null }[];
  claims: ActorAssociation[];
  briefs: ActorAssociation[];
  inferences?: ActorAssociation[];
  loaded_inferences?: number;
  loaded_claims: number;
  loaded_briefs: number;
  gaps: string[];
  truncated: boolean;
}
/** Keep associations fresh when their originating brief finishes indexing. */
export function useAssociationLive(source?: {
  run_id: string;
  article_id: string;
}) {
  const b = useBackend();
  const client = useQueryClient();
  const run = source?.run_id;
  const id = source?.article_id;
  useEffect(() => {
    if (!b.connected || !b.live || !run || !id) return;
    return b.live.subscribe({ type: "brief", run, id, watch: "" }, (event) => {
      if (
        event.type !== "brief" ||
        !event.refresh ||
        event.run !== run ||
        event.id !== id
      )
        return;
      for (const resource of ["actor-dossier", "topology"]) {
        const filter = { queryKey: [b.base, b.instance, resource] };
        const wasFetching = client.isFetching(filter) > 0;
        void client
          .invalidateQueries(filter, { cancelRefetch: false })
          .then(() => {
            // A pre-publication read may still return the old snapshot.
            // Follow it once rather than cancelling its progress.
            if (wasFetching)
              return client.invalidateQueries(filter, { cancelRefetch: false });
          });
      }
    });
  }, [b.base, b.instance, b.connected, b.live, run, id, client]);
}

export function useActorDossier(id: string | null) {
  const b = useBackend();
  return useQuery({
    queryKey: [b.base, b.instance, "actor-dossier", id],
    refetchOnMount: "always",
    enabled: b.connected && !!id,
    staleTime: 30_000,
    refetchInterval: id ? 60_000 : false,
    refetchOnWindowFocus: false,
    queryFn: ({ signal }) =>
      request<ActorDossier>(
        b.base,
        `actors/detail?id=${encodeURIComponent(id || "")}`,
        undefined,
        signal,
      ),
  });
}
