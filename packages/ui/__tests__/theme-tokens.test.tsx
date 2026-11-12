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
  it("defines the logo charcoal/ivory and warm neutral surfaces", () => {
    expect(colors.bg).toBe("#111111");
    expect(colors.panel).toBe("#141414");
    expect(colors.raised).toBe("#232321");
    expect(colors.border).toBe("#898274");
    expect(colors.text).toBe("#F3EEE4");
    expect(colors.dim).toBe("#C6C0B5");
    expect(colors.muted).toBe("#A09A8F");
  });

  it("defines accent/accent2 plus the four status hues", () => {
    expect(colors.accent).toBe("#C6A56E");
    expect(colors.accent2).toBe("#c678dd");
    expect(colors.success).toBe("#98c379");
    expect(colors.warning).toBe("#e5c07b");
    expect(colors.danger).toBe("#f08a8a");
  });

  it("defines glass translucency values for the HUD treatment", () => {
    expect(colors.glass).toBe("rgba(20, 20, 20, 0.55)");
    expect(colors.glassStrong).toBe("rgba(35, 35, 33, 0.72)");
    expect(colors.glassBorder).toBe("rgba(137, 130, 116, 0.85)");
    expect(colors.accentSoft).toBe("rgba(198, 165, 110, 0.1)");
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

  it("glow composes an outer bloom plus an inset rim from the given color", () => {
    const g = glow(colors.accent);
    expect(g.boxShadow).toContain(`12px ${colors.accent}55`);
    expect(g.boxShadow).toContain(`inset 0 0 0 1px ${colors.accent}33`);
  });

  it("glass treatment combines translucency, rim and backdrop blur", () => {
    expect(glass.backgroundColor).toBe(colors.glass);
    expect(glass.borderWidth).toBe(1);
    expect(glass.borderColor).toBe(colors.glassBorder);
    expect(glass.backdropFilter).toBe("blur(12px)");
    expect(glass.WebkitBackdropFilter).toBe("blur(12px)");
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
    expect(paint(panel, "backgroundColor")).toBe(colors.glass);
    expect(appliedStyle["backdrop-filter"]).toBe(glass.backdropFilter);
    expect(paint(panel, "borderTopColor")).toBe(glass.borderColor);
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
