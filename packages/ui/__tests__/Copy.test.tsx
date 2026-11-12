import React from "react";
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { Copy } from "@argos/ui";
import { paint, sheetProp } from "../../../vitest.test-support";

describe("Copy (real @argos/ui component)", () => {
  it("renders prose in the body type at 15/24", () => {
    render(<Copy>Retained articles from Atlas.</Copy>);
    const copy = screen.getByText("Retained articles from Atlas.");
    expect(sheetProp(copy, "fontFamily")).toContain("system-ui");
    expect(sheetProp(copy, "fontSize")).toBe("15px");
    expect(sheetProp(copy, "lineHeight")).toBe("24px");
  });

  it("uses the dim neutral token (readable but subordinate)", () => {
    render(<Copy>Connections preserve their recorded meaning.</Copy>);
    const copy = screen.getByText(
      "Connections preserve their recorded meaning.",
    );
    expect(paint(copy, "color")).toBe("#c6c0b5"); // dim
    // Copy is never accent or muted in this role.
    expect(paint(copy, "color")).not.toBe("#c6a56e");
    expect(paint(copy, "color")).not.toBe("#a09a8f");
  });

  it("keeps long prose visible without clipping", () => {
    render(
      <Copy>
        The local Argos API must be ready before articles and actions are
        available.
      </Copy>,
    );
    expect(screen.getByText(/must be ready before articles/)).toBeVisible();
  });
});
