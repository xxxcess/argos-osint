"use client";
import React, { createContext, useContext, useEffect, useState } from "react";
import {
  View,
  Text,
  Pressable,
  ScrollView,
  Platform,
  StyleSheet,
  Animated,
} from "react-native";
import {
  INTEL_CATEGORIES,
  INTEL_DOMAIN_NAMES,
  useOptionalBackend,
} from "@argos/api-client";
import { useReducedMotion } from "../../shell/motion";
import { moveCategoryFocus } from "../category-keyboard.web";
import { intelColors as c } from "../../shell/intel-theme";
import { intelFonts } from "../../shell/intel-fonts";

export type CategoryCountsSync = {
  unsynced: boolean;
  updatedAt?: number;
  reason?: string;
};

export const CategoryCountsSyncContext =
  createContext<CategoryCountsSync | null>(null);

function formatSyncTime(updatedAt?: number): string | null {
  if (!updatedAt || !Number.isFinite(updatedAt)) return null;
  return new Date(updatedAt).toLocaleTimeString("en-US");
}

function unsyncedCountLabel(
  label: string,
  updatedAt?: number,
  reason?: string,
): string {
  const parts = [`Article count for ${label} unsynced`];
  const at = formatSyncTime(updatedAt);
  if (at) parts.push(`last updated ${at}`);
  if (reason?.trim()) parts.push(reason.trim());
  return parts.join(" · ");
}

function categoryLabel(category: string) {
  const domain = INTEL_CATEGORIES.find((id) => id === category);
  return domain
    ? INTEL_DOMAIN_NAMES[domain]
    : category.slice(0, 1).toUpperCase() + category.slice(1);
}

export function CategoryRail({
  categories,
  category,
  counts,
  onCategory,
  wide,
  unsynced,
  unsyncedReason,
  unsyncedUpdatedAt,
}: {
  categories: string[];
  category: string;
  counts?: Record<string, number>;
  onCategory: (category: string) => void;
  wide: boolean;
  unsynced?: boolean;
  unsyncedReason?: string;
  unsyncedUpdatedAt?: number;
}) {
  const p = { categories, category, counts, onCategory };
  const countsSync = useContext(CategoryCountsSyncContext);
  const backend = useOptionalBackend();
  const isUnsynced = Boolean(
    unsynced || countsSync?.unsynced || backend?.stale,
  );
  const reason =
    unsyncedReason ??
    countsSync?.reason ??
    (backend?.stale ? backend.connectionError : undefined);
  const updatedAt =
    unsyncedUpdatedAt ?? countsSync?.updatedAt ?? backend?.lastSyncedAt;
  const reduced = useReducedMotion();
  const [top] = useState(() => new Animated.Value(0));
  const active = Math.max(0, categories.indexOf(category));
  useEffect(() => {
    const animation = Animated.timing(top, {
      toValue: active * 52,
      duration: reduced ? 0 : 200,
      useNativeDriver: false,
    });
    animation.start();
    return () => animation.stop();
  }, [active, reduced, top]);
  const categoryItems = (
    <View
      nativeID="intel-categories"
      style={[s.categories, !wide && { flexDirection: "row" }]}
      {...(Platform.OS === "web" ? { onKeyDown: moveCategoryFocus } : {})}
    >
      {wide ? (
        <Animated.View
          aria-hidden
          pointerEvents="none"
          style={{
            position: "absolute",
            left: 0,
            top,
            width: 2,
            height: 44,
            backgroundColor: c.accent,
          }}
        />
      ) : null}
      {p.categories.map((item) => {
        const label = categoryLabel(item);
        const value = counts?.[item];
        const hasCount = typeof value === "number" && Number.isFinite(value);
        return (
          <Pressable
            key={item}
            accessibilityRole="button"
            accessibilityLabel={label}
            accessibilityState={{ selected: p.category === item }}
            aria-pressed={p.category === item}
            onPress={() => p.onCategory(item)}
            style={({ pressed }) => [
              s.category,
              !wide && s.categoryChip,
              p.category === item && s.selectedCategory,
              pressed && { borderColor: c.select },
            ]}
          >
            <Text
              style={[
                s.categoryName,
                p.category === item && { color: c.accent, fontWeight: "700" },
              ]}
            >
              {label}
            </Text>
            <View style={s.countGroup}>
              {hasCount && isUnsynced ? (
                <Text
                  accessibilityLabel={unsyncedCountLabel(
                    label,
                    updatedAt,
                    reason,
                  )}
                  style={s.unsyncedGlyph}
                >
                  ◇
                </Text>
              ) : null}
              <Text
                accessibilityLabel={
                  hasCount
                    ? `Article count for ${label}: ${value}`
                    : `Article count for ${label}: unavailable`
                }
                style={[
                  s.countBadge,
                  p.category === item && {
                    color: c.accent,
                    backgroundColor: c.accentSoft,
                  },
                ]}
              >
                {hasCount ? String(value) : "—"}
              </Text>
            </View>
          </Pressable>
        );
      })}
    </View>
  );
  return wide ? (
    categoryItems
  ) : (
    <ScrollView horizontal>{categoryItems}</ScrollView>
  );
}
const s = StyleSheet.create({
  categories: { gap: 8 },
  category: {
    minHeight: 44,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 8,
    paddingHorizontal: 12,
    borderRadius: 8,
    borderWidth: 1,
    borderColor: "transparent",
  },
  categoryChip: { paddingHorizontal: 12 },
  selectedCategory: {
    backgroundColor: c.accentSoft,
    borderColor: c.accentBorder,
  },
  categoryName: {
    fontFamily: intelFonts.mono,
    fontSize: 13,
    color: c.dim,
    flexShrink: 1,
  },
  countGroup: {
    flexDirection: "row",
    alignItems: "center",
    gap: 4,
  },
  unsyncedGlyph: {
    fontFamily: intelFonts.mono,
    fontSize: 12,
    color: c.muted,
  },
  countBadge: {
    fontFamily: intelFonts.mono,
    fontSize: 13,
    color: c.dim,
    backgroundColor: c.inset,
    borderRadius: 14,
    minWidth: 28,
    height: 28,
    textAlign: "center",
    lineHeight: 29,
  },
});
