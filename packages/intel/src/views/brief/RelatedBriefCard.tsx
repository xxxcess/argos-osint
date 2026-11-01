"use client";
import React, { useState } from "react";
import { View, Text, Pressable, useWindowDimensions } from "react-native";
import type { Article } from "@argos/api-client";
import { intelColors as c } from "../../shell/intel-theme";
import { briefStyles as s } from "./briefStyles";
export function RelatedBriefCard({
  article,
  basis,
  tentative,
  onPress,
}: {
  article: Article;
  basis: string[];
  tentative: boolean;
  onPress: () => void;
}) {
  const { width } = useWindowDimensions();
  const [hovered, setHovered] = useState(false);
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`Open Focus Brief for related lead: ${article.title}`}
      onPress={onPress}
      onHoverIn={() => setHovered(true)}
      onHoverOut={() => setHovered(false)}
      style={[
        s.related,
        width >= 900 && { maxWidth: 650 },
        hovered && { borderColor: c.borderStrong },
      ]}
    >
      <View style={s.meta}>
        <Text style={s.micro}>{article.category.toUpperCase()}</Text>
        <Text style={s.micro}>
          {article.published_at.slice(0, 10) || "Date unavailable"}
        </Text>
      </View>
      <Text style={s.relatedTitle}>{article.title}</Text>
      <Text numberOfLines={2} style={s.relatedCopy}>
        {article.description || "No description available."}
      </Text>
      <Text style={s.micro}>
        {basis.join(" · ") || "Basis unavailable"}
        {tentative ? " · tentative" : ""} · Related lead, not corroboration
      </Text>
    </Pressable>
  );
}
