import React from "react";
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { Panel, Copy, colors } from "@argos/ui";
import { paint, sheetProp } from "../../../vitest.test-support";

describe("Panel (real @argos/ui component)", () => {
  it("renders the title as an accent, mono-centered telemetry header", () => {
    render(
      <Panel title="Neighborhood">
        <Copy>Panel body telemetry.</Copy>
      </Panel>,
    );
    const title = screen.getByText("Neighborhood");
    expect(sheetProp(title, "textTransform")).toBe("uppercase");
    expect(paint(title, "color")).toBe(colors.accent);
    expect(sheetProp(title, "fontFamily")).toContain("ui-monospace");

    // The children stay visible under the title.
    expect(screen.getByText("Panel body telemetry.")).toBeVisible();
  });

  it("renders no header when no title is given, children only", () => {
    const { container } = render(<Panel>Body only</Panel>);
    expect(container.firstElementChild!.textContent).toBe("Body only");
    expect(container.textContent).not.toMatch(/title/i);
  });

  it("applies the HUD glass treatment from the shared glass token", () => {
    const { container } = render(
      <Panel title="Glass">
        <Copy>body</Copy>
      </Panel>,
    );
    const panel = container.firstElementChild as HTMLElement;
    expect(paint(panel, "backgroundColor")).toBe(colors.glass);
    expect(sheetProp(panel, "backdropFilter")).toBe("blur(12px)"); // HUD glass
    expect(sheetProp(panel, "WebkitBackdropFilter")).toBe("blur(12px)");
    expect(paint(panel, "borderTopColor")).toBe(colors.glassBorder);
    expect(sheetProp(panel, "borderTopWidth")).toBe("1px");
    expect(sheetProp(panel, "padding")).toBe("20px"); // default density spacing
    expect(sheetProp(panel, "gap")).toBe("16px");
  });

  it("spreads glass before its own paddings, so inner spacing is preserved", () => {
    const { container } = render(
      <Panel>
        <Copy>content</Copy>
      </Panel>,
    );
    const panel = container.firstElementChild as HTMLElement;
    // Glass is a spread base: glass colors stay, panel adds padding/gap.
    expect(paint(panel, "backgroundColor")).toBe(colors.glass);
    expect(sheetProp(panel, "padding")).toBe("20px");
    expect(screen.getByText("content")).toBeVisible();
  });
});
