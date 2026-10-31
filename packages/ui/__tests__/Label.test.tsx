import React from "react";
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { Label } from "@argos/ui";
import { paint, sheetProp } from "../../../vitest.test-support";

describe("Label (real @argos/ui component)", () => {
  it("renders small mono uppercase telemetry type", () => {
    render(<Label>01 / Atlas collection</Label>);
    const label = screen.getByText("01 / Atlas collection");
    expect(sheetProp(label, "fontFamily")).toContain("ui-monospace");
    expect(sheetProp(label, "textTransform")).toBe("uppercase");
    expect(sheetProp(label, "fontSize")).toBe("11px");
    expect(sheetProp(label, "letterSpacing")).toBe("1.2px");
    expect(sheetProp(label, "lineHeight")).toBe("18px");
  });

  it("uses the muted neutral token, not accent or body colors", () => {
    render(<Label>API endpoint</Label>);
    expect(paint(screen.getByText("API endpoint"), "color")).toBe("#919191");
  });

  it("stays mono: no body-font leak in the applied family", () => {
    render(<Label>Signals / geopolitical</Label>);
    expect(sheetProp(screen.getByText("Signals / geopolitical"), "fontFamily")).not.toContain(
      "system-ui",
    );
  });
});
