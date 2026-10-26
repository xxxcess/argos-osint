"use client";
import React, { useEffect, useRef, useState } from "react";
import {
  View,
  Text,
  TextInput,
  FlatList,
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
import { Button, Label, Copy, Panel, Notice, colors as c } from "@argos/ui";
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
    <View style={{ minHeight: "100%", backgroundColor: c.bg }}>
      <View style={s.masthead}>
        <View style={s.row}>
          <Text style={s.brand}>◉ ARGOS</Text>
          <Text style={s.wordmark}>INTELLIGENCE WORKSPACE</Text>
        </View>
        <Text
          style={{
            color: b.connected ? c.accent : c.danger,
            fontSize: 11,
            letterSpacing: 1,
          }}
        >
          ● {b.connected ? "LOCAL API CONNECTED" : "OFFLINE · RECONNECTING"}
        </Text>
      </View>
      {!b.connected ? (
        <View
          style={{
            maxWidth: 560,
            width: "100%",
            alignSelf: "center",
            padding: 28,
            marginTop: 80,
            gap: 20,
          }}
        >
          <Label>Connection required</Label>
          <Text style={s.title}>Connect to your Intel workspace.</Text>
          <Copy>
            The local Argos API must be ready before articles and actions are
            available. Start the API, then retry.
          </Copy>
          <TextInput
            accessibilityLabel="API endpoint"
            value={endpoint}
            onChangeText={setEndpoint}
            style={s.input}
          />
          <Button
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
            Connect / retry
          </Button>
          {error.length > 0 && <Notice>{error}</Notice>}
          <Copy>Local development · no authentication gate</Copy>
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
  const selectedDay = day || days.data?.days[0] || "";
  const list = useArticles(category, selectedDay, search);
  function topology(next: Seed) {
    setSeed(next);
    setScreen("topology");
  }
  return (
    <>
      <View style={s.nav}>
        <View style={s.row}>
          <Button
            active={screen === "bulletin"}
            onPress={() => setScreen("bulletin")}
          >
            Bulletin
          </Button>
          <Button
            active={screen === "topology"}
            onPress={() => setScreen("topology")}
          >
            Topology
          </Button>
        </View>
        <Text style={s.small}>RETAINED INTELLIGENCE / ALL CYCLES</Text>
      </View>
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
          <Label>Atlas / Intel bulletin</Label>
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
              style={{ gap: 12, minWidth: 0, maxWidth: "100%", flexShrink: 1 }}
            >
              <Text style={s.title}>The world, in focus.</Text>
              <Copy>
                Signals, evidence and the context behind the headline.
              </Copy>
            </View>
            <Text style={s.small}>{selectedDay || "NO RETAINED CYCLES"}</Text>
          </View>
          <View
            style={[
              s.row,
              { flexWrap: "wrap", marginTop: 30, marginBottom: 24 },
            ]}
          >
            {categories.map((cat) => (
              <Button
                key={cat}
                active={category === cat}
                onPress={() => setCategory(cat)}
              >
                {cat[0].toUpperCase() + cat.slice(1)}
              </Button>
            ))}
          </View>
          <View style={[s.row, { flexWrap: "wrap", marginBottom: 28 }]}>
            <TextInput
              accessibilityLabel="Search articles"
              placeholder="Search headlines, sources and descriptions…"
              placeholderTextColor={c.muted}
              value={search}
              onChangeText={setSearch}
              style={[s.input, { flexGrow: 1, minWidth: 220 }]}
            />
          </View>
          <FlatList
            horizontal
            data={days.data?.days || []}
            keyExtractor={(day) => day}
            style={{ marginBottom: 20 }}
            contentContainerStyle={{ gap: 8 }}
            renderItem={({ item: d }) => (
              <Button
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
          {list.isPending && Boolean(selectedDay) && (
            <Copy>Loading bulletin…</Copy>
          )}
          {!days.isPending && !selectedDay && (
            <Panel title="No articles yet">
              <Copy>
                Collect an Atlas cycle in Argos to populate this bulletin.
              </Copy>
            </Panel>
          )}
          {list.data && list.data.pages.flatMap((p) => p.rows).length === 0 && (
            <Panel>
              <Copy>No articles match this day, category and search.</Copy>
            </Panel>
          )}
          <FlatList
            scrollEnabled={false}
            data={list.data?.pages.flatMap((p) => p.rows) || []}
            keyExtractor={(a) => a.run_id + "/" + a.id}
            renderItem={({ item, index }) => (
              <ArticleCard
                article={item}
                featured={index === 0}
                onPress={() => setArticle(item)}
              />
            )}
          />
          {list.hasNextPage && (
            <Button
              disabled={list.isFetchingNextPage}
              onPress={() => {
                void list.fetchNextPage();
              }}
            >
              Load more articles
            </Button>
          )}
        </View>
      )}
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
      style={[
        s.card,
        featured && {
          borderColor: "#456459",
          padding: 28,
          backgroundColor: "#101e20",
        },
        compact && { padding: 14, gap: 12 },
      ]}
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
      <View style={{ flex: 1, minWidth: 0, gap: 12 }}>
        <Label>
          {featured ? "Featured signal · " : ""}
          {a.category} / {a.source_name || a.source_domain}
        </Label>
        <Text
          style={[
            s.headline,
            featured && { fontSize: 28 },
            compact && { fontSize: 20, lineHeight: 27 },
          ]}
        >
          {a.title}
        </Text>
        <Copy>{a.description}</Copy>
        <Text style={s.small}>
          {a.published_at.slice(0, 16)} · {a.author || "Source article"}
        </Text>
        <View style={{ alignSelf: "flex-start" }}>
          <Button onPress={onPress}>Open Focus Brief ↗</Button>
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
        <Copy>Extraction has not published claims.</Copy>
      )}
      <Label>Inferences</Label>
      <JsonLines
        value={
          d?.output?.inferences ||
          d?.publication?.[2] ||
          d?.claims
            .filter((claim) => claim.classification === "inference")
            .map((claim) => ({ text: claim.text }))
        }
      />
    </Panel>
  );
  const reader = (
    <Panel title="02 / Reader">
      <Text style={s.headline}>{shown.title}</Text>
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
      <Text
        style={{
          fontFamily: "Georgia",
          fontSize: 17,
          lineHeight: 31,
          color: c.text,
        }}
      >
        {shown.description || "No description available."}
      </Text>
      {safeLink(shown.url) && (
        <Button
          onPress={() => {
            void Linking.openURL(shown.url);
          }}
        >
          Read original source ↗
        </Button>
      )}
    </Panel>
  );
  const context = (
    <Panel title="03 / Context">
      <Label>Brief lifecycle</Label>
      <Copy>{d?.status || "Loading…"}</Copy>
      <Text style={s.small}>Extraction: {d?.lifecycle || "unavailable"}</Text>
      <Text style={s.small}>
        Publication:{" "}
        {d?.publication ? "Retained canonical brief" : "Not published"}
      </Text>
      <Copy>
        Indexing: {d?.indexing?.state || "Not indexed"}
        {d?.indexing?.reason ? ` · ${d.indexing.reason}` : ""}
      </Copy>
      <Button
        disabled={ensure.isPending}
        onPress={() => ensure.mutate({ run: a.run_id, id: a.id })}
      >
        {ensure.isPending ? "Admitting…" : "Ensure / resume brief"}
      </Button>
      <Button
        disabled={reload.isPending}
        onPress={() =>
          reload.mutate({
            run: a.run_id,
            id: a.id,
            request_id: crypto.randomUUID(),
          })
        }
      >
        Reload article / new generation
      </Button>
      {reload.error && <Notice>{reload.error.message}</Notice>}
      <Button
        onPress={() => {
          void detail.refetch();
          void related.refetch();
        }}
      >
        Refresh retained brief
      </Button>
      <Label>Links</Label>
      <JsonLines value={d?.links} />
      <Label>Related leads · not corroboration</Label>
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
      {related.data?.pages.flatMap((p) => p.rows).length === 0 && (
        <Copy>No retained leads.</Copy>
      )}
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
    <View>
      <View style={[s.nav, { flexWrap: "wrap" }]}>
        <Button onPress={onBack}>← Bulletin</Button>
        <Label>Focus Brief / {shown.category}</Label>
        <Text style={s.small}>EVIDENCE BEFORE ASSUMPTION</Text>
      </View>
      {detail.isError && <Notice>{detail.error.message}</Notice>}
      {ensure.error && (
        <Notice>
          Could not start brief extraction: {ensure.error.message}
        </Notice>
      )}
      {detail.isPending && <Copy>Loading Focus Brief…</Copy>}
      {narrow && (
        <View style={[s.row, { padding: 16 }]}>
          {["Extracted", "Reader", "Context"].map((t) => (
            <Button key={t} active={t === tab} onPress={() => setTab(t)}>
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
        }}
      >
        {(!narrow || tab === "Extracted") && (
          <View style={{ flex: 24, minWidth: 0, backgroundColor: c.panel }}>
            {extracted}
          </View>
        )}
        {(!narrow || tab === "Reader") && (
          <View style={{ flex: 48, minWidth: 0, backgroundColor: c.panel }}>
            {reader}
          </View>
        )}
        {(!narrow || tab === "Context") && (
          <View style={{ flex: 28, minWidth: 0, backgroundColor: c.panel }}>
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
      <Text style={s.title}>
        {graph.data?.root_label || "Follow the evidence."}
      </Text>
      <Copy>
        Select an actor, claim or article from a Focus Brief to begin.
        Connections preserve their recorded meaning.
      </Copy>
      {graph.error && <Notice>{graph.error.message}</Notice>}
      {seed && graph.isPending && <Copy>Loading topology…</Copy>}
      <View
        style={[
          s.row,
          { flexWrap: "wrap", alignItems: "stretch", marginTop: 28 },
        ]}
      >
        {graph.data?.nodes.map((n) => (
          <View key={n.id} style={{ flexBasis: 280, flexGrow: 1 }}>
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
    <Copy>No retained output.</Copy>
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
  masthead: {
    paddingVertical: 22,
    paddingHorizontal: 32,
    borderBottomWidth: 1,
    borderColor: c.border,
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 18,
  },
  brand: { fontSize: 22, fontWeight: "700", letterSpacing: 3, color: c.text },
  wordmark: { fontSize: 9, letterSpacing: 2, color: c.muted },
  row: {
    flexDirection: "row",
    flexWrap: "wrap",
    flexShrink: 1,
    gap: 10,
    alignItems: "center",
  },
  nav: {
    flexWrap: "wrap",
    paddingVertical: 18,
    paddingHorizontal: 32,
    borderBottomWidth: 1,
    borderColor: c.border,
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "center",
    gap: 14,
  },
  small: { fontSize: 10, color: c.muted, letterSpacing: 1, lineHeight: 18 },
  page: {
    padding: 28,
    maxWidth: 1440,
    width: "100%",
    alignSelf: "center",
    gap: 14,
  },
  title: {
    fontSize: 38,
    lineHeight: 46,
    fontWeight: "600",
    color: c.text,
    letterSpacing: -1,
  },
  headline: { fontSize: 22, lineHeight: 30, fontWeight: "600", color: c.text },
  card: {
    borderWidth: 1,
    borderColor: c.border,
    padding: 22,
    marginBottom: 14,
    backgroundColor: c.panel,
    flexDirection: "row",
    gap: 24,
    borderRadius: 5,
  },
  thumbnail: {
    flexShrink: 0,
    borderRadius: 4,
    backgroundColor: c.raised,
  },
  input: {
    borderWidth: 1,
    borderColor: c.border,
    borderRadius: 4,
    backgroundColor: c.panel,
    padding: 12,
    color: c.text,
    fontSize: 14,
  },
});
