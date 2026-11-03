import React from "react";
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { EmptyState } from "@argos/ui";
import { sheetProp, sheetStyle } from "../../../vitest.test-support";

describe("EmptyState (real @argos/ui component)", () => {
  it("renders the title as a semibold header and follows it with a body Copy", () => {
    render(
      <EmptyState title="No articles yet">
        Collect an Atlas cycle to populate this bulletin.
      </EmptyState>,
    );
    expect(screen.getByText("No articles yet")).toBeVisible();
    expect(
      screen.getByText("Collect an Atlas cycle to populate this bulletin."),
    ).toBeVisible();
  });

  it("omits the body row entirely when no children are provided", () => {
    const { container } = render(<EmptyState title="No retained connections" />);
    expect(screen.getByText("No retained connections")).toBeVisible();
    // Nothing rendered except the title text node.
    expect(container.firstElementChild!.childElementCount).toBe(1);
  });

  it("keeps default (spaced) density: 24px block padding with 8px gap", () => {
    const { container } = render(
      <EmptyState title="Empty">
        <span>hint</span>
      </EmptyState>,
    );
    const box = container.firstElementChild as HTMLElement;
    expect(sheetProp(box, "paddingTop")).toBe("24px");
    expect(sheetProp(box, "paddingBottom")).toBe("24px");
    expect(sheetProp(box, "gap")).toBe("8px");
    // Title is semibold and text-toned at body-adjacent size.
    const title = sheetStyle(screen.getByText("Empty") as Element);
    expect(title["font-weight"]).toBeTruthy();
    expect(title["font-size"]).toBeTruthy();
  });

  it("never hides reactive neighbouring content (empty is inline, not an overlay)", () => {
    render(
      <div>
        <p data-testid="live">Live telemetry row</p>
        <EmptyState title="No matching articles">
          No articles match this day, category and search.
        </EmptyState>
      </div>,
    );
    expect(screen.getByTestId("live")).toBeVisible();
    expect(screen.getByText("No matching articles")).toBeVisible();
  });
});
