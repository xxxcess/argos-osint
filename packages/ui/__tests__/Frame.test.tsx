import React from "react";
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { Frame, Copy, Label, toneColor, colors } from "@argos/ui";
import { paint, sheetProp } from "../../../vitest.test-support";

describe("Frame (real @argos/ui component)", () => {
  it("children render inside the corner-accent HUD frame", () => {
    render(
      <Frame>
        <Label>framed telemetry</Label>
      </Frame>,
    );
    expect(screen.getByText("framed telemetry")).toBeVisible();
  });

  it("applies the glass token plus a 2px accent top rule for tone=accent", () => {
    const { container } = render(
      <Frame>
        <Copy>x</Copy>
      </Frame>,
    );
    const frame = container.firstElementChild as HTMLElement;
    expect(paint(frame, "backgroundColor")).toBe(colors.glass);
    expect(sheetProp(frame, "borderTopWidth")).toBe("2px"); // accent top rule
    expect(paint(frame, "borderTopColor")).toBe(toneColor.accent);
    expect(paint(frame, "borderLeftColor")).toBe(colors.glassBorder);
    expect(sheetProp(frame, "padding")).toBe("16px");
    expect(sheetProp(frame, "gap")).toBe("12px");
  });

  // FINDING: Frame's runtime `glass` spread loses `backdrop-filter` (RNW drops
  // unknown props from runtime style arrays); Panel keeps the blur.
  it("does NOT apply HUD backdrop blur (known RNW limitation)", () => {
    const { container } = render(
      <Frame>
        <Copy>x</Copy>
      </Frame>,
    );
    const frame = container.firstElementChild as HTMLElement;
    // Frame uses glass spread but backdrop-filter is dropped by react-native-web
    // from runtime style arrays; jsdom also doesn't support backdrop-filter.
    // The style prop returns empty string because it's not in atomic classes
    // and not in inline style.
    expect(sheetProp(frame, "backdropFilter")).toBe("");
  });

  it("tints the top rule per tone token", () => {
    const { container } = render(
      <Frame tone="success">
        <Copy>x</Copy>
      </Frame>,
    );
    const frame = container.firstElementChild as HTMLElement;
    expect(paint(frame, "borderTopColor")).toBe(toneColor.success);
    expect(sheetProp(frame, "borderTopWidth")).toBe("2px");
  });

  it("keeps body content readable for every tone", () => {
    const { container } = render(
      <Frame tone="danger">
        <Copy>danger frame content</Copy>
      </Frame>,
    );
    expect(paint(container.firstElementChild!, "borderTopColor")).toBe(
      toneColor.danger,
    );
    expect(screen.getByText("danger frame content")).toBeVisible();
  });
});
