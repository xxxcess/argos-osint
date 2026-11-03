import React from "react";
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { LoadingState, EmptyState } from "@argos/ui";
import { sheetProp } from "../../../vitest.test-support";

/**
 * Density contract: the default surface is spaced; `compact` is reserved for
 * launchers, palettes, headlines and category bullets (inline refresh rows).
 */
describe("Density modes (real components)", () => {
  it("LoadingState defaults to spaced density (20px block padding)", () => {
    const { container } = render(
      <LoadingState label="Loading retained days…" />,
    );
    const box = container.querySelector('[role="progressbar"]') as HTMLElement;
    expect(sheetProp(box, "paddingTop")).toBe("20px");
    expect(sheetProp(box, "paddingBottom")).toBe("20px");
  });

  it("LoadingState `compact` collapses block padding for inline refresh rows", () => {
    const { container } = render(
      <LoadingState label="Refreshing bulletin…" compact />,
    );
    const box = container.querySelector('[role="progressbar"]') as HTMLElement;
    expect(sheetProp(box, "paddingTop")).toBe("0px");
    expect(sheetProp(box, "paddingBottom")).toBe("0px");
    // Row machinery (gap + spinner + label) is unchanged, only spacing differs.
    expect(sheetProp(box, "gap")).toBe("10px");
    expect(screen.getByText("Refreshing bulletin…")).toBeVisible();
  });

  it("LoadingState instances without compact are identical", () => {
    const { container } = render(
      <div>
        <LoadingState label="A" />
        <LoadingState label="B" />
      </div>,
    );
    const boxes = container.querySelectorAll(
      '[role="progressbar"]:not([aria-hidden="true"])',
    );
    expect(boxes.length).toBe(2);
    const a = sheetProp(boxes[0] as Element, "paddingTop");
    const b = sheetProp(boxes[1] as Element, "paddingTop");
    expect(a).toBe(b);
    expect(a).toBe("20px");
    expect(screen.getByText("A")).toBeVisible();
    expect(screen.getByText("B")).toBeVisible();
  });

  it("EmptyState keeps spaced density (24px block padding, 8px gap)", () => {
    const { container } = render(
      <EmptyState title="No matching articles">No articles match.</EmptyState>,
    );
    const box = container.firstElementChild as HTMLElement;
    expect(sheetProp(box, "paddingTop")).toBe("24px");
    expect(sheetProp(box, "paddingBottom")).toBe("24px");
    expect(sheetProp(box, "gap")).toBe("8px");
  });

  it("EmptyState without children renders no extra body row (no phantom gap)", () => {
    const { container } = render(<EmptyState title="No retained connections" />);
    expect(container.firstElementChild!.childElementCount).toBe(1);
  });
});
