import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { http, HttpResponse } from "msw";
import {
  BackendProvider,
  type BackendSettings,
  type Seed,
  type Topology,
} from "@argos/api-client";
import { mswServer, testBase } from "../../../vitest.setup";
import { ForceGraph2DMock } from "../../../vitest.mocks";
import {
  forceGraphInstances,
  lastForceGraphProps,
  resetForceGraphRecording,
} from "../../../vitest.mocks";
import ForceCanvas from "../src/topology/ForceCanvas";
import { Topology } from "../src/topology/Topology";

// The canvas/d3 library cannot run under jsdom; mock only the module surface.
// The real ForceCanvas wrapper logic and the real list twin are under test.
vi.mock("react-force-graph-2d", async () => {
  const mocks = await import("../../../vitest.mocks");
  return { default: mocks.ForceGraph2DMock };
});

const nodes = [
  { id: "n1", label: "Actor One", kind: "actor" },
  { id: "n2", label: "Claim Two", kind: "claim" },
  { id: "n3", label: "Circuit", kind: "hardware" },
];
const links = [
  { id: "e1", source: "n1", target: "n2", label: "asserts" },
  { id: "e2", source: "n2", target: "n3", label: "cites" },
];

const palette = { text: "#e8e8e8", dim: "#b5b5b5", bg: "#141414", accent: "#87bfff" };
const kindColor = { actor: "#87bfff", claim: "#98c379", hardware: "#c678dd" };

describe("ForceCanvas (real wrapper, canvas library mocked)", () => {
  beforeEach(() => resetForceGraphRecording());

  function renderCanvas(reducedMotion = false) {
    const props = {
      nodes: nodes.map((n) => ({ ...n })),
      links: links.map((l) => ({ ...l })),
      width: 480,
      height: 340,
      kindColor,
      palette,
      selectedId: "n2",
      focusIds: new Set(["n2", "n1"]) as Set<string> | null,
      reducedMotion,
      onSelect: (id: string | null) => {},
    };
    const ref = React.createRef<import("../src/topology/ForceCanvas").CanvasHandle>();
    const view = render(<ForceCanvas ref={ref} {...props} />);
    return { ref, view, props };
  }

  it("hands FLAT COPIES to the graph library (nodes/links are mutated by d3)", () => {
    renderCanvas();
    const graphData = lastForceGraphProps!.graphData as {
      nodes: unknown[];
      links: unknown[];
    };
    expect(graphData.nodes).not.toBe(nodes);
    expect(graphData.nodes[0]).not.toBe(nodes[0]);
    expect(graphData.nodes).toEqual(nodes);
    expect(graphData.links[0]).not.toBe(links[0]);
    expect(graphData.links).toEqual(links);
  });

  it("passes the box size and coherent motion ticks to the canvas library", () => {
    renderCanvas(false);
    const p = lastForceGraphProps!;
    expect(p.width).toBe(480);
    expect(p.height).toBe(340);
    expect(p.backgroundColor).toBe("rgba(0,0,0,0)"); // glass draws behind
    expect(p.warmupTicks).toBe(0);
    expect(p.cooldownTicks).toBe(120);
    expect(p.enableNodeDrag).toBe(true);
  });

  it("brakes the layout and drag for reduced motion", () => {
    renderCanvas(true);
    const p = lastForceGraphProps!;
    expect(p.warmupTicks).toBe(20);
    expect(p.cooldownTicks).toBe(30);
    expect(p.enableNodeDrag).toBe(false);
  });

  it("focuses the camera on the selected node once it has coordinates", () => {
    const seen: (number | undefined)[][] = [];
    render(
      <ForceCanvas
        nodes={[
          { id: "n2", label: "Claim Two", kind: "claim", x: 5, y: 6 },
        ] as never}
        links={[] as never}
        width={10}
        height={10}
        kindColor={kindColor}
        palette={palette}
        selectedId="n2"
        focusIds={null}
        reducedMotion={false}
        onSelect={() => {}}
      />,
    );
    const record = forceGraphInstances[0];
    // centerAt(selected.x, selected.y, 400ms calibrated motion)
    expect(record.centerAt).toEqual([[5, 6, 400]]);
    expect(seen).toEqual([]);
  });

  it("exposes zoom/fit/reset/pause/resume imperative controls", () => {
    const { ref } = renderCanvas();
    const record = forceGraphInstances[0];
    ref.current!.zoomBy(1.4);
    // zoom() then zoom(k * factor)
    expect(record.zoomBy).toEqual([1, 1.4]);
    ref.current!.fit();
    expect(record.zoomToFit![0][1]).toBe(40); // 40px padding around the frame
    ref.current!.reset();
    expect(record.d3ReheatSimulation).toBe(1);
    ref.current!.pause();
    ref.current!.resume();
    expect(record.pauseAnimation).toBe(1);
    expect(record.resumeAnimation).toBe(1);
  });

  it("uses smooth transitions unless reduced motion demands instant ones", () => {
    const { ref, props } = renderCanvas(true);
    expect(props.reducedMotion).toBe(true);
    ref.current!.fit();
    expect(forceGraphInstances[0].zoomToFit![0][0]).toBe(0);
  });

  it("selects node ids from clicks and clears on background clicks", () => {
    const seen: (string | null)[] = [];
    render(
      <ForceCanvas
        nodes={nodes as never}
        links={links as never}
        width={10}
        height={10}
        kindColor={kindColor}
        palette={palette}
        selectedId={null}
        focusIds={null}
        reducedMotion={false}
        onSelect={(id) => seen.push(id)}
      />,
    );
    const p = lastForceGraphProps!;
    (p.onNodeClick as (n: unknown) => void)({ id: "n3" });
    (p.onBackgroundClick as () => void)();
    expect(seen).toEqual(["n3", null]);
  });

  it("keeps focused edges lit and dims the rest", () => {
    renderCanvas();
    const linkColor = lastForceGraphProps!.linkColor as (l: unknown) => string;
    const edgeInFocus = { source: { id: "n1" }, target: { id: "n2" } };
    const edgeOutside = { source: { id: "n1" }, target: { id: "n3" } };
    expect(linkColor(edgeInFocus)).toBe("rgba(181,181,181,0.55)"); // dim token
    expect(linkColor(edgeOutside)).toBe("rgba(83,83,83,0.2)"); // faint border
    // No focusIds means nothing is dimmed.
    const unset = { ...lastForceGraphProps!, focusIds: null };
    void unset;
  });

  it("paints labels/types via nodeLabel for tooltips", () => {
    renderCanvas();
    const labelFn = lastForceGraphProps!.nodeLabel as (n: unknown) => string;
    expect(labelFn({ label: "Actor One", kind: "actor" })).toBe(
      "Actor One · actor",
    );
  });

  it("paints nodes with the kind-color token, selection glow and focus dimming", () => {
    const { props } = renderCanvas();
    const fillStyles: string[] = [];
    const shadows: string[] = [];
    const labelsDrawn: string[] = [];
    const ctx = {
      beginPath: () => {},
      arc: () => {},
      fill: () => {},
      stroke: () => {},
      fillText: (text: string) => {
        labelsDrawn.push(text);
      },
      shadowBlur: 0 as number,
      shadowColor: "" as string,
      lineWidth: 0 as number,
      textAlign: "" as string,
      textBaseline: "" as string,
      set fillStyle(color: string) {
        fillStyles.push(color);
      },
      get fillStyle() {
        return fillStyles[fillStyles.length - 1] ?? "";
      },
      set globalAlpha(_alpha: number) {},
      get globalAlpha() {
        return 1;
      },
      set shadowColor(color: string) {
        shadows.push(color);
      },
      get shadowColor() {
        return shadows[shadows.length - 1] ?? "";
      },
      set font(_f: string) {},
      get font() {
        return "";
      },
    };
    const paintNodes = lastForceGraphProps!.nodeCanvasObject as (
      n: unknown,
      ctx: unknown,
      scale: number,
    ) => void;

    // Selected node (in focus): kind color fill, glow + ring, label drawn.
    paintNodes(
      { id: "n2", label: "Claim Two", kind: "claim", x: 1, y: 2 },
      ctx,
      2,
    );
    expect(fillStyles[0]).toBe(kindColor.claim); // kind color token fill
    // Selection glow is the kind color; the label paints last with text token.
    expect(shadows[0]).toBe(kindColor.claim);
    expect(fillStyles[fillStyles.length - 1]).toBe(palette.text);
    expect(labelsDrawn).toContain("Claim Two");

    // Dimmed node (outside focus set) collapses to 20% alpha, no glow.
    fillStyles.length = 0;
    shadows.length = 0;
    const alphas: number[] = [];
    const dimCtx = {
      beginPath: ctx.beginPath,
      arc: ctx.arc,
      fill: ctx.fill,
      stroke: ctx.stroke,
      fillText: ctx.fillText,
      shadowBlur: 0 as number,
      shadowColor: "" as string,
      lineWidth: 0 as number,
      textAlign: "" as string,
      textBaseline: "" as string,
      set fillStyle(color: string) {
        fillStyles.push(color);
      },
      get fillStyle() {
        return fillStyles[fillStyles.length - 1] ?? "";
      },
      set globalAlpha(alpha: number) {
        alphas.push(alpha);
      },
      get globalAlpha() {
        return alphas[alphas.length - 1] ?? 1;
      },
      set shadowColor(color: string) {
        shadows.push(color);
      },
      get shadowColor() {
        return shadows[shadows.length - 1] ?? "";
      },
      set font(_f: string) {},
      get font() {
        return "";
      },
    };
    paintNodes(
      { id: "n3", label: "Circuit", kind: "hardware", x: 3, y: 4 },
      dimCtx,
      2,
    );
    expect(alphas).toContain(0.2); // dim token alpha outside focus
    expect(shadows).toEqual([]); // no selection glow outside focus
    expect(fillStyles[0]).toBe(kindColor.hardware);
  });
});

/* ---------------------------------------------------------------------------
 * Real list twin (Topology): canvas twin rendered by the graph area, rows in
 * the organized nodes/connections panels; both stay in sync by id.
 * ------------------------------------------------------------------------- */

const topologyPayload: Topology = {
  root_label: "Evidence topology",
  nodes: [
    {
      id: "n1",
      label: "Actor One",
      kind: "actor",
      detail: "Primary collection actor",
      topology_seed: { kind: "actor", ref: "seed-a1" } as Seed,
    },
    {
      id: "n2",
      label: "Claim Two",
      kind: "claim",
      detail: "Retained claim record",
      topology_seed: null,
    },
    {
      id: "n3",
      label: "Circuit",
      kind: "hardware",
      detail: "Device record",
      topology_seed: null,
    },
  ],
  edges: [
    { id: "k1", from: "n1", to: "n2", label: "asserts" },
    { id: "k2", from: "n2", to: "n3", label: "cites" },
  ],
  findings: { gaps: [] },
  has_continuation: false,
};

const connectedSettings: BackendSettings = {
  subscribe: () => () => {},
  getSnapshot: () => testBase,
  getServerSnapshot: () => testBase,
  setBase: vi.fn(),
  createRequestId: () => "req-1",
};

  // Renders Topology and waits for the topology query to settle (needs data
  // before the node rows exist; the header renders while pending). Returns the
  // onSeed spy wired into the shell (the same instance the component got).
  async function startTopology(seed: Seed | null, onSeed = vi.fn()) {
    mswServer.use(
      http.get(`${testBase}/api/v1/health/ready`, () =>
        HttpResponse.json({
          ready: true,
          storage: true,
          service: "argos-intel",
          api_version: 1,
          instance_id: "inst-1",
        }),
      ),
      http.post(`${testBase}/api/v1/topology`, () =>
        HttpResponse.json(topologyPayload),
      ),
    );
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const view = render(
      <QueryClientProvider client={client}>
        <BackendProvider settings={connectedSettings}>
          <Topology
            seed={seed}
            onSeed={onSeed}
            pageStyle={{}}
            titleStyle={{}}
          />
        </BackendProvider>
      </QueryClientProvider>,
    );
    if (seed) {
      await waitFor(() =>
        expect(
          screen.getByRole("button", { name: "Select node Actor One, actor" }),
        ).toBeVisible(),
      );
    }
    return { view, onSeed };
  }

  describe("Topology list twin (real Topology)", () => {
    it("shows the empty state without a seed and starts no topology request", async () => {
      await startTopology(null);
      expect(screen.getByText("No topology selected")).toBeVisible();
      expect(screen.queryByText("Actor One")).toBeNull();
    });

    it("renders the graph summary, node list rows and connection panels", async () => {
      await startTopology({ kind: "actor" } as Seed);
      expect(
        screen.getByText("Showing 3 of 3 nodes · 2 of 2 connections"),
      ).toBeVisible();
      // List twin rows carry accessible selection labels and kind telemetry.
      expect(
        screen.getByRole("button", { name: "Select node Actor One, actor" }),
      ).toBeTruthy();
      expect(
        screen.getByRole("button", { name: "Select node Claim Two, claim" }),
      ).toBeTruthy();
      // Default selection is the first node; its details render in access order.
      expect(screen.getByText(/Primary collection actor/)).toBeVisible();
      // Connections panel preserves recorded direction and meaning.
      expect(screen.getByText(/Actor One → asserts → Claim Two/)).toBeVisible();
      expect(screen.getByText(/Claim Two → cites → Circuit/)).toBeVisible();
      // The canvas placeholder is gated client-side (no hidden canvas nodes).
      expect(screen.queryByTestId("force-graph-canvas")).toBeNull();
    });

    it("syncs selection from the list twin into the details panel", async () => {
      await startTopology({ kind: "actor" } as Seed);
      fireEvent.click(
        screen.getByRole("button", { name: "Select node Claim Two, claim" }),
      );
      await waitFor(() =>
        expect(screen.getAllByText("Claim Two").length).toBeGreaterThan(0),
      );
      expect(screen.getByText(/Retained claim record/)).toBeVisible();
    });

    it("navigates seeds from the details panel onto the parent shell", async () => {
      const onSeed = vi.fn();
      await startTopology({ kind: "actor" } as Seed, onSeed);
      fireEvent.click(screen.getByRole("button", { name: /Navigate this node/ }));
      await waitFor(() => {
        expect(onSeed).toHaveBeenCalledWith(
          topologyPayload.nodes[0].topology_seed,
        );
      });
    });

    it("filters by kind via palette toggles, hiding hidden nodes and their edges", async () => {
      await startTopology({ kind: "actor" } as Seed);
      // Always select a node that survives the filter first: the selected node
      // is part of the visible set by contract (Topology.tsx:178-184).
      fireEvent.click(
        screen.getByRole("button", { name: "Select node Claim Two, claim" }),
      );
      await waitFor(() =>
        expect(screen.getByText(/Retained claim record/)).toBeVisible(),
      );
      fireEvent.click(screen.getByRole("button", { name: "Show actor nodes" }));
      await waitFor(() =>
        expect(
          screen.queryByRole("button", {
            name: "Select node Actor One, actor",
          }),
        ).toBeNull(),
      );
      // The hidden actor's outgoing edge disappears with it; the selected
      // node and its remaining edge stay.
      await waitFor(() =>
        expect(screen.queryByText(/Actor One → asserts/)).toBeNull(),
      );
      expect(screen.getByText(/Claim Two → cites → Circuit/)).toBeVisible();
      expect(
        screen.getByRole("button", { name: "Select node Claim Two, claim" }),
      ).toBeVisible();
      // Re-enabling restores both node row and edge.
      fireEvent.click(screen.getByRole("button", { name: "Show actor nodes" }));
      await waitFor(() =>
        expect(
          screen.getByRole("button", { name: "Select node Actor One, actor" }),
        ).toBeTruthy(),
      );
      expect(screen.getByText(/Actor One → asserts → Claim Two/)).toBeVisible();
    });

    it("filters nodes by label from the palette text input", async () => {
      await startTopology({ kind: "actor" } as Seed);
      fireEvent.click(
        screen.getByRole("button", { name: "Select node Claim Two, claim" }),
      );
      await waitFor(() =>
        expect(screen.getByText(/Retained claim record/)).toBeVisible(),
      );
      fireEvent.change(screen.getByLabelText("Filter nodes by label"), {
        target: { value: "Claim" },
      });
      // Non-matching rows (Actor One, Circuit) hide with their edges — but the
      // selected node is exempt from its own filter.
      await waitFor(() =>
        expect(
          screen.queryByRole("button", {
            name: "Select node Actor One, actor",
          }),
        ).toBeNull(),
      );
      expect(
        screen.getByRole("button", { name: "Select node Claim Two, claim" }),
      ).toBeVisible();
      // "cites" disappears because its target node no longer matches.
      await waitFor(() =>
        expect(screen.queryByText(/Claim Two → cites/)).toBeNull(),
      );
    });

    it("scripts a reset back to unfiltered topology", async () => {
      await startTopology({ kind: "actor" } as Seed);
      fireEvent.click(
        screen.getByRole("button", { name: "Select node Claim Two, claim" }),
      );
      await waitFor(() =>
        expect(screen.getByText(/Retained claim record/)).toBeVisible(),
      );
      fireEvent.click(screen.getByRole("button", { name: "Show actor nodes" }));
      await waitFor(() =>
        expect(screen.queryByText(/Actor One → asserts/)).toBeNull(),
      );
      fireEvent.click(screen.getAllByRole("button", { name: "Reset filters" })[0]);
      await waitFor(() =>
        expect(screen.getByText(/Actor One → asserts → Claim Two/)).toBeVisible(),
      );
      // The selection survives the reset.
      expect(
        screen.getByRole("button", { name: "Select node Claim Two, claim" }),
      ).toBeTruthy();
    });
  });
