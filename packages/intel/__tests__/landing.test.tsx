import { afterEach, describe, expect, it, vi } from "vitest";
import { act, render } from "@testing-library/react";
import { Group, Mesh, Sprite } from "three";
import { connectionChecks } from "../src/landing/checks";
import OrbitCanvas from "../src/landing/OrbitCanvas";
const recording = vi.hoisted(() => ({
  props: {} as Record<string, unknown>,
  camera: vi.fn(),
  force: vi.fn(),
}));
vi.mock("react-force-graph-3d", async () => {
  const React = await import("react");
  return {
    default: React.forwardRef(function Graph(
      props: Record<string, unknown>,
      ref,
    ) {
      React.useLayoutEffect(() => {
        recording.props = props;
      });
      React.useImperativeHandle(ref, () => ({
        d3Force: recording.force,
        cameraPosition: recording.camera,
      }));
      return <div />;
    }),
  };
});
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
describe("landing canvas motion preference", () => {
  function graphData() {
    return recording.props.graphData as {
      nodes: { id: string }[];
      links: unknown[];
    };
  }

  it("uses a single mark with no connections and stays static under reduced motion", () => {
    const frame = vi.fn();
    vi.stubGlobal("requestAnimationFrame", frame);
    render(<OrbitCanvas reduced />);
    expect(graphData().nodes.map(({ id }) => id)).toEqual(["core"]);
    expect(graphData().links).toEqual([]);
    expect(recording.props.cooldownTicks).toBe(0);
    expect(frame).not.toHaveBeenCalled();
  });

  it("pulses the warm core halo with the existing period and settles with reduced motion", () => {
    const gradient = { addColorStop: vi.fn() };
    const canvas = {
      createRadialGradient: vi.fn(() => gradient),
      createLinearGradient: vi.fn(() => gradient),
      translate: vi.fn(),
      fillRect: vi.fn(),
      beginPath: vi.fn(),
      arc: vi.fn(),
      fill: vi.fn(),
      stroke: vi.fn(),
    };
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(
      canvas as unknown as CanvasRenderingContext2D,
    );
    let tick: FrameRequestCallback = () => {};
    vi.stubGlobal(
      "requestAnimationFrame",
      vi.fn((callback) => {
        tick = callback;
        return 42;
      }),
    );
    const cancel = vi.fn();
    vi.stubGlobal("cancelAnimationFrame", cancel);

    const { rerender } = render(<OrbitCanvas reduced={false} />);
    expect(graphData().nodes.map(({ id }) => id)).toEqual(["core"]);
    expect(graphData().links).toEqual([]);

    const createMark = recording.props.nodeThreeObject as () => Group;
    const mark = createMark();
    expect(mark.children).toHaveLength(3);
    const [core, halo, crescent] = mark.children;
    expect(core).toBeInstanceOf(Mesh);
    expect(halo).toBeInstanceOf(Sprite);
    expect(crescent).toBeInstanceOf(Sprite);
    expect(canvas.arc).toHaveBeenCalledTimes(2);
    expect(recording.props.cooldownTicks).toBe(Infinity);

    act(() => tick(600));
    expect((halo as Sprite).material.opacity).toBeCloseTo(0.62);
    act(() => tick(3000));
    expect((halo as Sprite).material.opacity).toBeCloseTo(0.62);

    rerender(<OrbitCanvas reduced />);
    expect(cancel).toHaveBeenCalledWith(42);
    expect((halo as Sprite).material.opacity).toBeCloseTo(0.55);
  });
});

it("only admits checks evidenced by the readiness response", () => {
  expect(connectionChecks().filter((check) => check.passed)).toHaveLength(0);
  const health = {
    service: "argos-intel",
    api_version: 1,
    storage: false,
    ready: false,
    instance_id: "fixture",
  };
  expect(
    connectionChecks(health)
      .filter((check) => check.passed)
      .map((check) => check.id),
  ).toEqual(["api", "service", "protocol"]);
  expect(
    connectionChecks({ ...health, storage: true, ready: true }).filter(
      (check) => check.passed,
    ),
  ).toHaveLength(5);
});
