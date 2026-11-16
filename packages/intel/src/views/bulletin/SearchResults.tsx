"use client";
import React from "react";
import { ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
import { Notice, LoadingState, EmptyState } from "@argos/ui";
import {
  actorAvailability,
  actorHits,
  actorHitViews,
  actorSearchProblem,
  ApiError,
  articleFromHit,
  cleanArticleSearchFilters,
  INTEL_DOMAIN_NAMES,
  type Article,
  type ArticleSearchFilters,
  type StrategicDomain,
  type useActorSearch,
  type useArticleSearch,
} from "@argos/api-client";
import { ActorHitCard } from "./ActorHitCard";
import { ArticleCard } from "./ArticleCard";
import { DeskButton } from "../desk-ui";
import { intelColors as c } from "../../shell/intel-theme";
import { intelFonts } from "../../shell/intel-fonts";

const UNAVAILABLE_STATUS = [404, 405, 501, 503];

/** Actor discovery hits for the submitted query, ranked and reported apart from articles. */
function ActorResults({
  query,
  actors,
  onActor,
}: {
  query: string;
  actors: ReturnType<typeof useActorSearch>;
  onActor: (actorId: string) => void;
}) {
  const problem = actorSearchProblem(query);
  const availability = actorAvailability(actors.data);
  const views = actorHitViews(actorHits(actors.data?.rows));
  const failed = actors.isError;
  const missingRoute =
    failed &&
    actors.error instanceof ApiError &&
    UNAVAILABLE_STATUS.includes(actors.error.status);
  const state = availability.kind === "ready" ? "" : availability.state;
  return (
    <View nativeID="intel-actor-results" style={s.group}>
      <Text accessibilityRole="header" style={s.groupHeading}>
        Actors
      </Text>
      {problem ? (
        <Notice>Actor search skipped. {problem}</Notice>
      ) : actors.isLoading ? (
        <LoadingState label="Searching actors…" compact />
      ) : missingRoute ? (
        <Notice>Actor search is unavailable on this backend.</Notice>
      ) : failed ? (
        <View style={{ gap: 12 }}>
          <Notice>{actors.error?.message || "Actor search failed."}</Notice>
          <DeskButton
            onPress={() => {
              void actors.refetch();
            }}
          >
            Retry actor search
          </DeskButton>
        </View>
      ) : actors.isSuccess && availability.kind === "unavailable" ? (
        <Notice>
          Actor index is {state.replace("_", " ")}. Actor results are not
          available.
        </Notice>
      ) : actors.isSuccess ? (
        <>
          {availability.kind === "partial" ? (
            <Notice>
              Actor index is {state}. Results may be incomplete or out of date.
            </Notice>
          ) : views.length === 0 ? (
            <EmptyState title="NO MATCHING ACTORS">
              No actor name, alias or description matches this search.
            </EmptyState>
          ) : null}
          <View style={s.cards}>
            {views.map((view) => (
              <ActorHitCard
                key={view.hit.actor_id}
                view={view}
                onPress={() => onActor(view.hit.actor_id)}
              />
            ))}
          </View>
          {actors.data?.truncated ? (
            <Notice>Showing the best actor matches only.</Notice>
          ) : null}
        </>
      ) : null}
    </View>
  );
}

/** Indexed title and description hits, and actor discovery hits, for one submitted query. */
export function SearchResults({
  query,
  search,
  actors,
  onActor,
  onArticle,
  onClear,
  filters,
  onFilters,
  day,
  category,
}: {
  query: string;
  search: ReturnType<typeof useArticleSearch>;
  actors: ReturnType<typeof useActorSearch>;
  /** Opens the dossier for a canonical `actor_id`. */
  onActor: (actorId: string) => void;
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
    UNAVAILABLE_STATUS.includes(search.error.status);
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
      <ActorResults query={query} actors={actors} onActor={onActor} />
      <Text accessibilityRole="header" style={s.groupHeading}>
        Articles
      </Text>
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
  group: { gap: 12 },
  groupHeading: {
    color: c.dim,
    fontFamily: intelFonts.mono,
    fontSize: 13,
    fontWeight: "700",
    letterSpacing: 1,
    textTransform: "uppercase",
  },
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
