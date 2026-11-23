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
    expect(paint(frame, "backgroundColor")).toBe(colors.glass.toLowerCase());
    expect(sheetProp(frame, "borderTopWidth")).toBe("2px"); // accent top rule
    expect(paint(frame, "borderTopColor")).toBe(toneColor.accent.toLowerCase());
    expect(paint(frame, "borderLeftColor")).toBe(colors.border.toLowerCase());
    expect(sheetProp(frame, "padding")).toBe("16px");
    expect(sheetProp(frame, "gap")).toBe("12px");
  });

  it("exposes the frame material to the document stylesheet", () => {
    const { container } = render(
      <Frame>
        <Copy>x</Copy>
      </Frame>,
    );
    expect(container.firstElementChild).toHaveAttribute("data-glass", "frame");
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
