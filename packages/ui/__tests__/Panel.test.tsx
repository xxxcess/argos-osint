import React from "react";
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import {
  Panel,
  Copy,
  colors,
  groupAlpha,
  groupEdge,
  groupSurface,
} from "@argos/ui";
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
    expect(paint(title, "color")).toBe(colors.accent.toLowerCase());
    expect(sheetProp(title, "fontFamily")).toContain("ui-monospace");

    // The children stay visible under the title.
    expect(screen.getByText("Panel body telemetry.")).toBeVisible();
  });

  it("renders no header when no title is given, children only", () => {
    const { container } = render(
      <Panel>
        <Copy>Body only</Copy>
      </Panel>,
    );
    expect(container.firstElementChild!.textContent).toBe("Body only");
    expect(container.textContent).not.toMatch(/title/i);
  });

  it("applies the opaque matte card treatment from the shared compatibility token", () => {
    const { container } = render(
      <Panel title="Glass">
        <Copy>body</Copy>
      </Panel>,
    );
    const panel = container.firstElementChild as HTMLElement;
    expect(paint(panel, "backgroundColor")).toBe(colors.glass.toLowerCase());
    expect(sheetProp(panel, "backdropFilter")).toBe("");
    expect(sheetProp(panel, "WebkitBackdropFilter")).toBe("");
    // No decorative stroke; the perimeter dissolves and content clears the 16 px ramp.
    expect(sheetProp(panel, "borderTopWidth")).toBe("0px");
    expect(sheetProp(panel, "padding")).toBe("24px");
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
    expect(paint(panel, "backgroundColor")).toBe(colors.glass.toLowerCase());
    expect(sheetProp(panel, "padding")).toBe("24px");
    expect(screen.getByText("content")).toBeVisible();
  });

  it("draws the diffuse edge as a canvas-colored inset shadow behind the content", () => {
    const style = groupSurface(groupEdge.standard) as Record<string, unknown>;
    expect(style.boxShadow).toBe("inset 0 0 7.5px 8px #060709");
    expect(style.borderWidth).toBe(0);
    expect(groupSurface(groupEdge.compact)).toMatchObject({
      borderRadius: 16,
      padding: 20,
    });
  });

  it("ramps coverage from 0 at the boundary to 1 after the ramp, as smoothstep", () => {
    for (const edge of Object.values(groupEdge)) {
      expect(groupAlpha(edge, -3)).toBe(0);
      expect(groupAlpha(edge, 0)).toBe(0);
      expect(groupAlpha(edge, edge.ramp / 2)).toBe(0.5);
      expect(groupAlpha(edge, edge.ramp)).toBe(1);
      expect(groupAlpha(edge, edge.ramp * 2)).toBe(1);
      // Content clears the fading perimeter.
      expect(edge.inset).toBeGreaterThanOrEqual(edge.ramp);
    }
  });

  it("fits the drawn Gaussian to the smoothstep within 0.03 alpha", () => {
    // Phi via the Abramowitz and Stegun 7.1.26 erf.
    const erf = (x: number) => {
      const t = 1 / (1 + 0.3275911 * Math.abs(x));
      const poly =
        t *
        (0.254829592 +
          t *
            (-0.284496736 +
              t * (1.421413741 + t * (-1.453152027 + t * 1.061405429))));
      return Math.sign(x) * (1 - poly * Math.exp(-x * x));
    };
    for (const edge of Object.values(groupEdge)) {
      const sigma = edge.blur / 2;
      let worst = 0;
      for (let d = 0; d <= edge.ramp * 2; d += 0.25) {
        const drawn = 0.5 * (1 + erf((d - edge.spread) / sigma / Math.SQRT2));
        worst = Math.max(worst, Math.abs(drawn - groupAlpha(edge, d)));
      }
      expect(worst).toBeLessThan(0.03);
    }
  });
});
