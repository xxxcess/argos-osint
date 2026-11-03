"use client";
import { useEffect, useState } from "react";
import type { ConnectionCheck } from "./checks";
export const STATUS_CYCLE_MS = 3000;
export type CorePhase = "offline" | "checking" | "healthy" | "ready";
/** Presentation follows real passed checks; health itself is never delayed or invented. */
export function useConnectionSequence(
  checks: ConnectionCheck[],
  ready: boolean,
  animated: boolean,
) {
  const passed = checks.filter((check) => check.passed);
  const ids = passed.map((check) => check.id).join(":");
  const [sequence, setSequence] = useState({ ids, index: 0 });
  if (sequence.ids !== ids) {
    const retained = sequence.ids
      .split(":")
      .filter(Boolean)
      .every((id) => passed.some((check) => check.id === id));
    setSequence({
      ids,
      index: retained ? Math.min(sequence.index, passed.length) : 0,
    });
  }
  const index = animated ? sequence.index : passed.length + 2;
  const advancing =
    animated && (index < passed.length || (ready && index < passed.length + 2));
  useEffect(() => {
    if (!advancing) return;
    const timer = setTimeout(
      () =>
        setSequence((current) => ({ ...current, index: current.index + 1 })),
      STATUS_CYCLE_MS,
    );
    return () => clearTimeout(timer);
  }, [advancing, index, ids]);
  const active = index < passed.length ? passed[index] : undefined;
  const phase: CorePhase = !passed.length
    ? "offline"
    : active || !ready
      ? "checking"
      : index === passed.length
        ? "healthy"
        : "ready";
  return {
    phase,
    active,
    passed,
    revealed: animated
      ? passed.slice(0, Math.min(index + 1, passed.length))
      : passed,
    finished: ready && index >= passed.length + 2,
  };
}
