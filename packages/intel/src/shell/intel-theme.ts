/**
 * Matte deep-space paint: one #060709 canvas, charcoal tonal steps, cool gray text.
 * Mirrors native theme.rs; palette-contrast.test.ts keeps the two in step.
 * Status colors (always paired with a label or icon): cyan live/running, blue info,
 * mint healthy, amber warning/stale, rose error, violet queued/paused, slate unknown.
 */
export const intelColors = {
  bg: "#060709",
  panel: "#0B0F14",
  panelRaised: "#121820",
  inset: "#060709",
  scrim: "rgba(6,7,9,0.9)",
  border: "rgba(213,220,229,0.02)",
  borderDecorative: "#2B343F",
  satinControlEdge: "rgba(213,220,229,0.03)",
  borderStrong: "#647587",
  divider: "#2B343F",
  text: "#D5DCE5",
  dim: "#A8B6C5",
  muted: "#8192A4",
  faint: "#8192A4",
  accent: "#24DCE3",
  accentPressed: "#1BB8C2",
  accentHover: "#68E7EB",
  onAccent: "#060709",
  selectionSurface: "#10272D",
  accentSoft: "rgba(36,220,227,0.10)",
  accentBorder: "rgba(36,220,227,0.75)",
  select: "#68E7EB",
  textSelection: "#10272D",
  success: "#23DFA0",
  danger: "#F280A0",
  dangerSoft: "rgba(242,128,160,0.12)",
  cool: "#53BAF5",
  warning: "#F1C84B",
  fab: "#121820",
};
/** Channel identifiers, not severity, confidence or API-health indicators. */
export const intelMetricColors = {
  cpu: "#24DCE3",
  memory: "#BA77F2",
  disk: "#35AFFF",
  network: "#F28C65",
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
  panel: "#0B0F14",
  grid: "rgba(100,117,135,0.09)",
  axis: "rgba(100,117,135,0.18)",
  accent: "#24DCE3",
  edge: "#8E9BB0",
  labelBg: "#060709",
  ghost: "#0B0F14",
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
    running: "#24DCE3",
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
