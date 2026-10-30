import React, { forwardRef, useImperativeHandle, useRef } from "react";

/**
 * Shared test doubles for modules that jsdom cannot execute (canvas/d3).
 *
 * `ForceGraph2DMock` replaces `react-force-graph-2d` so the real `ForceCanvas`
 * wrapper in `packages/intel/src/topology/ForceCanvas.tsx` can be exercised:
 * it records every prop it receives and every imperative method call, and
 * renders a plain element as the canvas host.
 */

export type ForceGraph2DHandler = {
  zoom: (k?: number, ms?: number) => number | void;
  zoomToFit: (ms?: number, padding?: number) => void;
  centerAt: (x?: number, y?: number, ms?: number) => void;
  pauseAnimation: () => void;
  resumeAnimation: () => void;
  d3ReheatSimulation: () => void;
};
type MutableRecord = {
  zoomBy?: number[];
  zoomToFit?: [number | undefined, number | undefined][];
  centerAt?: [number | undefined, number | undefined, number | undefined][];
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

export type ForceGraph2DMockProps = {
  graphData?: {
    nodes: unknown[];
    links: unknown[];
  };
  [extra: string]: unknown;
};

export const ForceGraph2DMock = forwardRef<
  ForceGraph2DHandler,
  ForceGraph2DMockProps
>(function ForceGraph2DMock(props, ref) {
  lastForceGraphProps = props;
  const record: MutableRecord = useRef<MutableRecord>({}).current;
  if (!forceGraphInstances.includes(record)) forceGraphInstances.push(record);
  useImperativeHandle(ref, () => ({
    zoom: (k?: number) => {
      (record.zoomBy ??= []).push(k ?? 1);
      return k ?? 1;
    },
    zoomToFit: (ms?: number, padding?: number) => {
      (record.zoomToFit ??= []).push([ms, padding]);
    },
    centerAt: (x?: number, y?: number, ms?: number) => {
      (record.centerAt ??= []).push([x, y, ms]);
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
