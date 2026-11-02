"use client";
import React from "react";
import {
  View,
  Text,
  TextInput,
  ScrollView,
  StyleSheet,
  useWindowDimensions,
} from "react-native";
import { Notice, LoadingState, EmptyState } from "@argos/ui";
import { keyFor, type Article, type useArticles } from "@argos/api-client";
import { BulletinSkeleton } from "./BulletinSkeleton";
import { FeedHeader } from "./FeedHeader";
import { ArticleCard } from "./ArticleCard";
import { DeskButton } from "../desk-ui";
import { CategoryRail } from "./CategoryRail";
import { intelColors as c, intelRadius } from "../../shell/intel-theme";
import { intelFonts } from "../../shell/intel-fonts";

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
  onReset: () => void;
};

export function BulletinView(p: Props) {
  const { width } = useWindowDimensions();
  const wide = width >= 900;
  const loading =
    !p.list.data &&
    p.articles.length === 0 &&
    (p.daysPending || (p.list.isLoading && !!p.selectedDay));
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
        <View style={s.headingRow}>
          <Text accessibilityRole="header" style={s.title}>
            Intelligence bulletin
          </Text>
        </View>
        <View style={[s.search, wide && { flex: 1, maxWidth: 512 }]}>
          <View style={s.searchField}>
            <TextInput
              accessibilityLabel="Search articles"
              placeholder="Search articles"
              placeholderTextColor={c.dim}
              value={p.search}
              onChangeText={p.onSearch}
              style={[s.input, width < 600 && { fontSize: 17 }]}
            />
          </View>
        </View>
      </View>
      <View style={[s.body, wide && { flexDirection: "row" }]}>
        <View style={[s.rail, wide && { width: 220, borderRightWidth: 1 }]}>
          <Text style={s.micro}>Categories</Text>
          <CategoryRail
            categories={p.categories}
            category={p.category}
            counts={p.counts}
            onCategory={p.onCategory}
            wide={wide}
          />
        </View>
        <View
          nativeID="intel-article-feed"
          style={[s.feed, !wide && { padding: 16 }]}
        >
          <FeedHeader
            days={p.days}
            selectedDay={p.selectedDay}
            onDay={p.onDay}
            onRefresh={() => {
              void p.list.refetch();
            }}
            refreshing={p.list.isFetching}
          />
          <ScrollView
            nativeID="intel-article-scroll"
            testID="article-list-scroll"
            style={s.list}
            contentContainerStyle={s.listContent}
            scrollEventThrottle={100}
            onScroll={({
              nativeEvent: { contentOffset, contentSize, layoutMeasurement },
            }) => {
              if (
                !p.list.hasNextPage ||
                p.list.isFetching ||
                p.list.isFetchNextPageError
              )
                return;
              if (
                contentOffset.y + layoutMeasurement.height >=
                contentSize.height - 240
              )
                void p.list.fetchNextPage({ cancelRefetch: false });
            }}
          >
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
            {loading ? <BulletinSkeleton /> : null}
            {p.daysSuccess && !p.selectedDay ? (
              <EmptyState title="No articles yet">
                Collect an Atlas cycle in Argos to populate this bulletin.
              </EmptyState>
            ) : null}
            {p.list.data && !!p.selectedDay && p.articles.length === 0 ? (
              <View style={{ gap: 16 }}>
                <EmptyState title="NO MATCHING INTELLIGENCE">
                  No articles match this day, category and search.
                </EmptyState>
                <DeskButton onPress={p.onReset}>Reset filters</DeskButton>
              </View>
            ) : null}
            <View nativeID="intel-feed-cards" style={s.cards}>
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
              <LoadingState label="Loading more articles…" compact />
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
          </ScrollView>
        </View>
      </View>
    </View>
  );
}

const s = StyleSheet.create({
  shell: {
    flex: 1,
    minHeight: 0,
    width: "100%",
    borderRadius: intelRadius.shell,
    borderWidth: 1,
    borderColor: c.border,
    backgroundColor: c.panel,
    overflow: "hidden",
  },
  body: { flex: 1, minHeight: 0 },
  header: {
    paddingHorizontal: 20,
    paddingVertical: 16,
    borderBottomWidth: 1,
    borderColor: c.border,
    gap: 16,
    justifyContent: "space-between",
    flexShrink: 0,
  },
  headingRow: {
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    gap: 12,
    minWidth: 0,
    flexShrink: 1,
  },
  title: {
    color: c.text,
    fontFamily: intelFonts.body,
    fontSize: 21,
    lineHeight: 27,
    fontWeight: "700",
    flexShrink: 1,
  },
  search: { gap: 6, minWidth: 0 },
  searchField: { minWidth: 0, width: "100%" },
  micro: {
    color: c.dim,
    fontFamily: intelFonts.mono,
    fontSize: 13,
    fontWeight: "600",
  },
  input: {
    minHeight: 44,
    backgroundColor: c.inset,
    borderWidth: 1,
    borderColor: c.borderStrong,
    borderRadius: intelRadius.control,
    paddingLeft: 14,
    paddingRight: 14,
    width: "100%",
    fontFamily: intelFonts.body,
    color: c.text,
    fontSize: 16,
    minWidth: 0,
  },
  rail: { gap: 12, padding: 12, borderColor: c.border, flexShrink: 0 },
  feed: {
    flex: 1,
    minWidth: 0,
    minHeight: 0,
    padding: 16,
    gap: 12,
    backgroundColor: c.inset,
    overflow: "visible",
  },
  list: { flex: 1, minHeight: 0 },
  listContent: { gap: 12, paddingBottom: 16 },
  cards: { gap: 12 },
  skeletons: { gap: 12 },
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
