/**
 * Matte deep-space paint: one #060709 canvas, charcoal tonal steps, cool gray text.
 * Mirrors native theme.rs; palette-contrast.test.ts keeps the two in step.
 * Status colors (always paired with a label or icon): cyan live/running, blue info,
 * mint healthy, amber warning/stale, rose error, violet queued/paused, slate unknown.
 */
export const intelColors = {
  bg: "#060709",
  panel: "#0B0E13",
  panelRaised: "#111720",
  inset: "#060709",
  scrim: "rgba(6,7,9,0.9)",
  border: "rgba(232,237,244,0.02)",
  borderDecorative: "#262D38",
  satinControlEdge: "rgba(232,237,244,0.03)",
  borderStrong: "#6F7B8D",
  divider: "#262D38",
  text: "#E8EDF4",
  dim: "#BEC7D4",
  muted: "#9AA6B6",
  faint: "#9AA6B6",
  accent: "#A9B8CC",
  accentPressed: "#8FA1B8",
  accentHover: "#C3CFDF",
  onAccent: "#060709",
  selectionSurface: "#1B2533",
  accentSoft: "rgba(169,184,204,0.10)",
  accentBorder: "rgba(169,184,204,0.75)",
  select: "#C3CFDF",
  textSelection: "#1B2533",
  success: "#5FD39A",
  danger: "#F06A8A",
  dangerSoft: "rgba(240,106,138,0.12)",
  cool: "#5AB8F0",
  warning: "#E5B45B",
  fab: "#111720",
};
/** Channel identifiers, not severity, confidence or API-health indicators. */
export const intelMetricColors = {
  cpu: "#5FD39A",
  memory: intelColors.cool,
  disk: "#C08CF5",
  network: "#E06AA0",
};
export const intelKindColors = {
  actor: "#FFB347",
  claim: "#21D9EE",
  inference: "#B48EFF",
  link: "#FF6A96",
  location: "#598BFF",
  article: "#5FD39A",
  source: "#CAD9F0",
  topic: "#F5EB5D",
};
export const intelKindFills = {
  actor: "rgba(255,179,71,0.12)",
  claim: "rgba(33,217,238,0.12)",
  inference: "rgba(180,142,255,0.14)",
  link: "rgba(255,106,150,0.14)",
  focus: intelColors.selectionSurface,
};
export const intelEffects = {
  accentGlow: "none",
  cardShadow: "none",
};
export const intelSpace = {
  unit: 4,
  small: 8,
  medium: 16,
  large: 24,
  card: 20,
  gutter: 12,
};
export const intelRadius = { shell: 16, card: 14, control: 8, chip: 4 };
export const intelType = {
  display: { fontSize: 33, fontWeight: "800" as const, letterSpacing: -0.5 },
  title: { fontSize: 23, fontWeight: "700" as const },
  body: { fontSize: 16, lineHeight: 23 },
  row: { fontSize: 16 },
  section: { fontSize: 13, fontWeight: "700" as const, letterSpacing: 1.4 },
  micro: { fontSize: 13, fontWeight: "600" as const, letterSpacing: 1.6 },
  chip: { fontSize: 13, fontWeight: "700" as const, letterSpacing: 1.2 },
  launcher: { fontSize: 19, fontWeight: "700" as const, letterSpacing: 1.2 },
};

/** Graph-only treatments, matching the evidence topology reference. */
export const topologyColors = {
  bg: "#060709",
  panel: "#0B0E13",
  grid: "rgba(111,123,141,0.09)",
  axis: "rgba(111,123,141,0.18)",
  accent: "#A9B8CC",
  edge: "#8E9BB0",
  labelBg: "#060709",
  ghost: "#0B0E13",
};

export const intelMotion = {
  fast: 120,
  base: 200,
  page: 260,
  focus: 600,
  fit: 450,
  inspector: 220,
  easing: "cubic-bezier(.2,.7,.2,1)",
  reduced: 0,
};
export const intelStatus = Object.fromEntries(
  Object.entries({
    idle: intelColors.muted,
    queued: "#B48EFF",
    running: "#21D9EE",
    succeeded: intelColors.success,
    failed: intelColors.danger,
    stale: intelColors.warning,
  }).map(([state, color]) => [state, { color, glow: "none" }]),
) as Record<
  "idle" | "queued" | "running" | "succeeded" | "failed" | "stale",
  { color: string; glow: string }
>;
export const landingColors = {
  core: "#FFB347",
  coreGlow: "#FFB347",
  orbiter: intelColors.accent,
  orbiterAlt: intelColors.cool,
  link: topologyColors.edge,
};
