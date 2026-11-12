/** Fleetworks web paint; layout/type/motion and the Rust TUI remain separate. */
export const intelColors = {
  bg: "#111111",
  panel: "#141414",
  panelRaised: "#232321",
  inset: "#0D0D0D",
  scrim: "rgba(13,13,13,0.78)",
  border: "#3A3833",
  borderStrong: "#898274",
  divider: "#3A3833",
  text: "#F3EEE4",
  dim: "#C6C0B5",
  muted: "#A09A8F",
  faint: "#A09A8F",
  accent: "#C6A56E",
  accentPressed: "#C4A265",
  accentHover: "#D8BD8C",
  onAccent: "#111111",
  selectionSurface: "#322C22",
  accentSoft: "rgba(198,165,110,0.10)",
  accentBorder: "rgba(198,165,110,0.75)",
  select: "#D8BD8C",
  textSelection: "#322C22",
  success: "#5FD39A",
  danger: "#F06A8A",
  dangerSoft: "rgba(240,106,138,0.12)",
  cool: "#5AB8F0",
  warning: "#E5B45B",
  fab: "#232321",
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
  accentGlow: "0 0 18px rgba(198,165,110,0.35)",
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
  bg: "#0D0D0D",
  panel: "rgba(20,20,20,0.94)",
  grid: "rgba(137,130,116,0.09)",
  axis: "rgba(137,130,116,0.18)",
  accent: "#C6A56E",
  edge: "#A79B80",
  labelBg: "rgba(13,13,13,0.94)",
  ghost: "rgba(20,20,20,0.9)",
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
    succeeded: intelColors.success,
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
