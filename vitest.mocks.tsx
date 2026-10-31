import React, { forwardRef, useImperativeHandle, useRef } from "react";

/**
 * Shared test doubles for modules that jsdom cannot execute (canvas/d3).
 *
 * `ForceGraph3DMock` replaces `react-force-graph-3d` so the real `ForceCanvas`
 * wrapper in `packages/intel/src/topology/ForceCanvas.tsx` can be exercised:
 * it records every prop it receives and every imperative method call, and
 * renders a plain element as the canvas host.
 */

export type ForceGraph3DHandler = {
  controls: () => { target: { x: number; y: number; z: number } };
  cameraPosition: (
    position?: { x: number; y: number; z: number },
    lookAt?: { x: number; y: number; z: number },
    ms?: number,
  ) => { x: number; y: number; z: number };
  zoomToFit: (ms?: number, padding?: number) => void;

  pauseAnimation: () => void;
  resumeAnimation: () => void;
  d3ReheatSimulation: () => void;
};
type MutableRecord = {
  controlTarget?: { x: number; y: number; z: number };
  zoomBy?: number[];
  zoomToFit?: [number | undefined, number | undefined][];
  cameraPosition?: unknown[][];
  pauseAnimation?: number;
  resumeAnimation?: number;
  d3ReheatSimulation?: number;
};

// Latestcaptured props + per-instance method call records.
export const forceGraphInstances: MutableRecord[] = [];
export let lastForceGraphProps: Record<string, unknown> | null = null;
export const resetForceGraphRecording = () => {
  forceGraphInstances.length = 0;
  lastForceGraphProps = null;
};

export type ForceGraph3DMockProps = {
  graphData?: {
    nodes: unknown[];
    links: unknown[];
  };
  [extra: string]: unknown;
};

export const ForceGraph3DMock = forwardRef<
  ForceGraph3DHandler,
  ForceGraph3DMockProps
>(function ForceGraph3DMock(props, ref) {
  lastForceGraphProps = props;
  const record: MutableRecord = useRef<MutableRecord>({}).current;
  if (!forceGraphInstances.includes(record)) forceGraphInstances.push(record);
  useImperativeHandle(ref, () => ({
    controls: () => ({ target: record.controlTarget ?? { x: 0, y: 0, z: 0 } }),
    cameraPosition: (position, lookAt, ms) => {
      (record.cameraPosition ??= []).push([position, lookAt, ms]);
      return position ?? { x: 0, y: 0, z: 300 };
    },
    zoomToFit: (ms?: number, padding?: number) => {
      (record.zoomToFit ??= []).push([ms, padding]);
    },
    pauseAnimation: () => {
      record.pauseAnimation = (record.pauseAnimation ?? 0) + 1;
    },
    resumeAnimation: () => {
      record.resumeAnimation = (record.resumeAnimation ?? 0) + 1;
    },
    d3ReheatSimulation: () => {
      record.d3ReheatSimulation = (record.d3ReheatSimulation ?? 0) + 1;
    },
  }));
  return <div data-testid="force-graph-canvas" />;
});
