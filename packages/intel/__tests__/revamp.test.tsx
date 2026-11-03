import { StageContent } from "../src/views/brief/StageContent";
import React from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Text } from "react-native";
import { Skeleton } from "@argos/ui";
import type { BriefProgress, Brief } from "@argos/api-client";
import { deriveBriefJobState, indexingJobState } from "../src/shell/jobState";
import { JobStatusBeacon } from "../src/shell/JobStatusBeacon";
import {
  ScreenTransition,
  transitionDirection,
} from "../src/shell/ScreenTransition";
import { BriefSection } from "../src/views/brief/BriefSection";
import { InspectorPresence } from "../src/topology/InspectorPresence";
import { labelZoomOpacity } from "../src/topology/labelLayout";
import { OrbitScene } from "../src/landing/OrbitScene";
import { spawnOrbiter, stepOrbiter } from "../src/landing/orbit";
import { intelColors } from "../src/shell/intel-theme";
const progress = (overrides: Partial<BriefProgress> = {}): BriefProgress => ({
  run: "r",
  id: "a",
  job_id: "j",
  stages: [],
  indexing: null,
  complete: false,
  refresh: false,
  ...overrides,
});
const detail = (overrides: Partial<Brief> = {}) => ({
  status: "",
  lifecycle: "",
  publication: null,
  indexing: null,
  ...overrides,
});
afterEach(() => vi.restoreAllMocks());
describe("brief job beacon", () => {
  it.each([
    [{}, "idle"],
    [{ ensure: { status: "pending" } }, "queued"],
    [
      {
        ensure: { status: "pending" },
        detail: detail({ job_id: "job-r1", publication: [1, {}, {}] }),
      },
      "succeeded",
    ],
    [{ reload: { status: "error" } }, "failed"],
    [{ detail: detail({ status: "Extracting" }) }, "running"],
    [{ detail: detail({ status: "queued" }) }, "queued"],
    [{ detail: detail({ status: "failed" }) }, "failed"],
    [{ detail: detail({ status: "stale" }) }, "stale"],
    [{ detail: detail({ publication: [1, {}, {}] }) }, "succeeded"],
    [{ detail: detail({ lifecycle: "complete" }) }, "succeeded"],
    [{ detail: detail({ lifecycle: "completed" }) }, "succeeded"],
    [{ live: { connected: false, progress: progress() } }, "stale"],
    [{ live: { connected: true, progress: progress() } }, "queued"],
    [
      {
        live: {
          connected: true,
          progress: progress({
            complete: false,
            stages: [
              {
                operation: "body",
                label: "Full article",
                state: "completed",
                row: "Completed",
                active: false,
                detail: "",
              },
              {
                operation: "links",
                label: "Links",
                state: "completed",
                row: "Completed",
                active: false,
                detail: "",
              },
            ],
          }),
        },
      },
      "succeeded",
    ],
    [
      { live: { connected: true, progress: progress({ complete: true }) } },
      "idle",
    ],
    [
      {
        live: {
          connected: true,
          progress: progress({
            stages: [
              {
                operation: "extract",
                label: "Extract",
                state: "running",
                row: "working",
                active: true,
                detail: "",
              },
            ],
          }),
        },
      },
      "running",
    ],
    [
      {
        live: {
          connected: true,
          progress: progress({
            indexing: { state: "failed", reason: "index unavailable" },
          }),
        },
      },
      "failed",
    ],
  ] as const)("maps %j to %s", (input, state) =>
    expect(deriveBriefJobState(input)).toBe(state),
  );
  it.each([
    [undefined, "idle"],
    ["not_indexed", "idle"],
    ["indexed", "succeeded"],
    ["pending", "queued"],
    ["indexing", "running"],
    ["failed", "failed"],
    ["deferred", "stale"],
  ] as const)("shows indexing %s as %s", (state, expected) => {
    expect(indexingJobState(state)).toBe(expected);
  });
  it("keeps terminal stages complete after their live connection closes", () => {
    expect(
      deriveBriefJobState({
        live: {
          connected: false,
          progress: progress({
            stages: [
              {
                operation: "links",
                label: "Links",
                state: "completed",
                row: "Completed",
                active: false,
                detail: "",
              },
            ],
          }),
        },
      }),
    ).toBe("succeeded");
  });
  it("lets a newer retry supersede an older failed mutation", () =>
    expect(
      deriveBriefJobState({
        ensure: { status: "error", submittedAt: 10 },
        reload: { status: "pending", submittedAt: 20 },
      }),
    ).toBe("queued"));
  it("announces live state changes with visible text", () => {
    const { rerender } = render(<JobStatusBeacon state="running" />);
    expect(screen.getByRole("status")).toHaveTextContent("Brief running");
    rerender(<JobStatusBeacon state="failed" />);
    expect(screen.getByRole("status")).toHaveAttribute("aria-live", "polite");
    expect(screen.getByText("Brief failed")).toBeVisible();
  });
});
describe("loading and transitions", () => {
  it("hides skeleton geometry while its parent announces loading", () => {
    render(
      <div role="status" aria-label="Loading evidence">
        <Skeleton lines={3} />
      </div>,
    );
    expect(
      screen.getByRole("status", { name: "Loading evidence" }),
    ).toBeVisible();
    expect(screen.getAllByTestId("skeleton-line")).toHaveLength(3);
    expect(
      screen.getAllByTestId("skeleton-line")[0].parentElement,
    ).toHaveAttribute("aria-hidden", "true");
  });
  it("collapses and restores a section using the keyboard", async () => {
    render(
      <BriefSection title="CLAIMS" count={2}>
        <Text>Retained evidence</Text>
      </BriefSection>,
    );
    const button = screen.getByRole("button", { name: "CLAIMS" });
    await userEvent.click(button);
    expect(button).toHaveAttribute("aria-expanded", "false");
    expect(
      screen.getByText("Retained evidence").closest('[aria-hidden="true"]'),
    ).not.toBeNull();
    await userEvent.click(button);
    expect(button).toHaveAttribute("aria-expanded", "true");
  });
  it("assigns forward and backward movement through the workspace", () => {
    expect(transitionDirection("bulletin", "brief")).toBe(1);
    expect(transitionDirection("topology", "brief")).toBe(-1);
  });
  it("swaps immediately with reduced motion without retaining the previous screen", async () => {
    vi.spyOn(window, "matchMedia").mockReturnValue({
      matches: true,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    } as unknown as MediaQueryList);
    const { rerender } = render(
      <ScreenTransition screen="bulletin">
        <Text>Bulletin body</Text>
      </ScreenTransition>,
    );
    await act(async () => {});
    rerender(
      <ScreenTransition screen="brief">
        <Text>Brief body</Text>
      </ScreenTransition>,
    );
    expect(screen.queryByText("Bulletin body")).toBeNull();
    expect(screen.getByText("Brief body")).toBeVisible();
  });
  it("retains the mounted outgoing screen without replaying effects", async () => {
    vi.spyOn(window, "matchMedia").mockReturnValue({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    } as unknown as MediaQueryList);
    const mounted = vi.fn();
    function Body() {
      React.useEffect(() => {
        mounted();
      }, []);
      return <Text>Outgoing bulletin</Text>;
    }
    const { rerender } = render(
      <ScreenTransition screen="bulletin">
        <Body />
      </ScreenTransition>,
    );
    await act(async () => {});
    rerender(
      <ScreenTransition screen="brief">
        <Text>New brief</Text>
      </ScreenTransition>,
    );
    expect(mounted).toHaveBeenCalledTimes(1);
    expect(screen.getByText("New brief")).toBeVisible();
  });
  it("keeps all Intel text tokens readable against raised matte surfaces", () => {
    const luminance = (hex: string) => {
      const values = hex
        .slice(1)
        .match(/../g)!
        .map((part) => parseInt(part, 16) / 255)
        .map((v) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
      return values[0] * 0.2126 + values[1] * 0.7152 + values[2] * 0.0722;
    };
    for (const foreground of [
      intelColors.text,
      intelColors.dim,
      intelColors.muted,
      intelColors.faint,
    ])
      for (const background of [
        intelColors.bg,
        intelColors.panel,
        intelColors.panelRaised,
      ])
        expect(
          (luminance(foreground) + 0.05) / (luminance(background) + 0.05),
        ).toBeGreaterThanOrEqual(4.5);
  });
});
describe("connection orbit", () => {
  it("spawns deterministically on the far sphere", () => {
    const o = spawnOrbiter(42);
    expect(o).toEqual(spawnOrbiter(42));
    expect(Math.hypot(o.x, o.y, o.z)).toBeCloseTo(600);
    expect(o.target).toBeGreaterThanOrEqual(80);
    expect(o.target).toBeLessThanOrEqual(160);
  });
  it("approaches the ring monotonically while angular velocity increases", () => {
    let o = spawnOrbiter(42);
    for (let i = 0; i < 20; i++) {
      const next = stepOrbiter(o, 0.25);
      expect(next.radius).toBeLessThanOrEqual(o.radius);
      expect(next.radius).toBeGreaterThanOrEqual(o.target);
      expect(next.speed).toBeGreaterThanOrEqual(o.speed);
      o = next;
    }
    expect(o.radius).toBe(o.target);
    expect(o.speed).toBeCloseTo(1.2);
  });
  it("gives the same position regardless of frame partitioning", () => {
    const o = spawnOrbiter(7);
    let split = o;
    for (let i = 0; i < 60; i++) split = stepOrbiter(split, 1 / 60);
    const once = stepOrbiter(o, 1);
    expect(split.x).toBeCloseTo(once.x);
    expect(split.y).toBeCloseTo(once.y);
    expect(split.z).toBeCloseTo(once.z);
  });
  it("has an accessible status and decorative fallback without WebGL", async () => {
    render(<OrbitScene />);
    await waitFor(() =>
      expect(
        screen.getByRole("status", { name: "Checking local API…" }),
      ).toBeVisible(),
    );
    expect(
      screen.getByTestId("landing-fallback").closest('[aria-hidden="true"]'),
    ).not.toBeNull();
  });
});

describe("topology presence", () => {
  it("retains selected details until exit finishes and then shows the empty prompt", async () => {
    vi.spyOn(window, "matchMedia").mockReturnValue({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    } as unknown as MediaQueryList);
    const { rerender } = render(
      <InspectorPresence visible>
        <Text>Selected actor</Text>
      </InspectorPresence>,
    );
    rerender(
      <InspectorPresence visible={false}>
        <Text>Choose a node</Text>
      </InspectorPresence>,
    );
    expect(
      screen.getByText("Selected actor").closest('[aria-hidden="true"]'),
    ).not.toBeNull();
    await screen.findByText("Choose a node");
    expect(screen.queryByText("Selected actor")).toBeNull();
  });
  it("fades labels progressively at far zoom distances", () => {
    expect(labelZoomOpacity(300)).toBe(1);
    expect(labelZoomOpacity(575)).toBe(0.5);
    expect(labelZoomOpacity(900)).toBe(0);
  });
});

// @tag:testing — stage state wins over stale retained content.
describe("section stage state", () => {
  it.each(["queued", "running", "retry_scheduled"])(
    "loads %s stages",
    async (state) => {
      render(
        <StageContent
          kind="inferences"
          stage={{
            operation: "intel_brief_inferences",
            label: "Inferences",
            state,
            row: state,
            detail: "",
            active: state === "running",
          }}
        >
          old content
        </StageContent>,
      );
      await act(async () => {});
      expect(
        screen.getByRole("status", { name: `Inferences: ${state}` }),
      ).toBeVisible();
      expect(screen.queryByText("old content")).toBeNull();
    },
  );
  it("shows failure details instead of old output", () => {
    render(
      <StageContent
        kind="claims"
        stage={{
          operation: "intel_brief_claims",
          label: "Claims",
          state: "failed",
          row: "Failed",
          detail: "Provider unavailable",
          active: false,
        }}
      >
        old content
      </StageContent>,
    );
    expect(screen.getByRole("alert")).toHaveTextContent("Provider unavailable");
    expect(screen.queryByText("old content")).toBeNull();
  });
  it("shows completed content", () => {
    render(
      <StageContent
        kind="claims"
        stage={{
          operation: "intel_brief_claims",
          label: "Claims",
          state: "completed",
          row: "Done",
          detail: "",
          active: false,
        }}
      >
        completed content
      </StageContent>,
    );
    expect(screen.getByText("completed content")).toBeVisible();
  });
});

describe("terminal stage diagnostics", () => {
  it.each(["failed", "cancelled", "superseded", "blocked"])(
    "never leaves %s work showing a permanent skeleton",
    (state) => {
      render(
        <StageContent
          stage={{
            operation: "intel_brief_links",
            label: "Links",
            state,
            row: "",
            active: false,
            detail: "Stage cannot continue",
          }}
          kind="inferences"
        >
          <Text>Old links</Text>
        </StageContent>,
      );
      expect(screen.getByRole("alert")).toHaveTextContent(
        "Links: Stage cannot continue",
      );
      expect(screen.queryByText("Old links")).toBeNull();
      expect(screen.queryByRole("status")).toBeNull();
    },
  );
  it("announces a blocked dependency even while the stage is queued", () => {
    render(
      <StageContent
        stage={{
          operation: "intel_brief_inferences",
          label: "Inferences",
          state: "queued",
          row: "Blocked by Claims",
          active: false,
          detail: "",
        }}
        kind="inferences"
      >
        <Text>Old inference</Text>
      </StageContent>,
    );
    expect(screen.getByRole("alert")).toHaveTextContent("Blocked by Claims");
    expect(screen.queryByRole("status")).toBeNull();
  });
});
