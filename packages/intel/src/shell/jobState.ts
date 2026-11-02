import type { Brief } from "@argos/api-client";
import type { BriefProgress } from "@argos/api-client";
export type JobState =
  | "idle"
  | "queued"
  | "running"
  | "succeeded"
  | "failed"
  | "stale";
export function indexingJobState(state?: string): JobState {
  switch (state) {
    case "indexed":
      return "succeeded";
    case "failed":
    case "error":
    case "blocked":
      return "failed";
    case "queued":
    case "pending":
      return "queued";
    case "running":
    case "indexing":
    case "processing":
      return "running";
    case "stale":
    case "deferred":
      return "stale";
    default:
      return "idle";
  }
}
type Action = {
  status: string;
  submittedAt?: number;
  data?: { job_id: string };
};
export function deriveBriefJobState({
  ensure,
  reload,
  live,
  detail,
}: {
  ensure?: Action;
  reload?: Action;
  live?: { connected: boolean; progress?: BriefProgress };
  detail?: Pick<
    Brief,
    "status" | "lifecycle" | "indexing" | "publication" | "job_id"
  >;
}): JobState {
  const latest = [ensure, reload]
    .filter((a): a is Action => !!a)
    .sort((a, b) => (b.submittedAt ?? 0) - (a.submittedAt ?? 0))[0];
  if (latest?.status === "pending") {
    // A pending ensure for a brief that already has a run is an attach,
    // not a new chain. Reload still presents as queued while it is in flight.
    const revisitingFinished =
      latest === ensure && Boolean(detail?.job_id || detail?.publication);
    if (!revisitingFinished) return "queued";
  }
  if (latest?.status === "error") return "failed";
  const expectedJob = latest?.data?.job_id ?? detail?.job_id;
  const progress =
    live?.progress &&
    (!expectedJob ||
      !live.progress.job_id ||
      live.progress.job_id === expectedJob)
      ? live.progress
      : undefined;
  const states = progress
    ? [...progress.stages.map((s) => s.state), progress.indexing?.state ?? ""]
    : [
        detail?.status ?? "",
        detail?.lifecycle ?? "",
        detail?.indexing?.state ?? "",
      ];
  if (states.some((s) => /failed|error|blocked/i.test(s))) return "failed";
  if (progress?.stages.some((s) => s.active))
    return !progress.complete && !live?.connected ? "stale" : "running";
  const stageDone = (state: string) =>
    /completed|succeeded|done|failed|cancelled|superseded/i.test(state);
  if (
    progress &&
    progress.stages.length > 0 &&
    progress.stages.every((stage) => !stage.active && stageDone(stage.state))
  ) {
    return progress.stages.some((stage) => /failed/i.test(stage.state))
      ? "failed"
      : "succeeded";
  }
  if (progress && !progress.complete)
    return live?.connected ? "queued" : "stale";
  if (states.some((s) => /running|extracting|indexing|processing/i.test(s)))
    return "running";
  if (states.some((s) => /queued|pending/i.test(s))) return "queued";
  if (states.some((s) => /stale|deferred/i.test(s))) return "stale";
  if (
    (progress?.complete &&
      (progress.stages.some((stage) =>
        /completed|succeeded|done/i.test(stage.state),
      ) ||
        progress.indexing?.state === "indexed")) ||
    detail?.publication ||
    /^(complete|completed|succeeded|done)$/i.test(detail?.lifecycle ?? "")
  )
    return "succeeded";
  if (states.some((s) => /stale/i.test(s))) return "stale";
  return "idle";
}
