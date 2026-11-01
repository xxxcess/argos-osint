/** Intel-only Nexus Desk tokens. The shared Argos/TUI palette is unchanged. */
export const intelColors = {
  bg: "#0B0B0C",
  panel: "#141416",
  panelRaised: "#1B1B1E",
  inset: "#0F0F11",
  scrim: "rgba(8,8,9,0.78)",
  border: "#26262A",
  borderStrong: "#34343A",
  divider: "#1E1E21",
  text: "#ECECEE",
  dim: "#B4B4BA",
  muted: "#85858E",
  faint: "#84848D",
  accent: "#5FD39A",
  accentSoft: "rgba(95,211,154,0.10)",
  accentBorder: "rgba(95,211,154,0.35)",
  select: "#7FE0B4",
  textSelection: "#34343A",
  danger: "#F06A8A",
  dangerSoft: "rgba(240,106,138,0.12)",
  cool: "#5AB8F0",
  warning: "#E5B45B",
  fab: "#2A2F3A",
};
/** Channel identifiers, not severity, confidence or API-health indicators. */
export const intelMetricColors = {
  cpu: intelColors.accent,
  memory: intelColors.cool,
  disk: "#C08CF5",
  network: "#E06AA0",
};
export const intelKindColors = {
  actor: "#FFB347",
  claim: "#21D9EE",
  inference: "#AD83FF",
  link: "#FF5C8A",
  location: "#598BFF",
  article: intelColors.accent,
  source: "#CAD9F0",
  topic: "#F5EB5D",
};
export const intelKindFills = {
  actor: "rgba(255,179,71,0.12)",
  claim: "rgba(33,217,238,0.12)",
  inference: "rgba(173,131,255,0.14)",
  link: "rgba(255,92,138,0.14)",
  focus: "#132433",
};
export const intelEffects = {
  accentGlow: "0 0 18px rgba(95,211,154,0.35)",
  cardShadow: "0 8px 24px rgba(0,0,0,0.35)",
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
  display: { fontSize: 32, fontWeight: "800" as const, letterSpacing: -0.5 },
  title: { fontSize: 22, fontWeight: "700" as const },
  body: { fontSize: 15, lineHeight: 22 },
  row: { fontSize: 15 },
  section: { fontSize: 12, fontWeight: "700" as const, letterSpacing: 1.4 },
  micro: { fontSize: 12, fontWeight: "600" as const, letterSpacing: 1.6 },
  chip: { fontSize: 12, fontWeight: "700" as const, letterSpacing: 1.2 },
  launcher: { fontSize: 18, fontWeight: "700" as const, letterSpacing: 1.2 },
};

/** Graph-only treatments, matching the evidence topology reference. */
export const topologyColors = {
  bg: "#050506",
  panel: "rgba(18,18,20,0.94)",
  grid: "rgba(63,83,119,0.09)",
  axis: "rgba(86,107,140,0.18)",
  accent: "#00DFB1",
  edge: "#167C72",
  labelBg: "rgba(3,10,22,0.94)",
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
    queued: intelColors.warning,
    running: intelColors.cool,
    succeeded: intelColors.accent,
    failed: intelColors.danger,
    stale: "#C08CF5",
  }).map(([state, color]) => [state, { color, glow: `0 0 14px ${color}` }]),
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
