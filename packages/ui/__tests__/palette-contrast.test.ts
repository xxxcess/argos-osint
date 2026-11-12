// @vitest-environment node
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { colors } from "../src";
import {
  intelColors,
  intelKindColors,
  intelKindFills,
  intelMetricColors,
  intelStatus,
  topologyColors,
} from "../../intel/src/shell/intel-theme";

type Rgb = [number, number, number];
function rgba(value: string): [number, number, number, number] {
  if (value.startsWith("#")) {
    const hex = value.slice(1);
    return [
      parseInt(hex.slice(0, 2), 16),
      parseInt(hex.slice(2, 4), 16),
      parseInt(hex.slice(4, 6), 16),
      hex.length === 8 ? parseInt(hex.slice(6, 8), 16) / 255 : 1,
    ];
  }
  const channels = value.match(/[\d.]+/g)?.map(Number);
  if (!channels || channels.length < 3)
    throw new Error(`Unsupported fixture color ${value}`);
  return [channels[0], channels[1], channels[2], channels[3] ?? 1];
}
function composite(foreground: string, background: string): Rgb {
  const front = rgba(foreground),
    back = rgba(background);
  return [0, 1, 2].map(
    (i) => front[i] * front[3] + back[i] * (1 - front[3]),
  ) as Rgb;
}
function css(rgb: Rgb): string {
  return `rgb(${rgb.join(",")})`;
}
function contrast(foreground: string, background: string): number {
  const luminance = (rgb: Rgb) =>
    rgb
      .map((channel) => channel / 255)
      .map((channel) =>
        channel <= 0.04045
          ? channel / 12.92
          : ((channel + 0.055) / 1.055) ** 2.4,
      )
      .reduce(
        (sum, channel, i) => sum + channel * [0.2126, 0.7152, 0.0722][i],
        0,
      );
  const a = luminance(composite(foreground, background)),
    b = luminance(rgba(background).slice(0, 3) as Rgb);
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

describe("Fleetworks palette compositing contracts (rendered verification remains separate)", () => {
  it("shares native logo anchors without making either build run the other toolchain", () => {
    const native = readFileSync(
      new URL("../../../apps/fleetworks-desktop/src/theme.rs", import.meta.url),
      "utf8",
    );
    const anchors = {
      BACKGROUND: colors.bg,
      SURFACE: colors.panel,
      SURFACE_RAISED: colors.raised,
      SURFACE_INSET: colors.inset,
      BORDER_SUBTLE: colors.borderSubtle,
      BORDER_CONTROL: colors.border,
      TEXT_PRIMARY: colors.text,
      TEXT_SECONDARY: colors.dim,
      TEXT_MUTED: colors.muted,
      ACCENT: colors.accent,
      ACCENT_PRESSED: colors.accentPressed,
      ACCENT_HOVER: colors.accentHover,
      ON_ACCENT: colors.onAccent,
      SELECTION_SURFACE: colors.selectionSurface,
    };
    for (const [name, value] of Object.entries(anchors))
      expect(native).toContain(`pub const ${name}: &str = "${value}";`);
    expect([
      intelColors.bg,
      intelColors.panel,
      intelColors.panelRaised,
      intelColors.text,
      intelColors.dim,
      intelColors.muted,
      intelColors.accent,
      intelColors.onAccent,
      intelColors.selectionSurface,
    ]).toEqual([
      colors.bg,
      colors.panel,
      colors.raised,
      colors.text,
      colors.dim,
      colors.muted,
      colors.accent,
      colors.onAccent,
      colors.selectionSurface,
    ]);
  });
  it("keeps normal text readable over actual neutral, selected and glass composites", () => {
    const surfaces = [
      colors.bg,
      colors.panel,
      colors.raised,
      colors.inset,
      colors.selectionSurface,
      ...[colors.bg, colors.panel, colors.raised].flatMap((base) => [
        css(composite(colors.glass, base)),
        css(composite(colors.glassStrong, base)),
        css(composite(colors.accentSoft, base)),
        css(composite(intelColors.accentSoft, base)),
      ]),
    ];
    for (const background of surfaces)
      for (const foreground of [
        colors.text,
        colors.dim,
        colors.muted,
        colors.accent,
        colors.accentPressed,
        intelColors.faint,
      ])
        expect(
          contrast(foreground, background),
          `${foreground} on ${background}`,
        ).toBeGreaterThanOrEqual(4.5);
    for (const accent of [
      colors.accent,
      colors.accentPressed,
      colors.accentHover,
    ])
      expect(contrast(colors.onAccent, accent)).toBeGreaterThanOrEqual(4.5);
    expect(contrast(colors.text, colors.accent)).toBeLessThan(4.5);
  });
  it("retains essential border/focus contrast after alpha blending", () => {
    for (const background of [
      colors.bg,
      colors.panel,
      colors.raised,
      colors.selectionSurface,
    ])
      for (const foreground of [
        colors.border,
        colors.glassBorder,
        intelColors.borderStrong,
        intelColors.accentBorder,
        intelColors.select,
      ])
        expect(
          contrast(foreground, background),
          `${foreground} on ${background}`,
        ).toBeGreaterThanOrEqual(3);
    expect(
      contrast(topologyColors.edge, topologyColors.bg),
    ).toBeGreaterThanOrEqual(3);
    expect(
      contrast(
        intelColors.text,
        css(composite(topologyColors.labelBg, topologyColors.bg)),
      ),
    ).toBeGreaterThanOrEqual(4.5);
  });
  it("keeps labeled status, evidence and metric colors independent of brand gold", () => {
    expect(intelStatus.succeeded.color).toBe(intelColors.success);
    expect(intelStatus.succeeded.color).not.toBe(intelColors.accent);
    expect(new Set(Object.values(intelMetricColors)).size).toBe(4);
    expect(Object.values(intelMetricColors)).not.toContain(intelColors.accent);
    for (const color of [
      colors.success,
      colors.warning,
      colors.danger,
      colors.accent2,
      ...Object.values(intelKindColors),
      ...Object.values(intelMetricColors),
      ...Object.values(intelStatus).map((status) => status.color),
    ])
      expect(
        contrast(color, intelColors.panelRaised),
        `semantic ${color}`,
      ).toBeGreaterThanOrEqual(4.5);
    for (const kind of ["actor", "claim", "inference", "link"] as const)
      expect(
        contrast(
          intelKindColors[kind],
          css(composite(intelKindFills[kind], intelColors.panelRaised)),
        ),
        `${kind} translucent fill`,
      ).toBeGreaterThanOrEqual(4.5);
  });
});
