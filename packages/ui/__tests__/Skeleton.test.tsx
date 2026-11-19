import React from "react";
import { act, render, screen } from "@testing-library/react";
import { AccessibilityInfo, Animated } from "react-native";
import { afterEach, expect, it, vi } from "vitest";
import { Skeleton } from "@argos/ui";

afterEach(() => vi.restoreAllMocks());

it("honors reduced motion changes and stops decorative animation on unmount", async () => {
  vi.spyOn(AccessibilityInfo, "isReduceMotionEnabled").mockResolvedValue(true);
  let changed: (reduced: boolean) => void = () => {};
  const remove = vi.fn();
  vi.spyOn(AccessibilityInfo, "addEventListener").mockImplementation(((
    _event: string,
    listener: (value: boolean) => void,
  ) => {
    changed = listener;
    return { remove };
  }) as unknown as typeof AccessibilityInfo.addEventListener);
  const animation = { start: vi.fn(), stop: vi.fn(), reset: vi.fn() };
  const loop = vi.spyOn(Animated, "loop").mockReturnValue(animation);
  const { unmount } = render(<Skeleton lines={3} />);
  await act(async () => {});
  expect(screen.getAllByTestId("skeleton-line")).toHaveLength(3);
  expect(
    screen.getAllByTestId("skeleton-line")[0].closest('[aria-hidden="true"]'),
  ).not.toBeNull();
  expect(loop).not.toHaveBeenCalled();
  act(() => changed(false));
  expect(animation.start).toHaveBeenCalledTimes(1);
  act(() => changed(true));
  expect(animation.stop).toHaveBeenCalled();
  expect(animation.start).toHaveBeenCalledTimes(1);
  unmount();
  expect(remove).toHaveBeenCalledTimes(1);
});

it("never starts animation after a delayed accessibility response outlives the component", async () => {
  let resolve!: (reduced: boolean) => void;
  vi.spyOn(AccessibilityInfo, "isReduceMotionEnabled").mockReturnValue(
    new Promise<boolean>((done) => {
      resolve = done;
    }),
  );
  const loop = vi.spyOn(Animated, "loop");
  const { unmount } = render(<Skeleton />);
  unmount();
  await act(async () => resolve(false));
  expect(loop).not.toHaveBeenCalled();
});

it("paints a steady cool base with no opacity pulse", async () => {
  vi.spyOn(AccessibilityInfo, "isReduceMotionEnabled").mockResolvedValue(true);
  render(<Skeleton lines={2} />);
  await act(async () => {});
  for (const line of screen.getAllByTestId("skeleton-line")) {
    expect(getComputedStyle(line).backgroundColor).toBe("rgb(39, 49, 59)");
    expect(getComputedStyle(line).opacity).not.toBe("0.55");
  }
});

it("pauses the sheen while the page is hidden and resumes when it is visible", async () => {
  vi.spyOn(AccessibilityInfo, "isReduceMotionEnabled").mockResolvedValue(false);
  const animation = { start: vi.fn(), stop: vi.fn(), reset: vi.fn() };
  const loop = vi.spyOn(Animated, "loop").mockReturnValue(animation);
  const state = vi.spyOn(document, "visibilityState", "get");
  state.mockReturnValue("visible");
  const { unmount } = render(<Skeleton />);
  await act(async () => {});
  expect(animation.start).toHaveBeenCalledTimes(1);

  state.mockReturnValue("hidden");
  act(() => {
    document.dispatchEvent(new Event("visibilitychange"));
  });
  expect(animation.stop).toHaveBeenCalled();
  expect(loop).toHaveBeenCalledTimes(1);

  state.mockReturnValue("visible");
  act(() => {
    document.dispatchEvent(new Event("visibilitychange"));
  });
  expect(loop).toHaveBeenCalledTimes(2);

  unmount();
  const calls = loop.mock.calls.length;
  act(() => {
    document.dispatchEvent(new Event("visibilitychange"));
  });
  expect(loop).toHaveBeenCalledTimes(calls);
});

it("shares one loop across skeletons and stops it with the last one", async () => {
  vi.spyOn(AccessibilityInfo, "isReduceMotionEnabled").mockResolvedValue(false);
  const animation = { start: vi.fn(), stop: vi.fn(), reset: vi.fn() };
  const loop = vi.spyOn(Animated, "loop").mockReturnValue(animation);
  const first = render(<Skeleton />);
  const second = render(<Skeleton lines={2} />);
  await act(async () => {});
  expect(loop).toHaveBeenCalledTimes(1);
  expect(animation.start).toHaveBeenCalledTimes(1);
  first.unmount();
  expect(animation.stop).not.toHaveBeenCalled();
  second.unmount();
  expect(animation.stop).toHaveBeenCalled();
});
