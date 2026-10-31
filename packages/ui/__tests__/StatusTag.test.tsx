import React from "react";
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { StatusTag, toneColor } from "@argos/ui";
import { paint, sheetProp } from "../../../vitest.test-support";

describe("StatusTag (real @argos/ui component)", () => {
  it("formats children in bracketed uppercase mono telemetry type", () => {
    render(<StatusTag>DEFCON 2</StatusTag>);
    const tag = screen.getByText("[DEFCON 2]") as HTMLElement;
    expect(sheetProp(tag, "textTransform")).toBe("uppercase");
    expect(sheetProp(tag, "fontFamily")).toContain("ui-monospace");
  });

  it.each([
    ["accent", toneColor.accent],
    ["success", toneColor.success],
    ["warning", toneColor.warning],
    ["danger", toneColor.danger],
    ["purple", toneColor.purple],
  ] as const)("renders the %s tone from the toneColor token map", (tone, token) => {
    render(<StatusTag tone={tone}>{tone}</StatusTag>);
    const tag = screen.getByText(`[${tone}]`);
    expect(paint(tag, "color")).toBe(token);
    // The rim is the same tone at ~40% alpha (color + "66" rendered as rgba).
    const rgb = token.replace("#", "");
    const comp = [0, 2, 4]
      .map((i) => Number.parseInt(rgb.slice(i, i + 2), 16))
      .join(", ");
    expect(paint(tag, "borderTopColor")).toBe(`rgba(${comp}, 0.4)`);
  });

  it("defaults to the accent tone", () => {
    render(<StatusTag>ALERT</StatusTag>);
    expect(paint(screen.getByText("[ALERT]"), "color")).toBe(
      toneColor.accent,
    );
  });

  it("maps semantic tones distinctly (accent ≠ purple ≠ success)", () => {
    render(
      <div>
        <StatusTag tone="accent">accent</StatusTag>
        <StatusTag tone="purple">purple</StatusTag>
        <StatusTag tone="success">success</StatusTag>
      </div>,
    );
    const a = paint(screen.getByText("[accent]"), "color");
    const p = paint(screen.getByText("[purple]"), "color");
    const s = paint(screen.getByText("[success]"), "color");
    expect(new Set([a, p, s]).size).toBe(3);
  });

  it("keeps the compact 11/16 telemetry metrics with bold weight", () => {
    render(<StatusTag tone="warning">STALE</StatusTag>);
    const tag = sheetProp(screen.getByText("[STALE]"), "fontSize");
    expect(tag).toBe("11px");
    expect(sheetProp(screen.getByText("[STALE]"), "lineHeight")).toBe("16px");
    expect(sheetProp(screen.getByText("[STALE]"), "fontWeight")).toBe("700");
  });
});
