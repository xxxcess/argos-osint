/** Intel-only Nexus Desk tokens. The shared Argos/TUI palette is unchanged. */
export const intelColors = {
  bg: "#0A0F1A",
  panel: "#151C2B",
  panelRaised: "#1A2232",
  inset: "#111726",
  scrim: "rgba(10,15,26,0.72)",
  border: "#232C3D",
  borderStrong: "#2E3850",
  divider: "#1F2737",
  text: "#E6EAF2",
  dim: "#A9B1C1",
  muted: "#6B7487",
  faint: "#4A5366",
  accent: "#5FD39A",
  accentSoft: "rgba(95,211,154,0.10)",
  accentBorder: "rgba(95,211,154,0.35)",
  select: "#7FE0B4",
  textSelection: "#2E3850",
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
  card: 32,
};
export const intelRadius = { shell: 16, card: 14, control: 8, chip: 4 };
export const intelType = {
  display: { fontSize: 40, fontWeight: "800" as const, letterSpacing: -0.5 },
  title: { fontSize: 26, fontWeight: "700" as const },
  body: { fontSize: 18, lineHeight: 28 },
  row: { fontSize: 18 },
  section: { fontSize: 15, fontWeight: "700" as const, letterSpacing: 1.2 },
  micro: { fontSize: 12, fontWeight: "600" as const, letterSpacing: 1.6 },
  chip: { fontSize: 12, fontWeight: "700" as const, letterSpacing: 1.2 },
  launcher: { fontSize: 22, fontWeight: "800" as const, letterSpacing: 6 },
};

/** Graph-only treatments, matching the evidence topology reference. */
export const topologyColors = {
  bg: "#030711",
  panel: "rgba(10,17,30,0.94)",
  grid: "rgba(63,83,119,0.09)",
  axis: "rgba(86,107,140,0.18)",
  accent: "#00DFB1",
  edge: "#167C72",
  labelBg: "rgba(3,10,22,0.94)",
};
