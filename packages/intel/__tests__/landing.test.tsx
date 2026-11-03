import { afterEach, describe, expect, it, vi } from "vitest";
import { act, render } from "@testing-library/react";
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
const checks = ["api", "service", "protocol", "storage", "workspace"];
describe("landing canvas motion preference", () => {
  it("renders one settled orbiter per passing check and never schedules animation under reduced motion", () => {
    const frame = vi.fn();
    vi.stubGlobal("requestAnimationFrame", frame);
    render(<OrbitCanvas reduced checks={checks} />);
    const data = recording.props.graphData as {
      nodes: { id: string; x: number; y: number; z: number }[];
    };
    expect(data.nodes).toHaveLength(6);
    for (const node of data.nodes.slice(1)) {
      const radius = Math.hypot(node.x, node.y, node.z);
      expect(radius).toBeGreaterThanOrEqual(80);
      expect(radius).toBeLessThanOrEqual(160);
    }
    expect(recording.props.cooldownTicks).toBe(0);
    expect(frame).not.toHaveBeenCalled();
  });
  it("cancels active orbit motion when the preference changes", () => {
    const cancel = vi.fn();
    vi.stubGlobal(
      "requestAnimationFrame",
      vi.fn(() => 42),
    );
    vi.stubGlobal("cancelAnimationFrame", cancel);
    const { rerender } = render(
      <OrbitCanvas reduced={false} checks={checks} />,
    );
    expect(recording.props.cooldownTicks).toBe(Infinity);
    rerender(<OrbitCanvas reduced checks={checks} />);
    expect(cancel).toHaveBeenCalledWith(42);
    const data = recording.props.graphData as {
      nodes: { x: number; y: number; z: number }[];
    };
    for (const node of data.nodes.slice(1))
      expect(Math.hypot(node.x, node.y, node.z)).toBeLessThanOrEqual(160);
  });
});

it("reports completion after the final staggered arrival, once", () => {
  let tick: FrameRequestCallback = () => {};
  vi.stubGlobal(
    "requestAnimationFrame",
    vi.fn((callback) => {
      tick = callback;
      return 42;
    }),
  );
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
  vi.spyOn(performance, "now").mockReturnValue(0);
  const complete = vi.fn();
  render(
    <OrbitCanvas reduced={false} checks={checks} ready onComplete={complete} />,
  );
  act(() => tick(1));
  act(() => tick(4999));
  expect(complete).not.toHaveBeenCalled();
  act(() => tick(5001));
  expect(complete).toHaveBeenCalledTimes(1);
  act(() => tick(12001));
  expect(complete).toHaveBeenCalledTimes(1);
});

it("starts with a core and adds exactly the successful connection checks", () => {
  const { rerender } = render(<OrbitCanvas reduced checks={[]} />);
  expect(
    (recording.props.graphData as { nodes: unknown[] }).nodes,
  ).toHaveLength(1);
  rerender(<OrbitCanvas reduced checks={["api", "service"]} />);
  expect(
    (recording.props.graphData as { nodes: { id: string }[] }).nodes.map(
      (node) => node.id,
    ),
  ).toEqual(["core", "api", "service"]);
  expect(recording.props.width).toBe(560);
  expect(recording.props.height).toBe(320);
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
