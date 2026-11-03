import { describe, expect, it } from "vitest";
import { decodeLiveEvent } from "../src/live";

const metrics = {
  cpu_percent: 0,
  memory_percent: null,
  disk_percent: 100,
  network_kbps: 0,
};
const progress = {
  type: "brief",
  run: "run",
  id: "article",
  job_id: "job",
  stages: [
    {
      operation: "intel_brief_claims",
      label: "Claims",
      state: "running",
      row: "Running",
      active: true,
      detail: "Waiting for model",
    },
  ],
  indexing: { state: "indexing", reason: "Queued" },
  complete: false,
  refresh: true,
};
const decode = (event: unknown) => decodeLiveEvent(JSON.stringify(event));

describe("live wire contracts consumed by the UI", () => {
  it("accepts zero, unavailable metrics and optional platform/chipset labels", () => {
    for (const data of [
      metrics,
      { ...metrics, platform: null, chipset: null },
      { ...metrics, platform: "macOS", chipset: "arm" },
    ]) {
      expect(decode({ type: "metrics", data })).toEqual({
        type: "metrics",
        data,
      });
    }
  });
  it.each(["cpu_percent", "memory_percent", "disk_percent", "network_kbps"])(
    "rejects invalid %s before it reaches rendering",
    (key) => {
      for (const value of [-1, "0", {}, undefined]) {
        expect(() =>
          decode({ type: "metrics", data: { ...metrics, [key]: value } }),
        ).toThrow("Invalid metrics event");
      }
    },
  );
  it.each(["platform", "chipset"])(
    "rejects unsafe %s types and unbounded labels",
    (key) => {
      for (const value of [42, {}, "", "x".repeat(65)]) {
        expect(() =>
          decode({ type: "metrics", data: { ...metrics, [key]: value } }),
        ).toThrow("Invalid metrics event");
      }
    },
  );
  it("preserves the complete stage identity and nullable job/index receipts", () => {
    expect(decode(progress)).toEqual(progress);
    const empty = { ...progress, job_id: null, indexing: null, stages: [] };
    expect(decode(empty)).toEqual(empty);
  });
  it.each([
    { job_id: 4 },
    { complete: "false" },
    { refresh: 1 },
    { stages: {} },
    { stages: [{ ...progress.stages[0], active: "true" }] },
    { stages: [{ ...progress.stages[0], detail: null }] },
    { indexing: { state: "failed" } },
  ])("rejects malformed brief progress %j", (patch) => {
    expect(() => decode({ ...progress, ...patch })).toThrow(
      "Invalid brief event",
    );
  });
  it("ignores heartbeat/unknown messages and rejects invalid JSON and error channels", () => {
    expect(decode({ type: "heartbeat" })).toBeNull();
    expect(decode({ type: "future-event" })).toBeNull();
    expect(() => decodeLiveEvent("{")).toThrow();
    expect(() => decode(null)).toThrow("Invalid live event");
    expect(() =>
      decode({ type: "error", channel: "unknown", error: "Failure" }),
    ).toThrow("Invalid error event");
    expect(
      decode({ type: "error", channel: "brief", error: "Failure" }),
    ).toEqual({ type: "error", channel: "brief", error: "Failure" });
  });
});
