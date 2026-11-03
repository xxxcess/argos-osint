"use client";
import React, { useState } from "react";
import { Pressable, Text } from "react-native";
import type { Seed } from "@argos/api-client";
import { intelColors as c, intelEffects } from "../../shell/intel-theme";
import { briefStyles as s } from "./briefStyles";
export function hasMemorySeed(seed: Seed | null | undefined): seed is Seed {
  return (
    !!seed &&
    [seed.Actor, seed.Claim, seed.Inference].some(
      (value) => typeof value === "string" && !!value,
    )
  );
}

/** Extracted brief graph. Indexing is not required for items already on the brief. */
export function extractedBriefSeed(article: {
  run_id: string;
  id: string;
}): Seed | null {
  if (!article.run_id || !article.id) return null;
  return { Brief: `brief:${article.run_id}/${article.id}` };
}

export function topologyTarget(
  seed: Seed | null | undefined,
  extracted: Seed | null,
): Seed | null {
  if (hasMemorySeed(seed)) return seed;
  return extracted;
}

/** Node id to select when navigation stays on the extracted brief graph. */
export function extractedFocus(
  seed: Seed | null,
  extracted: Seed | null,
  nodeId: string,
): string | undefined {
  if (!seed || !extracted || seed.Brief !== extracted.Brief || !nodeId)
    return undefined;
  return nodeId;
}
export function BriefAction({
  children,
  label,
  onPress,
  variant = "row",
  disabled = false,
}: {
  children: React.ReactNode;
  label?: string;
  onPress: () => void;
  variant?: "row" | "back" | "cta";
  disabled?: boolean;
}) {
  const cta = variant === "cta";
  const back = variant === "back";
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled }}
      disabled={disabled}
      onPress={onPress}
      onHoverIn={() => setHovered(true)}
      onHoverOut={() => setHovered(false)}
      onFocus={() => setFocused(true)}
      onBlur={() => setFocused(false)}
      style={({ pressed }) => [
        s.action,
        disabled && { opacity: 0.45 },
        cta && s.cta,
        back && s.back,
        (hovered || focused) && {
          borderColor: c.accent,
          backgroundColor: back ? "transparent" : c.accentSoft,
        },
        cta && (hovered || focused) && { boxShadow: intelEffects.accentGlow },
        pressed && { backgroundColor: c.panelRaised },
      ]}
    >
      <Text
        style={[
          s.actionText,
          cta && s.ctaText,
          back && { color: hovered || focused ? c.accent : c.dim },
        ]}
      >
        {children}
      </Text>
    </Pressable>
  );
}
