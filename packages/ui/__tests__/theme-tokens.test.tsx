import React from "react";
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { colors, fonts, glass, glow, toneColor, Panel, Copy } from "@argos/ui";
import { paint, sheetProp, sheetStyle } from "../../../vitest.test-support";

/**
 * Token contract tests. Tokens are imported from `@argos/ui` (single source of
 * truth, with logo anchors shared with native and a separate Rust TUI palette);
 * these assertions pin the Fleetworks web paint so token drift fails fast, and
 * prove real components consume them rather than redeclaring values.
 */
describe("Theme tokens (imported from @argos/ui source)", () => {
  it("defines the matte canvas and cool charcoal surfaces", () => {
    expect(colors.bg).toBe("#060709");
    expect(colors.panel).toBe("#0B0F14");
    expect(colors.raised).toBe("#121820");
    expect(colors.border).toBe("#647587");
    expect(colors.text).toBe("#D5DCE5");
    expect(colors.dim).toBe("#A8B6C5");
    expect(colors.muted).toBe("#8192A4");
  });

  it("defines accent/accent2 plus the four status hues", () => {
    expect(colors.accent).toBe("#24DCE3");
    expect(colors.accent2).toBe("#c678dd");
    expect(colors.success).toBe("#23DFA0");
    expect(colors.warning).toBe("#F1C84B");
    expect(colors.danger).toBe("#F280A0");
  });

  it("retains compatibility names as opaque matte fills and faint decorative edges", () => {
    expect(colors.glass).toBe("#0B0F14");
    expect(colors.glassStrong).toBe("#121820");
    expect(colors.glassBorder).toBe("#647587");
    expect(colors.cardEdge).toBe("rgba(213, 220, 229, 0.02)");
    expect(colors.satinControlEdge).toBe("rgba(213, 220, 229, 0.03)");
    expect(colors.accentSoft).toBe("rgba(36, 220, 227, 0.1)");
  });

  it("keeps mono for telemetry and body for prose", () => {
    expect(fonts.mono).toContain("ui-monospace");
    expect(fonts.mono).toContain("JetBrains Mono");
    expect(fonts.body).toContain("system-ui");
    expect(fonts.body).toContain("sans-serif");
  });

  it("maps each tone to its token without alterations", () => {
    expect(toneColor["accent"]).toBe(colors.accent);
    expect(toneColor["success"]).toBe(colors.success);
    expect(toneColor["warning"]).toBe(colors.warning);
    expect(toneColor["danger"]).toBe(colors.danger);
    expect(toneColor["purple"]).toBe(colors.accent2);
  });

  it("the legacy glow helper retains a static state rim without an outer bloom", () => {
    const g = glow(colors.accent);
    expect(g.boxShadow).toBe(`inset 0 0 0 1px ${colors.accent}`);
  });

  it("the legacy glass recipe is an opaque matte card without backdrop blur", () => {
    expect(glass.backgroundColor).toBe(colors.panel);
    expect(glass.borderWidth).toBe(1);
    expect(glass.borderColor).toBe(colors.cardEdge);
    expect(glass).not.toHaveProperty("backdropFilter");
    expect(glass).not.toHaveProperty("WebkitBackdropFilter");
  });

  it("real components consume the tokens (Panel spreads the shared glass)", () => {
    const { container } = render(
      <Panel>
        <Copy>token consumer</Copy>
      </Panel>,
    );
    const panel = container.firstElementChild as HTMLElement;
    // The Panel's resolved declarations equal the token declarations one to one.
    const appliedStyle = sheetStyle(panel);
    expect(paint(panel, "backgroundColor")).toBe(colors.glass.toLowerCase());
    expect(appliedStyle["backdrop-filter"]).toBeUndefined();
    expect(paint(panel, "borderTopColor")).toBe(colors.cardEdge);
    expect(appliedStyle["border-top-width"]).toBe("1px");
    expect(screen.getByText("token consumer")).toBeVisible();
  });

  it("panel title renders with the accent token", () => {
    render(
      <Panel title="Bone">
        <Copy>body</Copy>
      </Panel>,
    );
    expect(paint(screen.getByText("Bone"), "color")).toBe(
      colors.accent.toLowerCase(),
    );
    expect(sheetProp(screen.getByText("Bone"), "fontFamily")).toContain(
      "ui-monospace",
    );
  });

  it("keeps distinct tokens actually distinct (accent/danger/accentSoft/bg)", () => {
    const set = [colors.accent, colors.danger, colors.accentSoft, colors.bg];
    expect(new Set(set).size).toBe(set.length);
  });
});
