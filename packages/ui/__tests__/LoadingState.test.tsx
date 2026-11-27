import React from "react";
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { LoadingState } from "@argos/ui";
import { sheetProp } from "../../../vitest.test-support";

describe("LoadingState", () => {
  it("labels the busy region and hides the decorative skeleton from assistive technology", () => {
    render(<LoadingState label="Loading bulletin…" />);
    const progress = screen.getByRole("progressbar", {
      name: "Loading bulletin…",
      busy: true,
    });
    expect(progress).toBeVisible();
    expect(screen.queryByText("Loading bulletin…")).not.toBeInTheDocument();
    const line = screen.getByTestId("skeleton-line");
    expect(progress).toContainElement(line);
    expect(line.closest('[aria-hidden="true"]')).not.toBeNull();
  });

  it("uses the loading surface's contextual fill", () => {
    render(
      <LoadingState
        label="Loading rows…"
        tint={{ baseColor: "#5FD39A", sheenColor: "rgba(146,215,177,0.06)" }}
      />,
    );
    const line = screen.getByTestId("skeleton-line");
    expect(getComputedStyle(line).backgroundColor).toBe("rgb(95, 211, 154)");
  });

  it("matches the text scale in each density", () => {
    const { rerender } = render(<LoadingState label="Loading rows…" />);
    expect(sheetProp(screen.getByTestId("skeleton-line"), "height")).toBe(
      "16px",
    );
    rerender(<LoadingState label="Refreshing…" compact />);
    expect(sheetProp(screen.getByTestId("skeleton-line"), "height")).toBe(
      "13px",
    );
  });
});
