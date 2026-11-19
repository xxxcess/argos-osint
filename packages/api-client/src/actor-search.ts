// @tag:discovery-services — Browser view of `GET /api/v1/actors/search?mode=discovery`.
// The gateway already applies canonical eligibility; these helpers only keep the
// page honest about what it received. No identity is derived from a label or alias.

export const ACTOR_SEARCH_LIMIT = 10;
export const ACTOR_SEARCH_MAX_BYTES = 256;
export const ACTOR_SEARCH_MAX_TERMS = 16;
const ACTOR_ID_MAX_BYTES = 256;
const bytes = (value: string) => new TextEncoder().encode(value).length;
const CONTROL = /[\u0000-\u001f\u007f]/;

/** Index states the gateway reports; any other spelling is treated as `unknown`. */
export type ActorIndexState =
  | "ready"
  | "indexing"
  | "paused"
  | "throttled"
  | "stale"
  | "unavailable"
  | "not_built"
  | "degraded"
  | "unknown";

/** A candidate after the gateway's canonical filtering; `actor_id` is the only identity. */
export interface ActorSearchHit {
  actor_id: string;
  label: string;
  summary?: string | null;
  matched_field?: string | null;
  matched_alias?: string | null;
  actor_type?: string | null;
  countries?: string[];
  projection_revision?: number;
  index_generation?: string;
  /** Retrieval rank only; never confidence or identity. */
  score?: number;
}

export interface ActorSearchResponse {
  rows: ActorSearchHit[];
  index: { state: string; reason_code?: string | null };
  truncated: boolean;
  continuation?: string | null;
}

/** Same plain-query limits as the gateway, plus its no-control-character rule. */
export function actorSearchProblem(raw: string): string | null {
  const query = raw.trim();
  if (!query) return "Enter a word or phrase to search.";
  if (bytes(query) > ACTOR_SEARCH_MAX_BYTES)
    return `Use at most ${ACTOR_SEARCH_MAX_BYTES} bytes.`;
  if (CONTROL.test(query)) return "Remove control characters.";
  if (query.split(/\s+/).length > ACTOR_SEARCH_MAX_TERMS)
    return `Use at most ${ACTOR_SEARCH_MAX_TERMS} words.`;
  return null;
}

/** A canonical id is opaque: never trimmed or rewritten, only rejected when unusable as a route id. */
export function canonicalActorId(value: unknown): string | null {
  if (typeof value !== "string" || !value.trim()) return null;
  if (CONTROL.test(value) || bytes(value) > ACTOR_ID_MAX_BYTES) return null;
  return value;
}

/**
 * Keep the gateway's rank order, drop hits that cannot be opened by canonical
 * id and collapse repeats of one canonical id onto its best-ranked hit.
 */
export function actorHits(rows: readonly ActorSearchHit[] | undefined) {
  const seen = new Set<string>();
  const out: ActorSearchHit[] = [];
  for (const hit of rows ?? []) {
    const id = canonicalActorId(hit?.actor_id);
    if (!id || typeof hit.label !== "string" || !hit.label.trim()) continue;
    if (seen.has(id)) continue;
    seen.add(id);
    out.push(hit);
  }
  return out;
}

const nameKey = (label: string) => label.normalize("NFKC").trim().toLowerCase();

/** What a result row shows; `id` is set only when another hit shares the name. */
export interface ActorHitView {
  hit: ActorSearchHit;
  detail: string;
  match: string;
  id?: string;
}

/** Same-name actors differ by canonical id, type and countries, never by label alone. */
export function actorHitViews(hits: readonly ActorSearchHit[]): ActorHitView[] {
  const names = new Map<string, number>();
  for (const hit of hits)
    names.set(nameKey(hit.label), (names.get(nameKey(hit.label)) ?? 0) + 1);
  return hits.map((hit) => ({
    hit,
    detail: [hit.actor_type, ...(hit.countries ?? [])]
      .filter((part): part is string => !!part?.trim())
      .join(" · "),
    match: hit.matched_alias
      ? `Matched alias “${hit.matched_alias}”`
      : hit.matched_field
        ? `Matched ${hit.matched_field}`
        : "",
    ...((names.get(nameKey(hit.label)) ?? 0) > 1 ? { id: hit.actor_id } : {}),
  }));
}

/** `ready` is complete; `partial` shows rows with a caveat; `unavailable` shows none. */
export type ActorAvailability =
  | { kind: "ready" }
  | { kind: "partial"; state: ActorIndexState }
  | { kind: "unavailable"; state: ActorIndexState };

export function actorAvailability(
  response: Pick<ActorSearchResponse, "index"> | undefined,
): ActorAvailability {
  const state = response?.index?.state;
  switch (state) {
    case "ready":
      return { kind: "ready" };
    case "indexing":
    case "paused":
    case "throttled":
    case "stale":
    case "degraded":
      return { kind: "partial", state };
    case "unavailable":
    case "not_built":
      return { kind: "unavailable", state };
    default:
      return { kind: "unavailable", state: "unknown" };
  }
}
