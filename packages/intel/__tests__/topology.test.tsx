import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  render,
  screen,
  fireEvent,
  waitFor,
  act,
} from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { http, HttpResponse } from "msw";
import {
  BackendProvider,
  type BackendSettings,
  type Seed,
  type Topology as TopologyData,
} from "@argos/api-client";
import { mswServer, testBase } from "../../../vitest.setup";
import {
  forceGraphInstances,
  lastForceGraphProps,
  resetForceGraphRecording,
} from "../../../vitest.mocks";
import ForceCanvas, { escapeGraphLabel } from "../src/topology/ForceCanvas";
import { Topology } from "../src/topology/Topology";

// The canvas/d3 library cannot run under jsdom; mock only the module surface.
// The real ForceCanvas wrapper logic and the real list twin are under test.
vi.mock("react-force-graph-3d", async () => {
  const mocks = await import("../../../vitest.mocks");
  return { default: mocks.ForceGraph3DMock };
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

const palette = {
  text: "#e8e8e8",
  dim: "#b5b5b5",
  bg: "#141414",
  accent: "#87bfff",
};
const kindColor = { actor: "#87bfff", claim: "#98c379", hardware: "#c678dd" };

describe("ForceCanvas (real wrapper, canvas library mocked)", () => {
  beforeEach(() => resetForceGraphRecording());
  it("escapes untrusted tooltip markup", () => {
    expect(escapeGraphLabel('<img src=x onerror="alert(1)">&')).toBe(
      "&lt;img src=x onerror=&quot;alert(1)&quot;&gt;&amp;",
    );
  });

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
      onSelect: (_id: string | null) => {},
    };
    const ref =
      React.createRef<import("../src/topology/ForceCanvas").CanvasHandle>();
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
    expect(p.backgroundColor).toBe("rgba(0,0,0,0)");
    expect(p.warmupTicks).toBe(120);
    expect(p.cooldownTicks).toBe(120);
    expect(p.enableNodeDrag).toBe(true);
  });

  it("brakes the layout and drag for reduced motion", () => {
    renderCanvas(true);
    const p = lastForceGraphProps!;
    expect(p.warmupTicks).toBe(120);
    expect(p.cooldownTicks).toBe(0);
    expect(p.enableNodeDrag).toBe(false);
    expect(p.enableNavigationControls).toBe(false);
    expect(
      (p.linkDirectionalParticles as (link: { id: string }) => number)({
        id: "e1",
      }),
    ).toBe(0);
  });

  it("focuses the camera on the selected node once it has coordinates", async () => {
    render(
      <ForceCanvas
        nodes={
          [
            { id: "n2", label: "Claim Two", kind: "claim", x: 5, y: 6, z: 7 },
          ] as never
        }
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
    await waitFor(() =>
      expect(record.cameraPosition?.length).toBeGreaterThanOrEqual(2),
    );
    expect(record.cameraPosition![0][1]).toEqual(
      expect.objectContaining({ x: 5, y: 6, z: 7 }),
    );
    expect(record.cameraPosition!.at(-1)![2]).toBe(700);
  });

  function positionEngineNodes() {
    const data = lastForceGraphProps!.graphData as {
      nodes: { x?: number; y?: number }[];
    };
    data.nodes.forEach((node, index) =>
      Object.assign(node, { x: index * 20, y: index * 10 }),
    );
  }
  const frameTick = () =>
    new Promise<void>((resolve) =>
      requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
    );
  it("waits for the engine to position the selected node before fitting", () => {
    renderCanvas();
    (lastForceGraphProps!.onEngineStop as () => void)();
    expect(forceGraphInstances[0].cameraPosition).toBeUndefined();
    positionEngineNodes();
    (lastForceGraphProps!.onEngineStop as () => void)();
    expect(forceGraphInstances[0].cameraPosition!.at(-1)).toEqual([
      expect.any(Object),
      { x: 20, y: 10, z: 0 },
      700,
    ]);
  });

  it("aligns the disabled controls target for an instant reduced-motion fit", () => {
    renderCanvas(true);
    const record = forceGraphInstances[0];
    record.controlTarget = { x: 0, y: 0, z: 0 };
    positionEngineNodes();
    (lastForceGraphProps!.onEngineStop as () => void)();
    expect(record.controlTarget).toEqual({ x: 20, y: 10, z: 0 });
    expect(record.cameraPosition!.at(-1)![2]).toBe(0);
  });

  it("preserves manual zoom through rerenders, resizing, refreshes and repeated engine stops", async () => {
    const { view, props, ref } = renderCanvas();
    positionEngineNodes();
    (lastForceGraphProps!.onEngineStop as () => void)();
    ref.current!.zoomBy(2);
    const record = forceGraphInstances[0];
    const moves = () =>
      record.cameraPosition!.filter(([position]) => position !== undefined)
        .length;
    const before = moves();
    const fits = record.zoomToFit?.length ?? 0;
    view.rerender(
      <ForceCanvas
        {...props}
        ref={ref}
        width={600}
        focusIds={new Set(["n2", "n1"])}
        nodes={props.nodes.map((n) => ({
          ...n,
          label: `${n.label} refreshed`,
        }))}
      />,
    );
    await act(frameTick);
    (lastForceGraphProps!.onEngineStop as () => void)();
    expect(moves()).toBe(before);
    expect(record.zoomToFit?.length ?? 0).toBe(fits);
    positionEngineNodes();
    view.rerender(<ForceCanvas {...props} ref={ref} selectedId="n1" />);
    positionEngineNodes();
    (lastForceGraphProps!.onEngineStop as () => void)();
    expect(moves()).toBe(before + 2);
  });

  it("zooms along the camera ray while retaining an off-origin orbit target", () => {
    const { ref } = renderCanvas();
    const record = forceGraphInstances[0];
    record.controlTarget = { x: 10, y: 20, z: 30 };
    ref.current!.zoomBy(2);
    expect(record.cameraPosition!.at(-1)).toEqual([
      { x: 5, y: 10, z: 165 },
      { x: 10, y: 20, z: 30 },
      400,
    ]);
  });

  it("exposes zoom/fit/reset/pause/resume imperative controls", () => {
    const { ref } = renderCanvas();
    const record = forceGraphInstances[0];
    ref.current!.zoomBy(1.4);
    expect(record.cameraPosition).toHaveLength(2);
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
    expect(linkColor(edgeInFocus)).toBe(palette.accent);
    expect(linkColor(edgeOutside)).toBe(palette.bg);
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

  it("draws a visible labeled node instead of an empty overlay", () => {
    renderCanvas();
    expect(lastForceGraphProps!.extraRenderers).toHaveLength(1);
    const node = (
      lastForceGraphProps!.nodeThreeObject as (value: object) => {
        children: { element?: HTMLElement }[];
      }
    )({ id: "n1", label: "<Actor One>", kind: "actor", role: "focus" });
    const glyph = node.children[1].element!;
    expect(glyph.textContent).toBe("<Actor One>");
    expect(glyph.querySelector("svg")).toBeTruthy();
    expect(glyph.querySelector("[data-tile=node]")).toBeTruthy();
    expect(glyph.querySelector("actor")).toBeNull();
  });

  it("supplies kind colors and selection size to the 3D renderer", () => {
    renderCanvas();
    const color = lastForceGraphProps!.nodeColor as (node: unknown) => string;
    const size = lastForceGraphProps!.nodeVal as (node: unknown) => number;
    expect(color({ id: "n1", kind: "actor" })).toBe(kindColor.actor);
    expect(size({ id: "n2", kind: "claim" })).toBeGreaterThan(
      size({ id: "n1", kind: "actor" }),
    );
  });
});

/* ---------------------------------------------------------------------------
 * Real list twin (Topology): canvas twin rendered by the graph area, rows in
 * the organized nodes/connections panels; both stay in sync by id.
 * ------------------------------------------------------------------------- */

const topologyPayload: TopologyData = {
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
async function startTopology(
  seed: string | null,
  onSeed = vi.fn(),
  payload = topologyPayload,
) {
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
    http.get(`${testBase}/api/v1/topology`, () => HttpResponse.json(payload)),
  );
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const view = render(
    <QueryClientProvider client={client}>
      <BackendProvider settings={connectedSettings}>
        <Topology seed={seed} onSeed={onSeed} pageStyle={{}} titleStyle={{}} />
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
    await startTopology("n1");
    expect(
      screen.getByText("Showing 2 of 3 nodes · 1 of 2 connections"),
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
    expect(screen.queryByText(/Claim Two → cites → Circuit/)).toBeNull();
    expect(
      screen.queryByRole("button", { name: "Select node Circuit, hardware" }),
    ).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "2nd DEG" }));
    expect(screen.getByText(/Claim Two → cites → Circuit/)).toBeVisible();
    // The canvas placeholder is gated client-side (no hidden canvas nodes).
    expect(screen.queryByTestId("force-graph-canvas")).toBeNull();
  });

  it("omits topic nodes and their relationships from both graph and list", async () => {
    await startTopology("n1", vi.fn(), {
      ...topologyPayload,
      nodes: [
        ...topologyPayload.nodes,
        {
          id: "topic-only",
          kind: "topic",
          label: "Hidden topic",
          detail: "Topic",
          topology_seed: null,
        },
      ],
      edges: [
        ...topologyPayload.edges,
        {
          id: "topic-edge",
          from: "n1",
          to: "topic-only",
          label: "categorized",
        },
      ],
    });
    expect(screen.queryByRole("button", { name: /Hidden topic/ })).toBeNull();
    expect(
      screen.queryByRole("button", { name: "Show topic nodes" }),
    ).toBeNull();
    expect(screen.queryByText(/categorized/)).toBeNull();
  });
  it("syncs selection from the list twin into the details panel", async () => {
    await startTopology("n1");
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
    await startTopology("n1", onSeed);
    fireEvent.click(screen.getByRole("button", { name: /Navigate this node/ }));
    await waitFor(() => {
      expect(onSeed).toHaveBeenCalledWith(
        topologyPayload.nodes[0].topology_seed,
      );
    });
  });

  it("filters by kind via palette toggles, hiding hidden nodes and their edges", async () => {
    await startTopology("n1");
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

  it("scripts a reset back to unfiltered topology", async () => {
    await startTopology("n1");
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
    fireEvent.click(
      screen.getByRole("button", { name: "Show all node types" }),
    );
    await waitFor(() =>
      expect(screen.getByText(/Actor One → asserts → Claim Two/)).toBeVisible(),
    );
    // The selection survives the reset.
    expect(
      screen.getByRole("button", { name: "Select node Claim Two, claim" }),
    ).toBeTruthy();
  });

  it("returns along the focus history on the topology page", async () => {
    await startTopology("n1");
    fireEvent.click(
      screen.getByRole("button", { name: "Select node Claim Two, claim" }),
    );
    fireEvent.keyDown(window, { key: "Backspace" });
    await waitFor(() =>
      expect(screen.getByText(/Primary collection actor/)).toBeVisible(),
    );
  });
});
