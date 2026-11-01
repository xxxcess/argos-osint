"use client";
import React, { useState } from "react";
import { View, Text, Image, Pressable, StyleSheet } from "react-native";
import type { Article } from "@argos/api-client";
import { intelColors as c, intelRadius } from "../../shell/intel-theme";
import { intelFonts } from "../../shell/intel-fonts";

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
  featured: _featured = false,
}: {
  article: Article;
  onPress: () => void;
  featured?: boolean;
}) {
  const [focused, setFocused] = useState(false);
  const [failedImage, setFailedImage] = useState("");
  const [hovered, setHovered] = useState(false);
  const thumbnail = safeImage(a.image_url) && failedImage !== a.image_url;
  const source = a.source_name || a.source_domain;
  const byline =
    source && a.author ? `${source} · ${a.author}` : source || a.author;
  return (
    <Pressable
      testID={`article-card-${a.id}`}
      accessibilityRole="button"
      accessibilityLabel={`Open Focus Brief: ${a.title}`}
      onPress={onPress}
      onFocus={() => setFocused(true)}
      onBlur={() => setFocused(false)}
      onHoverIn={() => setHovered(true)}
      onHoverOut={() => setHovered(false)}
      style={({ pressed }) => [
        s.card,
        (hovered || focused || pressed) && {
          borderColor: c.accent,
          backgroundColor: c.panelRaised,
        },
      ]}
    >
      <View style={s.content}>
        <View style={s.meta}>
          <Text style={s.category}>{a.category.toUpperCase()}</Text>
          <Text style={s.date}>{a.published_at.slice(0, 10)}</Text>
        </View>
        <Text accessibilityRole="header" numberOfLines={2} style={s.title}>
          {a.title}
        </Text>
        {a.description ? (
          <Text numberOfLines={3} style={s.summary}>
            {a.description}
          </Text>
        ) : null}
        {byline ? <Text style={s.source}>{byline}</Text> : null}
      </View>
      {thumbnail ? (
        <Image
          accessibilityLabel={`Article image: ${a.title}`}
          source={{ uri: a.image_url }}
          resizeMode="cover"
          onError={() => setFailedImage(a.image_url)}
          style={{
            width: 72,
            height: 72,
            borderRadius: 8,
          }}
        />
      ) : null}
    </Pressable>
  );
}
const s = StyleSheet.create({
  card: {
    flexDirection: "row",
    gap: 12,
    padding: 16,
    borderWidth: 1,
    borderColor: c.border,
    borderRadius: intelRadius.card,
    backgroundColor: c.panel,
    minWidth: 0,
  },
  content: { flex: 1, minWidth: 0, gap: 6 },
  meta: {
    flexDirection: "row",
    alignItems: "center",
    flexWrap: "wrap",
    gap: 12,
    minWidth: 0,
  },
  category: {
    lineHeight: 16,
    fontFamily: intelFonts.mono,
    fontSize: 12,
    fontWeight: "600",
    color: c.dim,
    flexShrink: 1,
    minWidth: 0,
    maxWidth: "100%",
  },
  date: {
    lineHeight: 16,
    fontFamily: intelFonts.mono,
    fontSize: 12,
    color: c.dim,
    minWidth: 0,
    maxWidth: "100%",
  },
  title: {
    fontFamily: intelFonts.body,
    fontSize: 18,
    lineHeight: 24,
    fontWeight: "600",
    color: c.text,
  },
  summary: {
    fontFamily: intelFonts.body,
    fontSize: 15,
    lineHeight: 22,
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
