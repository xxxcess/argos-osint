import React from "react";
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import {
  colors,
  fonts,
  glass,
  glow,
  toneColor,
  Panel,
  Copy,
} from "@argos/ui";
import { paint, sheetProp, sheetStyle } from "../../../vitest.test-support";

/**
 * Token contract tests. Tokens are imported from `@argos/ui` (single source of
 * truth, mirroring the TUI palette in crates/argos-osint-bin/src/tui/theme.rs);
 * these assertions pin the terminal-desk values so token drift fails fast, and
 * prove real components consume them rather than redeclaring values.
 */
describe("Theme tokens (imported from @argos/ui source)", () => {
  it("defines the terminal-desk neutrals", () => {
    expect(colors.bg).toBe("#141414");
    expect(colors.panel).toBe("#1e1e1e");
    expect(colors.raised).toBe("#303030");
    expect(colors.border).toBe("#535353");
    expect(colors.text).toBe("#e8e8e8");
    expect(colors.dim).toBe("#b5b5b5");
    expect(colors.muted).toBe("#919191");
  });

  it("defines accent/accent2 plus the four status hues", () => {
    expect(colors.accent).toBe("#87bfff");
    expect(colors.accent2).toBe("#c678dd");
    expect(colors.success).toBe("#98c379");
    expect(colors.warning).toBe("#e5c07b");
    expect(colors.danger).toBe("#f08a8a");
  });

  it("defines glass translucency values for the HUD treatment", () => {
    expect(colors.glass).toBe("rgba(30, 30, 30, 0.55)");
    expect(colors.glassStrong).toBe("rgba(48, 48, 48, 0.72)");
    expect(colors.glassBorder).toBe("rgba(83, 83, 83, 0.6)");
    expect(colors.accentSoft).toBe("rgba(135, 191, 255, 0.14)");
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
    expect(paint(screen.getByText("Bone"), "color")).toBe(colors.accent);
    expect(sheetProp(screen.getByText("Bone"), "fontFamily")).toContain(
      "ui-monospace",
    );
  });

  it("keeps distinct tokens actually distinct (accent/danger/accentSoft/bg)", () => {
    const set = [colors.accent, colors.danger, colors.accentSoft, colors.bg];
    expect(new Set(set).size).toBe(set.length);
  });
});
