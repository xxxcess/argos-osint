export type BriefStage = {
  operation: string;
  label: string;
  state: string;
  row: string;
  active: boolean;
  detail: string;
};
export type BriefProgress = {
  run: string;
  id: string;
  job_id: string | null;
  stages: BriefStage[];
  indexing: { state: string; reason: string } | null;
  complete: boolean;
  refresh: boolean;
};
export type LiveChannel =
  | { type: "metrics" }
  | { type: "brief"; run: string; id: string; watch: string };
export type LiveEvent =
  | { type: "status"; connected: boolean }
  | {
      type: "metrics";
      data: {
        cpu_percent: number | null;
        memory_percent: number | null;
        disk_percent: number | null;
        network_kbps: number | null;
        platform?: string | null;
        chipset?: string | null;
      };
    }
  | ({ type: "brief" } & BriefProgress)
  | { type: "error"; channel: "metrics" | "brief"; error: string };
/** Platform adapter; the browser implementation only knows the Next origin. */
export interface LiveTransport {
  subscribe(
    channel: LiveChannel,
    listener: (event: LiveEvent) => void,
  ): () => void;
}

/** Validate the wire boundary before data enters Query or component rendering. */
export function decodeLiveEvent(raw: string): LiveEvent | null {
  const value: unknown = JSON.parse(raw);
  if (!value || typeof value !== "object")
    throw new Error("Invalid live event");
  const event = value as Record<string, unknown>;
  const strings = (row: unknown, names: string[]) =>
    !!row &&
    typeof row === "object" &&
    names.every(
      (name) => typeof (row as Record<string, unknown>)[name] === "string",
    );
  if (event.type === "metrics") {
    const data = event.data as Record<string, unknown> | undefined;
    if (
      !data ||
      !["cpu_percent", "memory_percent", "disk_percent", "network_kbps"].every(
        (key) =>
          data[key] === null ||
          (typeof data[key] === "number" &&
            Number.isFinite(data[key]) &&
            data[key] >= 0),
      ) ||
      !["platform", "chipset"].every(
        (key) =>
          data[key] == null ||
          (typeof data[key] === "string" &&
            data[key].length > 0 &&
            data[key].length <= 64),
      )
    )
      throw new Error("Invalid metrics event");
  } else if (event.type === "brief") {
    if (
      !strings(event, ["run", "id"]) ||
      (event.job_id !== null && typeof event.job_id !== "string") ||
      typeof event.complete !== "boolean" ||
      typeof event.refresh !== "boolean" ||
      !Array.isArray(event.stages) ||
      !event.stages.every(
        (stage) =>
          strings(stage, ["operation", "label", "state", "row", "detail"]) &&
          typeof stage.active === "boolean",
      ) ||
      (event.indexing !== null && !strings(event.indexing, ["state", "reason"]))
    )
      throw new Error("Invalid brief event");
  } else if (event.type === "error") {
    if (
      !["metrics", "brief"].includes(String(event.channel)) ||
      typeof event.error !== "string"
    )
      throw new Error("Invalid error event");
  } else return null;
  return event as LiveEvent;
}
