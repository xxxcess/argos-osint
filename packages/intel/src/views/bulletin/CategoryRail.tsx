"use client";
import React, { useEffect, useState } from "react";
import {
  View,
  Text,
  Pressable,
  ScrollView,
  Platform,
  StyleSheet,
  Animated,
} from "react-native";
import { INTEL_CATEGORIES, INTEL_DOMAIN_NAMES } from "@argos/api-client";
import { useReducedMotion } from "../../shell/motion";
import { moveCategoryFocus } from "../category-keyboard.web";
import { intelColors as c } from "../../shell/intel-theme";
import { intelFonts } from "../../shell/intel-fonts";

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
}: {
  categories: string[];
  category: string;
  counts?: Record<string, number>;
  onCategory: (category: string) => void;
  wide: boolean;
}) {
  const p = { categories, category, counts, onCategory };
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
  function articleCount(category: string) {
    return String(counts?.[category] ?? 0);
  }
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
      {p.categories.map((category) => (
        <Pressable
          key={category}
          accessibilityRole="button"
          accessibilityLabel={categoryLabel(category)}
          accessibilityState={{ selected: p.category === category }}
          aria-pressed={p.category === category}
          onPress={() => p.onCategory(category)}
          style={({ pressed }) => [
            s.category,
            !wide && s.categoryChip,
            p.category === category && s.selectedCategory,
            pressed && { borderColor: c.select },
          ]}
        >
          <Text
            style={[
              s.categoryName,
              p.category === category && { color: c.accent, fontWeight: "700" },
            ]}
          >
            {categoryLabel(category)}
          </Text>
          <Text
            accessibilityLabel={`Article count for ${categoryLabel(category)}`}
            style={[
              s.countBadge,
              p.category === category && {
                color: c.accent,
                backgroundColor: c.accentSoft,
              },
            ]}
          >
            {articleCount(category)}
          </Text>
        </Pressable>
      ))}
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
