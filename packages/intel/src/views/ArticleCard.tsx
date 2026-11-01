"use client";
import React, { useState } from "react";
import {
  View,
  Text,
  Image,
  Pressable,
  StyleSheet,
  useWindowDimensions,
} from "react-native";
import type { Article } from "@argos/api-client";
import { intelColors as c, intelRadius } from "../shell/intel-theme";
import { intelFonts } from "../shell/intel-fonts";

function safeImage(url: string) {
  try {
    return ["http:", "https:"].includes(new URL(url).protocol);
  } catch {
    return false;
  }
}

export function ArticleCard({
  article: a,
  onPress,
  featured = false,
}: {
  article: Article;
  onPress: () => void;
  featured?: boolean;
}) {
  const compact = useWindowDimensions().width < 600;
  const [failedImage, setFailedImage] = useState("");
  const [hovered, setHovered] = useState(false);
  const thumbnail = safeImage(a.image_url) && failedImage !== a.image_url;
  return (
    <Pressable
      testID={`article-card-${a.id}`}
      accessibilityRole="button"
      accessibilityLabel={`Open Focus Brief: ${a.title}`}
      onPress={onPress}
      onHoverIn={() => setHovered(true)}
      onHoverOut={() => setHovered(false)}
      style={({ pressed }) => [
        s.card,
        compact && { padding: 16, gap: 12 },
        (hovered || pressed) && { borderColor: c.borderStrong },
      ]}
    >
      <View style={s.content}>
        <View style={s.meta}>
          <Text style={s.category}>{a.category.toUpperCase()}</Text>
          <Text style={s.date}>{a.published_at.slice(0, 10)}</Text>
        </View>
        {thumbnail ? (
          <Image
            accessibilityLabel={`Article image: ${a.title}`}
            source={{ uri: a.image_url }}
            resizeMode="cover"
            onError={() => setFailedImage(a.image_url)}
            style={{
              width: "100%",
              height: compact ? 140 : featured ? 220 : 160,
              borderRadius: 8,
            }}
          />
        ) : null}
        <Text
          accessibilityRole="header"
          style={[s.title, compact && { fontSize: 22, lineHeight: 30 }]}
        >
          {a.title}
        </Text>
        <Text
          numberOfLines={3}
          style={[s.summary, compact && { fontSize: 17, lineHeight: 27 }]}
        >
          {a.description}
        </Text>
        <Text style={s.source}>
          {a.source_name || a.source_domain} · {a.author || "Source article"}
        </Text>
      </View>
    </Pressable>
  );
}
const s = StyleSheet.create({
  card: {
    flexDirection: "column",
    gap: 24,
    padding: 32,
    borderWidth: 1,
    borderColor: c.border,
    borderRadius: intelRadius.card,
    backgroundColor: c.panelRaised,
    minWidth: 0,
  },
  content: { flex: 1, minWidth: 0, gap: 14 },
  meta: {
    flexDirection: "row",
    alignItems: "center",
    flexWrap: "wrap",
    gap: 32,
    minWidth: 0,
  },
  category: {
    fontFamily: intelFonts.body,
    fontSize: 18,
    fontWeight: "700",
    letterSpacing: 1.5,
    color: c.dim,
    flexShrink: 1,
    minWidth: 0,
    maxWidth: "100%",
  },
  date: {
    fontFamily: intelFonts.mono,
    fontSize: 18,
    color: c.dim,
    minWidth: 0,
    maxWidth: "100%",
  },
  title: {
    fontFamily: intelFonts.body,
    fontSize: 28,
    lineHeight: 36,
    fontWeight: "700",
    color: c.text,
  },
  summary: {
    fontFamily: intelFonts.body,
    fontSize: 20,
    lineHeight: 32,
    color: c.dim,
    maxWidth: 700,
  },
  source: {
    fontFamily: intelFonts.mono,
    fontSize: 12,
    lineHeight: 20,
    color: c.dim,
  },
});
