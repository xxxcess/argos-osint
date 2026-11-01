"use client";
import React, { useState } from "react";
import {
  View,
  Text,
  TextInput,
  ScrollView,
  Pressable,
  Platform,
  StyleSheet,
  useWindowDimensions,
} from "react-native";
import { Notice, LoadingState, EmptyState } from "@argos/ui";
import { keyFor, type Article, type useArticles } from "@argos/api-client";
import { ArticleCard } from "./ArticleCard";
import { DeskButton, Globe } from "./desk-ui";
import { moveCategoryFocus } from "./category-keyboard.web";
import { intelColors as c, intelRadius } from "../shell/intel-theme";
import { intelFonts } from "../shell/intel-fonts";

type Props = {
  categories: string[];
  category: string;
  onCategory: (category: string) => void;
  search: string;
  onSearch: (search: string) => void;
  days: string[];
  selectedDay: string;
  onDay: (day: string) => void;
  daysPending: boolean;
  daysSuccess: boolean;
  daysError: Error | null;
  onRetryDays: () => void;
  articles: Article[];
  counts?: Record<string, number>;
  list: ReturnType<typeof useArticles>;
  onArticle: (article: Article) => void;
  onHome: () => void;
  onReset: () => void;
};

function BulletinSkeleton() {
  return (
    <View
      accessibilityRole="progressbar"
      accessibilityLabel="Loading bulletin…"
      accessibilityState={{ busy: true }}
      style={s.skeletons}
    >
      {[0, 1, 2].map((id) => (
        <View key={id} style={s.skeleton} testID="bulletin-skeleton">
          <View style={s.skeletonLine} />
          <View style={[s.skeletonLine, { width: "85%" }]} />
          <View style={[s.skeletonLine, { width: "65%" }]} />
        </View>
      ))}
    </View>
  );
}

export function BulletinView(p: Props) {
  const { width } = useWindowDimensions();
  const wide = width >= 900;
  const [daysOpen, setDaysOpen] = useState(false);
  function articleCount(category: string) {
    const count = p.counts?.[category];
    return count == null ? "0" : String(count);
  }
  const categoryItems = (
    <View
      nativeID="intel-categories"
      style={[s.categories, !wide && { flexDirection: "row" }]}
      {...(Platform.OS === "web" ? { onKeyDown: moveCategoryFocus } : {})}
    >
      {p.categories.map((category) => (
        <Pressable
          key={category}
          accessibilityRole="button"
          accessibilityLabel={category[0].toUpperCase() + category.slice(1)}
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
            {category[0].toUpperCase() + category.slice(1)}
          </Text>
          <Text
            accessibilityLabel="Loaded article count, not a total"
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
  return (
    <View nativeID="intel-bulletin" style={s.shell}>
      <View
        style={[
          s.header,
          wide && {
            flexDirection: "row",
            alignItems: "center",
          },
          !wide && { paddingHorizontal: 24 },
        ]}
      >
        <View style={[s.headingRow, wide && { gap: 24 }]}>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Home"
            onPress={p.onHome}
            style={s.back}
          >
            <Text style={s.backGlyph}>‹</Text>
          </Pressable>
          <Globe />
          <Text
            accessibilityRole="header"
            style={[s.title, width >= 1200 && { fontSize: 30, lineHeight: 38 }]}
          >
            Intelligence bulletin
          </Text>
        </View>
        <View style={[s.search, wide && { flex: 1, maxWidth: 512 }]}>
          <Text nativeID="intel-search-label" style={s.micro}>
            Search articles
          </Text>
          <View style={s.searchField}>
            <View
              accessibilityElementsHidden
              importantForAccessibility="no-hide-descendants"
              style={s.searchGlyph}
            >
              <View style={s.searchLens} />
              <View style={s.searchHandle} />
            </View>
            <TextInput
              accessibilityLabel="Search articles"
              placeholder="Query intelligence database..."
              placeholderTextColor={c.dim}
              value={p.search}
              onChangeText={p.onSearch}
              style={[s.input, width < 600 && { fontSize: 16 }]}
            />
          </View>
        </View>
      </View>
      <View style={wide && { flexDirection: "row" }}>
        <View
          style={[
            s.rail,
            wide && { width: width >= 1200 ? 410 : 280, borderRightWidth: 1 },
          ]}
        >
          <Text style={[s.micro, s.railHeading]}>CATEGORIES</Text>
          {wide ? (
            categoryItems
          ) : (
            <ScrollView
              horizontal
              style={{ flexGrow: 0 }}
              contentContainerStyle={{ paddingBottom: 4 }}
            >
              {categoryItems}
            </ScrollView>
          )}
          <View style={s.railSupport}>
            <Text accessibilityLiveRegion="polite" style={s.loaded}>
              {p.list.data || (p.daysSuccess && p.days.length === 0)
                ? `${p.articles.length} loaded`
                : "—"}
            </Text>
            {p.days.length > 0 ? (
              <View style={s.dayMenu}>
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel={`Retained day ${p.selectedDay}`}
                  accessibilityState={{ expanded: daysOpen }}
                  aria-expanded={daysOpen}
                  onPress={() => setDaysOpen((open) => !open)}
                  style={s.dayTrigger}
                >
                  <Text style={s.dayLabel}>{p.selectedDay}</Text>
                  <Text style={s.dayLabel}>{daysOpen ? "▴" : "▾"}</Text>
                </Pressable>
                {daysOpen
                  ? p.days.map((day) => (
                      <Pressable
                        key={day}
                        accessibilityRole="button"
                        accessibilityLabel={`Day ${day}`}
                        accessibilityState={{ selected: day === p.selectedDay }}
                        onPress={() => {
                          p.onDay(day);
                          setDaysOpen(false);
                        }}
                        style={[
                          s.dayOption,
                          day === p.selectedDay && {
                            borderColor: c.accentBorder,
                          },
                        ]}
                      >
                        <Text style={s.dayLabel}>{day}</Text>
                      </Pressable>
                    ))
                  : null}
              </View>
            ) : null}
          </View>
        </View>
        <View style={[s.feed, !wide && { padding: 16 }]}>
          {p.daysError ? (
            <View style={{ gap: 12 }}>
              <Notice>{String(p.daysError)}</Notice>
              <DeskButton onPress={p.onRetryDays}>
                Retry retained days
              </DeskButton>
            </View>
          ) : null}
          {p.list.isError && !p.list.isFetchNextPageError ? (
            <View style={{ gap: 12 }}>
              <Notice>{String(p.list.error)}</Notice>
              <DeskButton
                onPress={() => {
                  void p.list.refetch();
                }}
              >
                Retry bulletin
              </DeskButton>
            </View>
          ) : null}
          {p.daysPending ? (
            <LoadingState label="Loading retained days…" />
          ) : null}
          {p.list.isPending && !!p.selectedDay ? <BulletinSkeleton /> : null}
          {p.daysSuccess && !p.selectedDay ? (
            <EmptyState title="No articles yet">
              Collect an Atlas cycle in Argos to populate this bulletin.
            </EmptyState>
          ) : null}
          {p.list.isSuccess && !!p.selectedDay && p.articles.length === 0 ? (
            <View style={{ gap: 16 }}>
              <EmptyState title="NO MATCHING INTELLIGENCE">
                No articles match this day, category and search.
              </EmptyState>
              <DeskButton onPress={p.onReset}>Reset filters</DeskButton>
            </View>
          ) : null}
          {p.list.isRefetching && !p.list.isFetchingNextPage ? (
            <LoadingState label="Refreshing bulletin…" compact />
          ) : null}
          <View style={s.cards}>
            {p.articles.map((article, index) => (
              <ArticleCard
                key={keyFor(article)}
                article={article}
                featured={index === 0}
                onPress={() => p.onArticle(article)}
              />
            ))}
          </View>
          {p.list.isFetchingNextPage ? (
            <LoadingState label="Loading more articles…" />
          ) : null}
          {p.list.isFetchNextPageError ? (
            <Notice>
              Could not load more articles. {p.list.error?.message}
            </Notice>
          ) : null}
          {p.list.hasNextPage || p.list.isFetchNextPageError ? (
            <DeskButton
              disabled={p.list.isFetching}
              onPress={() => {
                void p.list.fetchNextPage({ cancelRefetch: false });
              }}
            >
              {p.list.isFetchingNextPage
                ? "Loading…"
                : p.list.isFetchNextPageError
                  ? "Retry loading articles"
                  : "Load more articles"}
            </DeskButton>
          ) : null}
        </View>
      </View>
    </View>
  );
}

const s = StyleSheet.create({
  shell: {
    width: "100%",
    borderRadius: intelRadius.shell,
    borderWidth: 1,
    borderColor: c.border,
    backgroundColor: c.panel,
    overflow: "hidden",
  },
  header: {
    // 40px horizontal inset; vertical inset keeps the 64px field in a ~120px band.
    paddingHorizontal: 40,
    paddingVertical: 28,
    borderBottomWidth: 1,
    borderColor: c.border,
    gap: 24,
    justifyContent: "space-between",
  },
  headingRow: {
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    gap: 16,
    minWidth: 0,
    flexShrink: 1,
  },
  back: {
    width: 58,
    height: 58,
    borderRadius: 8,
    borderWidth: 1,
    borderColor: c.borderStrong,
    backgroundColor: c.inset,
    alignItems: "center",
    justifyContent: "center",
  },
  backGlyph: { fontSize: 30, color: c.text },
  title: {
    color: c.text,
    fontFamily: intelFonts.body,
    fontSize: 26,
    lineHeight: 34,
    fontWeight: "800",
    textTransform: "uppercase",
    flexShrink: 1,
  },
  search: { gap: 8, minWidth: 0 },
  searchField: { minWidth: 0, width: "100%" },
  searchGlyph: {
    position: "absolute",
    left: 20,
    top: 21,
    width: 22,
    height: 22,
    zIndex: 1,
    pointerEvents: "none",
  },
  searchLens: {
    width: 16,
    height: 16,
    borderRadius: 8,
    borderWidth: 2,
    borderColor: c.accent,
  },
  searchHandle: {
    position: "absolute",
    width: 9,
    height: 2,
    left: 13,
    top: 16,
    backgroundColor: c.accent,
    transform: [{ rotate: "45deg" }],
  },
  micro: {
    color: c.dim,
    fontFamily: intelFonts.mono,
    fontSize: 12,
    fontWeight: "600",
    letterSpacing: 1.6,
  },
  input: {
    minHeight: 64,
    backgroundColor: c.inset,
    borderWidth: 1,
    borderColor: c.borderStrong,
    borderRadius: 10,
    paddingLeft: 64,
    paddingRight: 20,
    width: "100%",
    fontFamily: intelFonts.mono,
    color: c.text,
    fontSize: 20,
    minWidth: 0,
  },
  rail: { gap: 20, padding: 28, borderColor: c.border, flexShrink: 0 },
  railHeading: { fontSize: 15, fontWeight: "700" },
  railSupport: { gap: 12, minWidth: 0 },
  railActions: {
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    gap: 8,
    minWidth: 0,
  },
  categories: { gap: 12 },
  category: {
    minHeight: 70,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 16,
    paddingHorizontal: 26,
    borderRadius: 10,
    borderWidth: 1,
    borderColor: "transparent",
  },
  categoryChip: { minHeight: 44, paddingHorizontal: 16 },
  selectedCategory: {
    backgroundColor: c.accentSoft,
    borderColor: c.accentBorder,
  },
  categoryName: {
    fontFamily: intelFonts.body,
    fontSize: 20,
    color: c.dim,
    flexShrink: 1,
  },
  countBadge: {
    fontFamily: intelFonts.mono,
    fontSize: 16,
    color: c.dim,
    backgroundColor: c.inset,
    borderRadius: 18,
    minWidth: 36,
    height: 36,
    textAlign: "center",
    lineHeight: 36,
  },
  feed: {
    flex: 1,
    minWidth: 0,
    padding: 38,
    gap: 24,
    backgroundColor: c.inset,
  },
  loaded: { fontFamily: intelFonts.mono, fontSize: 12, color: c.dim },
  dayMenu: { gap: 8, minWidth: 0 },
  dayTrigger: {
    minHeight: 44,
    paddingHorizontal: 16,
    borderRadius: 8,
    borderWidth: 1,
    borderColor: c.borderStrong,
    backgroundColor: c.inset,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  dayOption: {
    minHeight: 40,
    paddingHorizontal: 16,
    borderRadius: 8,
    borderWidth: 1,
    borderColor: c.border,
    backgroundColor: c.panel,
    justifyContent: "center",
  },
  dayLabel: { fontFamily: intelFonts.mono, fontSize: 14, color: c.text },
  cards: { gap: 26 },
  skeletons: { gap: 26 },
  skeleton: {
    padding: 32,
    minHeight: 180,
    gap: 20,
    backgroundColor: c.panelRaised,
    borderRadius: intelRadius.card,
    borderWidth: 1,
    borderColor: c.border,
  },
  skeletonLine: {
    width: "45%",
    height: 18,
    backgroundColor: c.inset,
    borderRadius: 4,
  },
});
