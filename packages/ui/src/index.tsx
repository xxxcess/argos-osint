"use client";
import React, { useState } from "react";
import { ROCK_TILE, rockTileImage } from "./rockTile";
import {
  View,
  Text,
  Pressable,
  StyleSheet,
  ActivityIndicator,
} from "react-native";

// Matte Fleetworks paint. Existing geometry and semantic colors remain separate.
export const colors = {
  bg: "#060709",
  panel: "#0B0F14",
  raised: "#121820",
  elevated: "#19212B",
  inset: "#060709",
  border: "#647587",
  borderSubtle: "#2B343F",
  text: "#D5DCE5",
  dim: "#A8B6C5",
  muted: "#8192A4",
  accent: "#24DCE3",
  accentPressed: "#1BB8C2",
  accentHover: "#68E7EB",
  onAccent: "#060709",
  selectionSurface: "#10272D",
  molten: "#E5B869",
  moltenSurface: "#241D12",
  accent2: "#c678dd",
  success: "#23dfa0",
  warning: "#f1c84b",
  danger: "#f280a0",
  // Compatibility names retain existing consumers with opaque matte fills.
  glass: "#0B0F14",
  glassStrong: "#121820",
  glassBorder: "#647587",
  cardEdge: "rgba(213, 220, 229, 0.02)",
  satinControlEdge: "rgba(213, 220, 229, 0.03)",
  accentSoft: "rgba(36, 220, 227, 0.1)",
};
export const fonts = {
  mono: 'ui-monospace, "SF Mono", "JetBrains Mono", Menlo, Consolas, monospace',
  body: 'system-ui, -apple-system, "Segoe UI", Roboto, Arial, sans-serif',
};
// Web-only style props (RN Web supports them); typed loosely for RN.
type WebStyle = Record<string, unknown>;
export const matteCard: WebStyle = {
  backgroundColor: colors.panel,
  borderWidth: 1,
  borderColor: colors.cardEdge,
};
export const matteDarkControl: WebStyle = {
  backgroundColor: colors.raised,
  borderWidth: 1,
  borderColor: colors.border,
};
export const mutedGoldPrimary: WebStyle = {
  backgroundColor: colors.accent,
  borderColor: colors.accent,
};
// Existing consumers retain their API while the visual treatment becomes matte.
export const glass = matteCard;
export const glow = (color: string): WebStyle => ({
  boxShadow: `inset 0 0 0 1px ${color}`,
});
/**
 * The diffuse perimeter of a group (theme.md section 2): a smoothstep alpha ramp of `ramp` px
 * inward from a `radius` boundary, content inset by `inset`. CSS has no alpha mask for a
 * background alone, so a canvas-colored inset shadow with `spread` and `blur = 2 sigma` draws a
 * Gaussian ramp fitted to the smoothstep (within 0.03 alpha, under one 8-bit level on these
 * colors). The same fit is used by the native `GroupEdge`. Children are never attenuated.
 */
export const groupEdge = {
  standard: { radius: 20, ramp: 16, inset: 24, spread: 8, blur: 7.5 },
  compact: { radius: 16, ramp: 12, inset: 20, spread: 6, blur: 5.6 },
};
export type GroupEdge = (typeof groupEdge)[keyof typeof groupEdge];
/** The group's coverage `depth` px inside its boundary: `t*t*(3-2t)`, `t = depth / ramp`. */
export function groupAlpha(edge: GroupEdge, depth: number): number {
  const t = Math.min(1, Math.max(0, depth / edge.ramp));
  return t * t * (3 - 2 * t);
}
export function groupSurface(edge: GroupEdge): WebStyle {
  return {
    backgroundColor: colors.panel,
    borderWidth: 0,
    borderRadius: edge.radius,
    padding: edge.inset,
    boxShadow: `inset 0 0 ${edge.blur}px ${edge.spread}px ${colors.bg}`,
  };
}
/**
 * Actions that submit, confirm, create or delete (Connect submits) carry the coarse matte rock
 * texture; every other control stays smooth. Fills and ink follow docs/design/matte-canvas/theme.md.
 */
export type ActionIntent = "submit" | "confirm" | "delete";
export const actionIntents: Record<
  ActionIntent,
  { rest: string; hover: string; pressed: string; ink: string }
> = {
  submit: {
    rest: "#10262C",
    hover: "#12282E",
    pressed: "#0E242A",
    ink: "#24DCE3",
  },
  confirm: {
    rest: "#10281F",
    hover: "#122A21",
    pressed: "#0E261D",
    ink: "#23DFA0",
  },
  delete: {
    rest: "#2B1823",
    hover: "#2D1A25",
    pressed: "#291621",
    ink: "#F280A0",
  },
};
/** Texture opacity per state: rest, hover, pressed, disabled. */
export const textureOpacity = {
  rest: 0.12,
  hover: 0.14,
  pressed: 0.1,
  disabled: 0.04,
};
/**
 * The approved crescent rock as a web-only background, anchored to the layer's top-left corner at
 * a fixed 256 px scale and repeated on both axes. It is drawn on an overlay layer whose own
 * `opacity` is the state opacity above.
 */
export const rockTextureStyle: WebStyle = {
  backgroundImage: rockTileImage,
  backgroundSize: `${ROCK_TILE}px ${ROCK_TILE}px`,
  backgroundPosition: "0 0",
  backgroundRepeat: "repeat",
};
/** The texture layer of an action: fills the control, never takes input, hidden from readers. */
export function RockTexture({ opacity }: { opacity: number }) {
  return (
    <View
      aria-hidden
      pointerEvents="none"
      style={[StyleSheet.absoluteFill, rockTextureStyle as object, { opacity }]}
    />
  );
}
export type Tone = "accent" | "success" | "warning" | "danger" | "purple";
export const toneColor: Record<Tone, string> = {
  accent: colors.accent,
  success: colors.success,
  warning: colors.warning,
  danger: colors.danger,
  purple: colors.accent2,
};

/** Bracketed uppercase telemetry tag, e.g. [ALERT: DEFCON 2]. */
export function StatusTag({
  children,
  tone = "accent",
}: {
  children: React.ReactNode;
  tone?: Tone;
}) {
  const color = toneColor[tone];
  return (
    <Text style={[s.tag, { color, borderColor: color + "66" }]}>
      [{children}]
    </Text>
  );
}
/** Matte container retaining the existing tone-tinted top rule. */
export function Frame({
  children,
  tone = "accent",
}: {
  children: React.ReactNode;
  tone?: Tone;
}) {
  return (
    <View
      style={[
        s.frame,
        glass as object,
        { borderTopColor: toneColor[tone], borderTopWidth: 2 },
      ]}
    >
      {children}
    </View>
  );
}

export function Label({ children }: { children: React.ReactNode }) {
  return <Text style={s.label}>{children}</Text>;
}
export function Copy({ children }: { children: React.ReactNode }) {
  return <Text style={s.copy}>{children}</Text>;
}
export function LoadingState({
  label,
  compact = false,
}: {
  label: string;
  compact?: boolean;
}) {
  return (
    <View
      accessibilityRole="progressbar"
      accessibilityLabel={label}
      accessibilityState={{ busy: true }}
      style={[s.loading, compact && { paddingVertical: 0 }]}
    >
      <ActivityIndicator
        size="small"
        color={colors.accent}
        aria-hidden={true}
      />
      <Text style={s.copy}>{label}</Text>
    </View>
  );
}
export function EmptyState({
  title,
  children,
}: {
  title: string;
  children?: React.ReactNode;
}) {
  return (
    <View style={s.empty}>
      <Text accessibilityRole="header" style={s.emptyTitle}>
        {title}
      </Text>
      {children != null ? <Copy>{children}</Copy> : null}
    </View>
  );
}

export function Button({
  children,
  onPress,
  active,
  disabled = false,
  label,
  variant = "default",
  intent,
}: {
  children: React.ReactNode;
  onPress: () => void;
  active?: boolean;
  disabled?: boolean;
  label?: string;
  variant?: "default" | "quiet" | "primary";
  intent?: ActionIntent;
}) {
  const [hovered, setHovered] = useState(false);
  const tint = intent ? actionIntents[intent] : undefined;
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled }}
      aria-pressed={active}
      disabled={disabled}
      onPress={onPress}
      onHoverIn={() => setHovered(true)}
      onHoverOut={() => setHovered(false)}
      style={({ pressed }) => [
        s.button,
        tint && s.action,
        tint && {
          backgroundColor: disabled
            ? colors.raised
            : pressed
              ? tint.pressed
              : hovered
                ? tint.hover
                : tint.rest,
        },
        variant === "quiet" && s.quiet,
        variant === "primary" && s.primary,
        !disabled &&
          hovered &&
          !pressed &&
          variant === "primary" &&
          s.primaryHover,
        !tint &&
          !disabled &&
          hovered &&
          !pressed &&
          variant !== "primary" &&
          s.hovered,
        !disabled && pressed && variant === "primary" && s.primaryPressed,
        !tint && !disabled && pressed && variant !== "primary" && s.pressed,
        !tint && active && s.active,
        !tint && disabled && s.disabled,
      ]}
    >
      {({ pressed }) => (
        <>
          {tint ? (
            <RockTexture
              opacity={
                disabled
                  ? textureOpacity.disabled
                  : pressed
                    ? textureOpacity.pressed
                    : hovered
                      ? textureOpacity.hover
                      : textureOpacity.rest
              }
            />
          ) : null}
          <Text
            style={[
              s.buttonText,
              variant === "quiet" && s.quietText,
              variant === "primary" && s.primaryText,
              tint && { color: disabled ? colors.muted : tint.ink },
              !tint && active && s.activeText,
              !tint && disabled && s.disabledText,
            ]}
          >
            {children}
          </Text>
        </>
      )}
    </Pressable>
  );
}
export function Panel({
  children,
  title,
}: {
  children: React.ReactNode;
  title?: string;
}) {
  return (
    <View style={s.panel}>
      {title ? (
        <Text accessibilityRole="header" style={s.panelTitle}>
          {title}
        </Text>
      ) : null}
      {children}
    </View>
  );
}
export function Notice({ children }: { children: React.ReactNode }) {
  return (
    <View accessibilityRole="alert" style={s.notice}>
      <Text style={s.noticeText}>{children}</Text>
    </View>
  );
}
export { ErrorBoundary } from "./ErrorBoundary";
const s = StyleSheet.create({
  loading: {
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    gap: 10,
    paddingVertical: 20,
    minWidth: 0,
  },
  empty: { paddingVertical: 24, gap: 8, minWidth: 0 },
  emptyTitle: {
    color: colors.text,
    fontSize: 16,
    lineHeight: 24,
    fontWeight: "600",
  },
  label: {
    fontFamily: fonts.mono,
    fontSize: 12,
    lineHeight: 19,
    color: colors.muted,
    textTransform: "uppercase",
    letterSpacing: 1.2,
  },
  copy: {
    fontFamily: fonts.body,
    color: colors.dim,
    fontSize: 15,
    lineHeight: 24,
  },
  button: {
    minHeight: 40,
    maxWidth: "100%",
    justifyContent: "center",
    paddingVertical: 9,
    paddingHorizontal: 12,
    borderWidth: 1,
    borderRadius: 2,
    backgroundColor: colors.raised,
    borderColor: colors.border,
  },
  action: {
    minHeight: 44,
    paddingHorizontal: 16,
    borderRadius: 10,
    borderColor: colors.border,
    boxShadow:
      "inset 0 1px 0 var(--fleetworks-specular-rim, rgba(213, 220, 229, 0.2))",
    overflow: "hidden",
  },
  buttonText: {
    textTransform: "uppercase",
    letterSpacing: 0.8,
    fontFamily: fonts.mono,
    color: colors.text,
    fontSize: 13,
    lineHeight: 20,
    flexShrink: 1,
  },
  quiet: { borderColor: colors.border, backgroundColor: "transparent" },
  quietText: { color: colors.dim },
  primary: {
    borderColor: colors.accent,
    backgroundColor: colors.accent,
  },
  primaryText: { color: colors.onAccent },
  hovered: {
    backgroundColor: colors.accentSoft,
    borderColor: colors.accent,
  },
  primaryHover: {
    backgroundColor: colors.accentHover,
    borderColor: colors.accentHover,
  },
  pressed: {
    borderColor: colors.accent,
    backgroundColor: colors.glassStrong,
  },
  primaryPressed: {
    borderColor: colors.accentPressed,
    backgroundColor: colors.accentPressed,
  },
  active: {
    borderColor: colors.accent,
    backgroundColor: colors.accent,
  },
  activeText: { color: colors.onAccent, fontWeight: "700" },
  disabled: {
    backgroundColor: "transparent",
    borderColor: colors.glassBorder,
  },
  disabledText: { color: colors.muted },
  panel: {
    ...groupSurface(groupEdge.standard),
    gap: 16,
    minWidth: 0,
  },
  frame: { padding: 16, gap: 12, minWidth: 0 },
  tag: {
    fontFamily: fonts.mono,
    fontSize: 12,
    lineHeight: 17,
    letterSpacing: 1,
    textTransform: "uppercase",
    fontWeight: "700",
  },
  panelTitle: {
    textTransform: "uppercase",
    letterSpacing: 1.5,
    fontFamily: fonts.mono,
    fontSize: 13,
    lineHeight: 21,
    color: colors.accent,
    paddingBottom: 14,
    marginBottom: 4,
    borderBottomWidth: 1,
    borderColor: colors.border,
  },
  notice: {
    paddingVertical: 12,
    paddingHorizontal: 16,
    borderLeftWidth: 2,
    borderColor: colors.danger,
    backgroundColor: "rgba(242, 128, 160, 0.1)",
    minWidth: 0,
  },
  noticeText: {
    fontFamily: fonts.body,
    fontSize: 15,
    lineHeight: 23,
    color: colors.danger,
  },
});

export { FleetworksMark } from "./FleetworksMark";
export { Skeleton } from "./Skeleton";
