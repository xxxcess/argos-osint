import React from "react";
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { LoadingState, colors } from "@argos/ui";
import { sheetProp } from "../../../vitest.test-support";

describe("LoadingState (real @argos/ui component)", () => {
  it("exposes a labeled progressbar pending state", () => {
    render(<LoadingState label="Loading bulletin…" />);
    const progress = screen.getByRole("progressbar", {
      name: "Loading bulletin…",
    });
    expect(progress).toBeVisible();
    expect(screen.getByText("Loading bulletin…")).toBeVisible();
  });

  it("renders the ActivityIndicator spinner outside the a11y tree, in accent", () => {
    const { container } = render(<LoadingState label="Loading rows…" />);
    const spinnerHost = container.querySelectorAll('[aria-hidden="true"]');
    expect(spinnerHost.length).toBeGreaterThan(0);
    // The spinner svg itself is accent-colored via inline stroke.
    const svg = container.querySelector("svg");
    expect(svg).toBeTruthy();
    const strokes = svg?.querySelectorAll("circle");
    expect(strokes?.length).toBe(2);
    const stop = strokes?.[strokes!.length - 1];
    expect(stop?.getAttribute("style")).toContain("#87bfff");
  });

  it("uses default (spaced) density with 20px vertical padding", () => {
    const { container } = render(<LoadingState label="Loading rows…" />);
    const box = container.querySelector('[role="progressbar"]') as HTMLElement;
    expect(sheetProp(box, "gap")).toBe("10px"); // single spacing unit
    expect(sheetProp(box, "paddingTop")).toBe("20px");
    expect(sheetProp(box, "paddingBottom")).toBe("20px");
  });

  it("collapses vertical padding in compact mode (palettes/refresh rows)", () => {
    const { container } = render(<LoadingState label="Refreshing…" compact />);
    const box = container.querySelector('[role="progressbar"]') as HTMLElement;
    expect(sheetProp(box, "paddingTop")).toBe("0px");
    expect(sheetProp(box, "paddingBottom")).toBe("0px");
    // Horizontal row machinery (gap + spinner + label) survives density switch.
    expect(sheetProp(box, "gap")).toBe("10px");
    expect(screen.getByText("Refreshing…")).toBeVisible();
  });
});
