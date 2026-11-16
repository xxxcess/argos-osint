"use client";
import React from "react";
import { ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
import { Notice, LoadingState, EmptyState } from "@argos/ui";
import {
  ApiError,
  articleFromHit,
  cleanArticleSearchFilters,
  INTEL_DOMAIN_NAMES,
  type Article,
  type ArticleSearchFilters,
  type StrategicDomain,
  type useArticleSearch,
} from "@argos/api-client";
import { ArticleCard } from "./ArticleCard";
import { DeskButton } from "../desk-ui";
import { intelColors as c } from "../../shell/intel-theme";
import { intelFonts } from "../../shell/intel-fonts";

/** Indexed title and description hits for one submitted query. */
export function SearchResults({
  query,
  search,
  onArticle,
  onClear,
  filters,
  onFilters,
  day,
  category,
}: {
  query: string;
  search: ReturnType<typeof useArticleSearch>;
  onArticle: (article: Article) => void;
  onClear: () => void;
  /** Filters now applied to the search. */
  filters: ArticleSearchFilters;
  onFilters: (filters: ArticleSearchFilters) => void;
  /** The bulletin's selected retained day and category, offered as filters. */
  day: string;
  category: string;
}) {
  const [sourceDraft, setSourceDraft] = React.useState(filters.source ?? "");
  const sourceProblem =
    sourceDraft.trim() &&
    cleanArticleSearchFilters({ source: sourceDraft }).source === undefined
      ? "Use a domain such as example.test."
      : null;
  const categoryName =
    INTEL_DOMAIN_NAMES[category as StrategicDomain] ?? category;
  const seen = new Set<string>();
  const hits = (search.data?.pages.flatMap((page) => page.hits) || []).filter(
    (hit) => {
      const key = `${hit.run_id}/${hit.article_id}`;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    },
  );
  const error = search.isError && !search.isFetchNextPageError;
  const unavailable =
    error &&
    search.error instanceof ApiError &&
    [404, 405, 501, 503].includes(search.error.status);
  return (
    <ScrollView
      nativeID="intel-search-results"
      style={s.list}
      contentContainerStyle={s.content}
    >
      <View style={s.bar}>
        <Text accessibilityRole="header" style={s.heading}>
          Results for “{query}”
        </Text>
        <DeskButton onPress={onClear}>Clear search</DeskButton>
      </View>
      <View nativeID="intel-search-filters" style={s.filters}>
        <Text style={s.filterLabel}>Narrow by</Text>
        {day ? (
          <DeskButton
            active={filters.day === day}
            label={`Only ${day}`}
            onPress={() =>
              onFilters({ ...filters, day: filters.day === day ? "" : day })
            }
          >
            {`Only ${day}`}
          </DeskButton>
        ) : null}
        {category ? (
          <DeskButton
            active={filters.category === category}
            label={`Only ${categoryName}`}
            onPress={() =>
              onFilters({
                ...filters,
                category: filters.category === category ? "" : category,
              })
            }
          >
            {`Only ${categoryName}`}
          </DeskButton>
        ) : null}
        <TextInput
          accessibilityLabel="Source domain"
          placeholder="Source domain, e.g. example.test"
          placeholderTextColor={c.dim}
          value={sourceDraft}
          onChangeText={setSourceDraft}
          onSubmitEditing={() => {
            if (sourceProblem) return;
            onFilters({ ...filters, source: sourceDraft.trim().toLowerCase() });
          }}
          autoCapitalize="none"
          autoCorrect={false}
          returnKeyType="search"
          style={s.sourceInput}
        />
        {sourceProblem ? (
          <Text accessibilityRole="alert" style={s.hint}>
            {sourceProblem}
          </Text>
        ) : null}
      </View>
      {search.isLoading ? <LoadingState label="Searching…" compact /> : null}
      {unavailable ? (
        <Notice>Article search is unavailable on this backend.</Notice>
      ) : error ? (
        <View style={{ gap: 12 }}>
          <Notice>{search.error?.message || "Search failed."}</Notice>
          <DeskButton
            onPress={() => {
              void search.refetch();
            }}
          >
            Retry search
          </DeskButton>
        </View>
      ) : null}
      {search.isSuccess && hits.length === 0 ? (
        <EmptyState title="NO MATCHING ARTICLES">
          No article title or description matches this search.
        </EmptyState>
      ) : null}
      <View style={s.cards}>
        {hits.map((hit) => (
          <View key={`${hit.run_id}/${hit.article_id}`} style={{ gap: 4 }}>
            <ArticleCard
              article={articleFromHit(hit)}
              onPress={() => onArticle(articleFromHit(hit))}
            />
            <Text style={s.match}>Matched {hit.matched_field}</Text>
          </View>
        ))}
      </View>
      {search.isFetchNextPageError ? (
        <Notice>Could not load more results. {search.error?.message}</Notice>
      ) : null}
      {search.data?.pages.some((page) => page.truncated) ? (
        <Notice>
          Showing the best matches only. Narrow the search to see others.
        </Notice>
      ) : null}
      {search.hasNextPage || search.isFetchNextPageError ? (
        <DeskButton
          disabled={search.isFetching}
          onPress={() => {
            void search.fetchNextPage({ cancelRefetch: false });
          }}
        >
          {search.isFetchingNextPage
            ? "Loading…"
            : search.isFetchNextPageError
              ? "Retry loading results"
              : "Load more results"}
        </DeskButton>
      ) : null}
    </ScrollView>
  );
}

const s = StyleSheet.create({
  list: { flex: 1, minHeight: 0 },
  content: { gap: 16, paddingBottom: 24 },
  bar: {
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
  },
  heading: {
    color: c.text,
    fontFamily: intelFonts.body,
    fontSize: 16,
    fontWeight: "700",
    flexShrink: 1,
  },
  cards: { gap: 12 },
  filters: {
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    gap: 8,
  },
  filterLabel: {
    color: c.dim,
    fontFamily: intelFonts.mono,
    fontSize: 12,
  },
  sourceInput: {
    minWidth: 200,
    flexGrow: 1,
    maxWidth: 320,
    color: c.text,
    fontFamily: intelFonts.body,
    fontSize: 14,
    borderWidth: 1,
    borderColor: c.border,
    borderRadius: 6,
    paddingHorizontal: 10,
    paddingVertical: 6,
  },
  hint: { color: c.dim, fontFamily: intelFonts.body, fontSize: 13 },
  match: {
    color: c.dim,
    fontFamily: intelFonts.mono,
    fontSize: 12,
    paddingHorizontal: 4,
  },
});
