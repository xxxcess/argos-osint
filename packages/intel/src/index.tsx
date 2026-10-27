"use client";
import React, { useEffect, useRef, useState } from "react";
import {
  View,
  Text,
  TextInput,
  FlatList,
  ScrollView,
  Image,
  Linking,
  useWindowDimensions,
  StyleSheet,
} from "react-native";
import {
  useBackend,
  useRead,
  useAction,
  useArticles,
  useRelated,
  useTopology,
  keyFor,
  type Article,
  type Brief,
  type Seed,
} from "@argos/api-client";
import {
  Button,
  Label,
  Copy,
  Panel,
  Notice,
  LoadingState,
  EmptyState,
  fonts,
  glass,
  glow,
  colors as c,
} from "@argos/ui";
const categories = [
  "geopolitical",
  "economic",
  "military",
  "information",
  "stability",
  "technology",
];
export function IntelApp() {
  const b = useBackend();
  const [endpoint, setEndpoint] = useState("http://127.0.0.1:8080");
  const [error, setError] = useState("");
  return (
    <View style={s.shell}>
      <View style={s.masthead}>
        <View style={s.headerInner}>
          <View style={s.row}>
            <Text style={s.brand}>
              ARGOS<Text style={{ color: c.accent }}> / </Text>INTEL
            </Text>
            <Text nativeID="workspace-description" style={s.wordmark}>
              Local intelligence workspace
            </Text>
          </View>
          <Text
            style={{
              color: b.connected ? c.success : c.warning,
              fontFamily: fonts.mono,
              fontSize: 11,
              lineHeight: 18,
            }}
          >
            ● {b.connected ? "API connected" : "Offline · reconnecting"}
          </Text>
        </View>
      </View>
      {!b.connected ? (
        <View style={s.connection}>
          <Label>Connection required</Label>
          {b.connecting ? (
            <LoadingState label="Checking local API…" compact />
          ) : null}
          <Text accessibilityRole="header" style={s.title}>
            Connect to your Intel workspace.
          </Text>
          <Copy>
            The local Argos API must be ready before articles and actions are
            available. Start the API, then retry.
          </Copy>
          <View style={s.field}>
            <Label>API endpoint</Label>
            <TextInput
              accessibilityLabel="API endpoint"
              value={endpoint}
              onChangeText={setEndpoint}
              autoCapitalize="none"
              autoCorrect={false}
              style={s.input}
            />
          </View>
          <Button
            disabled={b.connecting}
            onPress={() => {
              try {
                b.setBase(endpoint);
                b.retry();
                setError("");
              } catch (e) {
                setError(String(e));
              }
            }}
          >
            {b.connecting ? "Connecting…" : "Connect / retry"}
          </Button>
          {error.length > 0 && <Notice>{error}</Notice>}
          <Text style={s.small}>Loopback only · local development</Text>
        </View>
      ) : (
        <Workspace key={`${b.base}/${b.instance}`} />
      )}
    </View>
  );
}
function Workspace() {
  const [category, setCategory] = useState(categories[0]);
  const [day, setDay] = useState("");
  const [search, setSearch] = useState("");
  const [article, setArticle] = useState<Article | null>(null);
  const [screen, setScreen] = useState<"bulletin" | "topology">("bulletin");
  const [seed, setSeed] = useState<Seed | null>(null);
  const days = useRead<{ days: string[] }>("days");
  const selectedDay = days.data?.days.includes(day)
    ? day
    : days.data?.days[0] || "";
  const list = useArticles(category, selectedDay, search);
  const articles = [
    ...new Map(
      (list.data?.pages.flatMap((page) => page.rows) || []).map((article) => [
        keyFor(article),
        article,
      ]),
    ).values(),
  ];
  function loadNextPage() {
    if (!list.hasNextPage || list.isFetching || list.isFetchNextPageError)
      return;
    void list.fetchNextPage({ cancelRefetch: false });
  }
  function topology(next: Seed) {
    setSeed(next);
    setScreen("topology");
  }
  return (
    <>
      <View style={s.nav}>
        <View style={s.navInner}>
          <View style={s.row}>
            <Button
              variant="quiet"
              active={screen === "bulletin"}
              onPress={() => setScreen("bulletin")}
            >
              Bulletin
            </Button>
            <Button
              variant="quiet"
              active={screen === "topology"}
              onPress={() => setScreen("topology")}
            >
              Topology
            </Button>
          </View>
          <Text nativeID="workspace-route" style={s.small}>
            Atlas → Intel → Brain
          </Text>
        </View>
      </View>
      <ScrollView
        testID="workspace-scroll"
        style={{ flex: 1, minHeight: 0 }}
        contentContainerStyle={{ flexGrow: 1 }}
        scrollEventThrottle={100}
        onScroll={({
          nativeEvent: { contentOffset, contentSize, layoutMeasurement },
        }) => {
          if (
            screen === "bulletin" &&
            !article &&
            contentOffset.y + layoutMeasurement.height >=
              contentSize.height - 240
          )
            loadNextPage();
        }}
      >
        {screen === "topology" ? (
          <Topology seed={seed} onSeed={setSeed} />
        ) : article ? (
          <Focus
            key={`${article.run_id}/${article.id}`}
            article={article}
            onBack={() => setArticle(null)}
            onArticle={setArticle}
            onTopology={topology}
          />
        ) : (
          <View style={s.page}>
            <Label>01 / Atlas collection</Label>
            <View
              style={[
                s.row,
                {
                  justifyContent: "space-between",
                  alignItems: "flex-end",
                  flexWrap: "wrap",
                },
              ]}
            >
              <View
                style={{ gap: 8, minWidth: 0, maxWidth: "100%", flexShrink: 1 }}
              >
                <Text accessibilityRole="header" style={s.title}>
                  Intelligence bulletin
                </Text>
                <Copy>Retained articles from Atlas.</Copy>
              </View>
            </View>
            <View style={[s.row, { flexWrap: "wrap", marginTop: 12, gap: 4 }]}>
              {categories.map((cat) => (
                <Button
                  key={cat}
                  variant="quiet"
                  active={category === cat}
                  onPress={() => setCategory(cat)}
                >
                  {cat[0].toUpperCase() + cat.slice(1)}
                </Button>
              ))}
            </View>
            <View style={s.filters}>
              <Label>Search articles</Label>
              <TextInput
                accessibilityLabel="Search articles"
                placeholder="Headlines, sources, descriptions…"
                placeholderTextColor={c.muted}
                value={search}
                onChangeText={setSearch}
                style={s.input}
              />
            </View>
            <FlatList
              horizontal
              data={days.data?.days || []}
              keyExtractor={(day) => day}
              style={{ flexGrow: 0, marginBottom: 8 }}
              contentContainerStyle={{ gap: 8 }}
              renderItem={({ item: d }) => (
                <Button
                  variant="quiet"
                  active={d === selectedDay}
                  label={`Day ${d}`}
                  onPress={() => setDay(d)}
                >
                  {d}
                </Button>
              )}
            />
            {(days.isError || list.isError) && (
              <Notice>{String(days.error || list.error)}</Notice>
            )}
            {days.isPending ? (
              <LoadingState label="Loading retained days…" />
            ) : null}
            {list.isPending && selectedDay ? (
              <LoadingState label="Loading bulletin…" />
            ) : null}
            {days.isSuccess && !selectedDay ? (
              <EmptyState title="No articles yet">
                Collect an Atlas cycle in Argos to populate this bulletin.
              </EmptyState>
            ) : null}
            {list.isSuccess && selectedDay && articles.length === 0 ? (
              <EmptyState title="No matching articles">
                No articles match this day, category and search.
              </EmptyState>
            ) : null}
            <View style={s.listHeading}>
              <Label>Signals / {category}</Label>
              <Text accessibilityLiveRegion="polite" style={s.small}>
                {list.data || (days.isSuccess && days.data.days.length === 0)
                  ? `${articles.length} loaded`
                  : "—"}
              </Text>
            </View>
            {list.isRefetching && !list.isFetchingNextPage ? (
              <LoadingState label="Refreshing bulletin…" compact />
            ) : null}
            <FlatList
              scrollEnabled={false}
              data={articles}
              keyExtractor={(a) => a.run_id + "/" + a.id}
              renderItem={({ item, index }) => (
                <ArticleCard
                  article={item}
                  featured={index === 0}
                  onPress={() => setArticle(item)}
                />
              )}
            />
            {list.isFetchingNextPage ? (
              <LoadingState label="Loading more articles…" />
            ) : null}
            {list.isFetchNextPageError ? (
              <Notice>
                Could not load more articles. {list.error?.message}
              </Notice>
            ) : null}
            {list.hasNextPage && (
              <Button
                disabled={list.isFetchingNextPage}
                onPress={() => {
                  void list.fetchNextPage({ cancelRefetch: false });
                }}
              >
                {list.isFetchingNextPage
                  ? "Loading…"
                  : list.isFetchNextPageError
                    ? "Retry loading articles"
                    : "Load more articles"}
              </Button>
            )}
          </View>
        )}
      </ScrollView>
    </>
  );
}
function ArticleCard({
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
  const thumbnail = safeLink(a.image_url) && failedImage !== a.image_url;
  return (
    <View
      testID={`article-card-${a.id}`}
      style={[s.card, compact && { gap: 12, paddingVertical: 20 }]}
    >
      {thumbnail && (
        <Image
          accessibilityLabel={`Article image: ${a.title}`}
          source={{ uri: a.image_url }}
          resizeMode="cover"
          onError={() => setFailedImage(a.image_url)}
          style={[
            s.thumbnail,
            {
              width: compact ? 72 : featured ? 240 : 160,
              height: compact ? 88 : featured ? 180 : 120,
            },
          ]}
        />
      )}
      <View style={{ flex: 1, minWidth: 0, gap: 10 }}>
        <Label>
          {featured ? "Featured signal · " : ""}
          {a.category} / {a.source_name || a.source_domain}
        </Label>
        <Text
          accessibilityRole="header"
          style={[
            s.headline,
            featured && { fontSize: 23, lineHeight: 31 },
            compact && { fontSize: 18, lineHeight: 26 },
          ]}
        >
          {a.title}
        </Text>
        <Copy>{a.description}</Copy>
        <Text style={s.small}>
          {a.published_at.slice(0, 16)} · {a.author || "Source article"}
        </Text>
        <View style={{ alignSelf: "flex-start" }}>
          <Button variant="quiet" onPress={onPress}>
            Open Focus Brief ↗
          </Button>
        </View>
      </View>
    </View>
  );
}
function uniqueById<T extends { id: string }>(rows: T[]): T[] {
  const seen = new Set<string>();
  return rows.filter((row) => {
    if (seen.has(row.id)) return false;
    seen.add(row.id);
    return true;
  });
}
function Focus({
  article: a,
  onBack,
  onArticle,
  onTopology,
}: {
  article: Article;
  onBack: () => void;
  onArticle: (a: Article) => void;
  onTopology: (s: Seed) => void;
}) {
  const { width } = useWindowDimensions();
  const narrow = width < 850;
  const [tab, setTab] = useState("Reader");
  const detail = useRead<Brief>("articles/detail?" + keyFor(a));
  const related = useRelated(a);
  const ensure = useAction<{ job_id: string }>("brief/ensure");
  const reload = useAction<{ job_id: string }>("brief/reload");
  const backend = useBackend();
  const admitted = useRef("");
  const admit = ensure.mutate;
  useEffect(() => {
    if (!backend.connected) return;
    const key = JSON.stringify([
      backend.base,
      backend.instance,
      a.run_id,
      a.id,
    ]);
    if (admitted.current === key) return;
    admitted.current = key;
    // Admission is idempotent in core; polling and render updates must not
    // repeatedly admit the same occurrence. Reopening can safely reattach.
    admit({ run: a.run_id, id: a.id });
  }, [
    a.run_id,
    a.id,
    backend.base,
    backend.instance,
    backend.connected,
    admit,
  ]);
  const d = detail.data;
  const shown = d?.article || a;
  const extracted = (
    <Panel title="01 / Extracted">
      {detail.isPending ? (
        <LoadingState label="Loading extracted evidence…" />
      ) : !d ? (
        <EmptyState title="Evidence unavailable">
          Retry the brief request to load retained evidence.
        </EmptyState>
      ) : (
        <>
          <Label>Confidence</Label>
          <Copy>
            {d?.confidence?.score != null
              ? `${d.confidence.score}/100 · ${d.confidence.provisional ? "provisional" : "assessed"} evidence score`
              : "No assessed confidence yet"}
          </Copy>
          <Label>Actors</Label>
          {uniqueById(d?.actors ?? []).map((actor) => (
            <Button
              key={actor.id}
              onPress={() =>
                onTopology({
                  RetainedActor: {
                    run_id: a.run_id,
                    article_id: a.id,
                    revision: d?.source_revision || "",
                    actor_ref: actor.id,
                    name: actor.display_name,
                  },
                })
              }
            >
              {actor.display_name} · {actor.type_hint}
            </Button>
          ))}
          {d.actors.length === 0 ? (
            <EmptyState title="No actors extracted">
              No retained actors are available for this article.
            </EmptyState>
          ) : null}
          <Label>Claims</Label>
          {d?.claims
            .filter((claim) => claim.classification !== "inference")
            .map((claim) => (
              <View key={claim.id} style={{ gap: 8 }}>
                <Copy>{claim.text}</Copy>
                <Text style={s.small}>
                  {claim.confidence == null
                    ? "Unassessed"
                    : `${Math.round(claim.confidence * 100)}%`}{" "}
                  · {claim.admiralty} · {claim.classification}
                </Text>
                <Button
                  onPress={() =>
                    onTopology(
                      claim.topology_seed || {
                        RetainedClaim: {
                          run_id: a.run_id,
                          article_id: a.id,
                          revision: d?.source_revision || "",
                          local_id: claim.id,
                          text: claim.text,
                          job_id: d?.job_id || "",
                        },
                      },
                    )
                  }
                >
                  Explore claim
                </Button>
              </View>
            ))}
          {!d?.claims.some((claim) => claim.classification !== "inference") && (
            <EmptyState title="No published claims">
              Extraction has not published claims.
            </EmptyState>
          )}
        </>
      )}
    </Panel>
  );
  const reader = (
    <Panel title="02 / Reader">
      <Text accessibilityRole="header" style={s.readerTitle}>
        {shown.title}
      </Text>
      <Text style={s.small}>
        {shown.source_name} · {shown.published_at} · {shown.author}
      </Text>
      {safeLink(shown.image_url) && (
        <Image
          accessibilityLabel={shown.title}
          source={{ uri: shown.image_url }}
          style={{ height: 240, width: "100%" }}
          resizeMode="cover"
        />
      )}
      <Text style={s.readerCopy}>
        {shown.description || "No description available."}
      </Text>
      {safeLink(shown.url) && (
        <Button
          variant="quiet"
          onPress={() => {
            void Linking.openURL(shown.url);
          }}
        >
          Read original source ↗
        </Button>
      )}
      <View testID="reader-inferences" style={{ gap: 12 }}>
        <Label>Inferences</Label>
        {detail.isPending ? (
          <LoadingState label="Loading inferences…" />
        ) : !d ? (
          <EmptyState title="Inferences unavailable">
            Retry the brief request to load retained inferences.
          </EmptyState>
        ) : (
          <JsonLines
            value={
              d.output?.inferences ||
              d.publication?.[2] ||
              d.claims
                .filter((claim) => claim.classification === "inference")
                .map((claim) => ({ text: claim.text }))
            }
          />
        )}
      </View>
    </Panel>
  );
  const context = (
    <Panel title="03 / Context">
      {detail.isPending ? (
        <LoadingState label="Loading brief status…" />
      ) : d ? (
        <>
          <Label>Brief lifecycle</Label>
          <Copy>{d?.status || "Loading…"}</Copy>
          <Text style={s.small}>
            Extraction: {d?.lifecycle || "unavailable"}
          </Text>
          <Text style={s.small}>
            Publication:{" "}
            {d?.publication ? "Retained canonical brief" : "Not published"}
          </Text>
          <Copy>
            Indexing: {d?.indexing?.state || "Not indexed"}
            {d?.indexing?.reason ? ` · ${d.indexing.reason}` : ""}
          </Copy>
        </>
      ) : (
        <EmptyState title="Brief status unavailable" />
      )}
      <Button
        disabled={ensure.isPending}
        onPress={() => ensure.mutate({ run: a.run_id, id: a.id })}
      >
        {ensure.isPending ? "Admitting…" : "Ensure / resume brief"}
      </Button>
      <Button
        variant="quiet"
        disabled={reload.isPending}
        onPress={() =>
          reload.mutate({
            run: a.run_id,
            id: a.id,
            request_id: backend.createRequestId(),
          })
        }
      >
        Reload article / new generation
      </Button>
      {reload.error && <Notice>{reload.error.message}</Notice>}
      <Button
        variant="quiet"
        onPress={() => {
          void detail.refetch();
          void related.refetch();
        }}
      >
        Refresh retained brief
      </Button>
      <Label>Links</Label>
      {detail.isPending ? (
        <LoadingState label="Loading retained links…" />
      ) : d ? (
        <JsonLines value={d.links} />
      ) : (
        <EmptyState title="Links unavailable" />
      )}
      <Label>Related leads · not corroboration</Label>
      {related.isPending ? (
        <LoadingState label="Loading related leads…" />
      ) : null}
      {related.isRefetching && !related.isFetchingNextPage ? (
        <LoadingState label="Refreshing related leads…" compact />
      ) : null}
      {related.error && <Notice>{related.error.message}</Notice>}
      {related.data?.pages
        .flatMap((p) => p.rows)
        .map((r) => (
          <View key={r.article.run_id + r.article.id} style={{ gap: 8 }}>
            <Button onPress={() => onArticle(r.article)}>
              {r.article.title}
            </Button>
            <Copy>
              {r.basis.join(" · ")}
              {r.tentative ? " · tentative" : ""}
            </Copy>
          </View>
        ))}
      {related.isSuccess &&
      related.data.pages.flatMap((p) => p.rows).length === 0 ? (
        <EmptyState title="No related leads">No retained leads.</EmptyState>
      ) : null}
      {related.isFetchingNextPage ? (
        <LoadingState label="Loading more related leads…" />
      ) : null}
      {related.hasNextPage && (
        <Button
          disabled={related.isFetchingNextPage}
          onPress={() => {
            void related.fetchNextPage();
          }}
        >
          More related leads
        </Button>
      )}
      <Button
        variant="quiet"
        onPress={() =>
          onTopology({
            Article: {
              run_id: a.run_id,
              article_id: a.id,
              revision: d?.source_revision || "",
              originating_memory: null,
            },
          })
        }
      >
        Explore article topology
      </Button>
    </Panel>
  );
  return (
    <View style={s.focusPage}>
      <View style={s.focusHeader}>
        <Button variant="quiet" onPress={onBack}>
          ← Bulletin
        </Button>
        <Label>Focus Brief / {shown.category}</Label>
        {!narrow ? (
          <Text style={s.small}>Evidence before assumption</Text>
        ) : null}
      </View>
      {detail.isError && <Notice>{detail.error.message}</Notice>}
      {ensure.error && (
        <Notice>
          Could not start brief extraction: {ensure.error.message}
        </Notice>
      )}
      {detail.isPending ? (
        <LoadingState label="Loading Focus Brief…" compact />
      ) : null}
      {narrow && (
        <View style={[s.row, { paddingVertical: 12, gap: 4 }]}>
          {["Extracted", "Reader", "Context"].map((t) => (
            <Button
              key={t}
              variant="quiet"
              active={t === tab}
              onPress={() => setTab(t)}
            >
              {t}
            </Button>
          ))}
        </View>
      )}
      <View
        style={{
          flexDirection: narrow ? "column" : "row",
          alignItems: "stretch",
          gap: 1,
          backgroundColor: c.border,
          borderWidth: 1,
          borderColor: c.border,
        }}
      >
        {(!narrow || tab === "Extracted") && (
          <View style={{ flex: 24, minWidth: 0, backgroundColor: c.bg }}>
            {extracted}
          </View>
        )}
        {(!narrow || tab === "Reader") && (
          <View style={{ flex: 48, minWidth: 0, backgroundColor: c.bg }}>
            {reader}
          </View>
        )}
        {(!narrow || tab === "Context") && (
          <View style={{ flex: 28, minWidth: 0, backgroundColor: c.bg }}>
            {context}
          </View>
        )}
      </View>
    </View>
  );
}
function Topology({
  seed,
  onSeed,
}: {
  seed: Seed | null;
  onSeed: (s: Seed) => void;
}) {
  const graph = useTopology(seed);
  return (
    <View style={s.page}>
      <Label>Brain / Topology</Label>
      <Text accessibilityRole="header" style={s.title}>
        {graph.data?.root_label || "Evidence topology"}
      </Text>
      <Copy>
        Select an actor, claim or article from a Focus Brief to begin.
        Connections preserve their recorded meaning.
      </Copy>
      {graph.error && <Notice>{graph.error.message}</Notice>}
      {!seed ? (
        <EmptyState title="No topology selected">
          Open an actor, claim or article from a Focus Brief.
        </EmptyState>
      ) : null}
      {seed && graph.isPending ? (
        <LoadingState label="Loading topology…" />
      ) : null}
      {graph.isRefetching ? (
        <LoadingState label="Refreshing topology…" compact />
      ) : null}
      {seed && graph.isSuccess && graph.data.nodes.length === 0 ? (
        <EmptyState title="No retained connections">
          No topology records are available for this seed.
        </EmptyState>
      ) : null}
      <View
        style={[
          s.row,
          { flexWrap: "wrap", alignItems: "stretch", marginTop: 12 },
        ]}
      >
        {graph.data?.nodes.map((n) => (
          <View key={n.id} style={s.topologyNode}>
            <Panel title={n.kind}>
              <Text style={s.headline}>{n.label}</Text>
              <Copy>{n.detail}</Copy>
              {n.topology_seed && (
                <Button onPress={() => onSeed(n.topology_seed!)}>
                  Navigate this node →
                </Button>
              )}
            </Panel>
          </View>
        ))}
      </View>
      {graph.data?.edges.map((e) => (
        <View key={e.id} style={s.card}>
          <Copy>
            {graph.data?.nodes.find((n) => n.id === e.from)?.label || e.from} →{" "}
            {e.label} →{" "}
            {graph.data?.nodes.find((n) => n.id === e.to)?.label || e.to}
          </Copy>
        </View>
      ))}
      {graph.data?.findings.gaps.map((g, i) => (
        <Notice key={i}>{g}</Notice>
      ))}
      {graph.data?.has_continuation && (
        <Copy>
          Showing bounded topology. Navigate a node to explore further.
        </Copy>
      )}
    </View>
  );
}
function JsonLines({ value }: { value: unknown }) {
  function lines(v: unknown): string[] {
    if (Array.isArray(v)) return v.flatMap(lines);
    if (typeof v === "string") return [v];
    if (!v || typeof v !== "object") return [];
    const object = v as Record<string, unknown>;
    const own = [
      "text",
      "claim",
      "inference_text",
      "why",
      "verification_needed",
      "interpretation",
      "empty_reason",
    ].flatMap((key) =>
      typeof object[key] === "string" && object[key]
        ? [String(object[key])]
        : [],
    );
    return own.length
      ? own
      : Object.entries(object)
          .filter(([key]) =>
            ["claims", "inferences", "leads", "links"].includes(key),
          )
          .flatMap(([, v]) => lines(v));
  }
  const items = lines(value);
  return items.length ? (
    <View style={{ gap: 12 }}>
      {items.map((line, i) => (
        <Copy key={i}>{line}</Copy>
      ))}
    </View>
  ) : (
    <EmptyState title="No retained output" />
  );
}

function safeLink(raw: string) {
  try {
    return ["http:", "https:"].includes(new URL(raw).protocol);
  } catch {
    return false;
  }
}
const s = StyleSheet.create({
  shell: { height: "100%", minHeight: 0, backgroundColor: c.bg },
  masthead: {
    ...(glass as object),
    borderColor: c.glassBorder,
    backgroundColor: c.glassStrong,
    borderWidth: 0,
    borderBottomWidth: 1,
  },
  headerInner: {
    width: "100%",
    maxWidth: 1320,
    alignSelf: "center",
    paddingVertical: 20,
    paddingHorizontal: 20,
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
  },
  brand: {
    fontFamily: fonts.mono,
    fontSize: 17,
    lineHeight: 24,
    fontWeight: "700",
    color: c.text,
    letterSpacing: 2,
    textTransform: "uppercase",
  },
  wordmark: {
    fontFamily: fonts.mono,
    fontSize: 11,
    letterSpacing: 1,
    textTransform: "uppercase",
    lineHeight: 18,
    color: c.muted,
  },
  row: {
    flexDirection: "row",
    flexWrap: "wrap",
    flexShrink: 1,
    gap: 12,
    alignItems: "center",
  },
  nav: {
    borderBottomWidth: 1,
    borderColor: c.glassBorder,
    backgroundColor: c.glass,
  },
  navInner: {
    width: "100%",
    maxWidth: 1320,
    alignSelf: "center",
    paddingVertical: 10,
    paddingHorizontal: 20,
    flexDirection: "row",
    flexWrap: "wrap",
    justifyContent: "space-between",
    alignItems: "center",
    gap: 10,
  },
  small: {
    fontFamily: fonts.mono,
    fontSize: 11,
    color: c.muted,
    lineHeight: 19,
    flexShrink: 1,
  },
  page: {
    paddingHorizontal: 20,
    paddingVertical: 32,
    maxWidth: 1160,
    width: "100%",
    alignSelf: "center",
    gap: 16,
  },
  title: {
    fontSize: 27,
    lineHeight: 35,
    fontWeight: "600",
    color: c.text,
    letterSpacing: -0.5,
  },
  headline: { fontSize: 19, lineHeight: 28, fontWeight: "600", color: c.text },
  readerTitle: {
    fontSize: 26,
    lineHeight: 35,
    fontWeight: "600",
    color: c.text,
    letterSpacing: -0.4,
  },
  readerCopy: { fontSize: 16, lineHeight: 29, color: c.text, maxWidth: 640 },
  filters: { gap: 8, paddingTop: 12, borderTopWidth: 1, borderColor: c.glassBorder },
  field: { gap: 8 },
  listHeading: {
    borderBottomWidth: 1,
    borderColor: c.accent + "55",
    paddingBottom: 12,
    flexDirection: "row",
    flexWrap: "wrap",
    justifyContent: "space-between",
    gap: 8,
  },
  card: {
    ...(glass as object),
    borderLeftWidth: 3,
    borderLeftColor: c.accent,
    borderRadius: 2,
    marginBottom: 12,
    paddingVertical: 20,
    paddingHorizontal: 16,
    flexDirection: "row",
    gap: 20,
  },
  thumbnail: { flexShrink: 0, borderRadius: 2, backgroundColor: c.panel, borderWidth: 1, borderColor: c.glassBorder },
  input: {
    minHeight: 44,
    width: "100%",
    minWidth: 0,
    borderWidth: 1,
    borderColor: c.glassBorder,
    borderRadius: 2,
    backgroundColor: c.glassStrong,
    padding: 12,
    fontFamily: fonts.mono,
    color: c.text,
    fontSize: 14,
    lineHeight: 20,
  },
  connection: {
    ...(glass as object),
    ...(glow(c.accent) as object),
    borderTopWidth: 2,
    borderTopColor: c.accent,
    borderRadius: 2,
    maxWidth: 520,
    width: "100%",
    alignSelf: "center",
    padding: 20,
    marginTop: 48,
    gap: 20,
  },
  focusPage: {
    maxWidth: 1440,
    width: "100%",
    alignSelf: "center",
    padding: 16,
    paddingBottom: 40,
    gap: 12,
  },
  focusHeader: {
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
    paddingVertical: 4,
  },
  topologyNode: {
    flexBasis: 280,
    flexGrow: 1,
    minWidth: 0,
    maxWidth: "100%",
    ...(glass as object),
    borderLeftWidth: 3,
    borderLeftColor: c.accent2,
    borderRadius: 2,
  },
});
