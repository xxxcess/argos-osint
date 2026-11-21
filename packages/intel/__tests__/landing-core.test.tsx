import { afterEach, expect, it, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";
import { OrbitScene } from "../src/landing/OrbitScene";
import { landingColors } from "../src/shell/intel-theme";
import { paint } from "../../../vitest.test-support";
vi.mock("../src/landing/webgl.web", () => ({ webGL: () => true }));
afterEach(() => vi.useRealTimers());
it("keeps the warm core and crescent through the readiness presentation", () => {
  vi.useFakeTimers();
  const health = {
    service: "argos-intel",
    api_version: 1,
    storage: true,
    ready: true,
    instance_id: "fixture",
  };
  const { rerender } = render(<OrbitScene />);
  expect(screen.getByTestId("landing-mark")).toBeInTheDocument();
  expect(paint(screen.getByTestId("landing-core"), "backgroundColor")).toBe(
    landingColors.core.toLowerCase(),
  );
  rerender(<OrbitScene health={health} ready />);
  expect(paint(screen.getByTestId("landing-core"), "backgroundColor")).toBe(
    landingColors.core.toLowerCase(),
  );
  for (let i = 0; i < 5; i++) act(() => vi.advanceTimersByTime(3000));
  expect(screen.getByRole("status")).toHaveTextContent(
    "All connections healthy",
  );
  expect(paint(screen.getByTestId("landing-core"), "backgroundColor")).toBe(
    landingColors.core.toLowerCase(),
  );
  act(() => vi.advanceTimersByTime(3000));
  expect(screen.getByRole("status")).toHaveTextContent("Workspace ready");
  expect(screen.getByTestId("landing-fallback").children).toHaveLength(2);
  expect(paint(screen.getByTestId("landing-core"), "backgroundColor")).toBe(
    landingColors.core.toLowerCase(),
  );
});
