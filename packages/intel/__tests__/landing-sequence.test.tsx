import { afterEach, describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { connectionChecks } from "../src/landing/checks";
import { useConnectionSequence } from "../src/landing/sequence";
const healthy = {
  service: "argos-intel",
  api_version: 1,
  storage: true,
  ready: true,
  instance_id: "fixture",
};
afterEach(() => vi.useRealTimers());
describe("landing presentation sequence", () => {
  it("starts offline, spends three seconds on each evidenced check, then blue and green", () => {
    vi.useFakeTimers();
    const { result, rerender } = renderHook(
      ({ health, ready }) =>
        useConnectionSequence(connectionChecks(health), ready, true),
      {
        initialProps: {
          health: undefined as typeof healthy | undefined,
          ready: false,
        },
      },
    );
    expect(result.current.phase).toBe("offline");
    expect(result.current.revealed).toHaveLength(0);
    rerender({ health: healthy, ready: true });
    expect(result.current.phase).toBe("checking");
    expect(result.current.active?.id).toBe("api");
    expect(result.current.revealed).toHaveLength(1);
    act(() => vi.advanceTimersByTime(2999));
    expect(result.current.active?.id).toBe("api");
    act(() => vi.advanceTimersByTime(1));
    expect(result.current.active?.id).toBe("service");
    expect(result.current.revealed).toHaveLength(2);
    for (let i = 0; i < 4; i++) act(() => vi.advanceTimersByTime(3000));
    expect(result.current.phase).toBe("healthy");
    expect(result.current.finished).toBe(false);
    act(() => vi.advanceTimersByTime(3000));
    expect(result.current.phase).toBe("ready");
    expect(result.current.finished).toBe(false);
    act(() => vi.advanceTimersByTime(3000));
    expect(result.current.finished).toBe(true);
  });
  it("does not label a partial connection healthy and resumes new checks in order", () => {
    vi.useFakeTimers();
    const { result, rerender } = renderHook(
      ({ health, ready }) =>
        useConnectionSequence(connectionChecks(health), ready, true),
      {
        initialProps: {
          health: { ...healthy, storage: false, ready: false },
          ready: false,
        },
      },
    );
    for (let i = 0; i < 3; i++) act(() => vi.advanceTimersByTime(3000));
    expect(result.current.phase).toBe("checking");
    expect(result.current.finished).toBe(false);
    rerender({ health: healthy, ready: true });
    expect(result.current.active?.id).toBe("storage");
  });
  it("skips timed fades for a completed static composition", () => {
    const { result } = renderHook(() =>
      useConnectionSequence(connectionChecks(healthy), true, false),
    );
    expect(result.current.phase).toBe("ready");
    expect(result.current.finished).toBe(true);
    expect(result.current.revealed).toHaveLength(5);
  });
});
