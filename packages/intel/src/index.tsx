"use client";
import React, {
  useEffect,
  useCallback,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { View, ScrollView, StyleSheet } from "react-native";
import {
  useBackend,
  useHostMetrics,
  useBriefLive,
  useRead,
  useAction,
  useArticles,
  useRelated,
  keyFor,
  INTEL_CATEGORIES,
  type Article,
  type Brief,
  type Seed,
} from "@argos/api-client";
import {
  Button,
  Label,
  Copy,
  Notice,
  LoadingState,
  EmptyState,
  fonts,
  glass,
  glow,
  colors as c,
  ErrorBoundary,
} from "@argos/ui";
import { OrbitScene } from "./landing/OrbitScene";
import { IntelStatusBar, JobDropUp, type StatusJob } from "./shell/IntelShell";
import { Topology } from "./topology/Topology";
import { HomeView } from "./views/HomeView";
import { BulletinView } from "./views/BulletinView";
import { ContextRail } from "./views/brief/ContextRail";
import { BriefSkeleton } from "./views/brief/BriefSkeleton";
import { deriveBriefJobState, type JobState } from "./shell/jobState";
import { ScreenTransition } from "./shell/ScreenTransition";
import { BriefView, RelatedBriefCard } from "./views/BriefView";
const categories = [...INTEL_CATEGORIES];
import {
  createMemoryNavigation,
  type IntelNavigation,
  type ArticleLocation,
} from "./navigation";
export type { IntelNavigation, IntelRoute } from "./navigation";
export function IntelApp({
  navigation,
}: { navigation?: IntelNavigation; displayScale?: number } = {}) {
  const [fallbackNavigation] = useState(createMemoryNavigation);
  const navigator = navigation ?? fallbackNavigation;
  const route = useSyncExternalStore(
    navigator.subscribe,
    navigator.getSnapshot,
    navigator.getServerSnapshot,
  );
  const b = useBackend();
  const [introComplete, setIntroComplete] = useState(false);
  const completeIntro = useCallback(() => setIntroComplete(true), []);
  if (route.screen !== "home" && !introComplete) setIntroComplete(true);
  const playLanding = route.screen === "home" && !introComplete;
  return (
    <View style={s.shell}>
      <ScreenTransition fill screen={playLanding ? "connection" : "home"}>
        {playLanding ? (
          <OrbitScene
            onComplete={completeIntro}
            ready={b.connected}
            health={b.health}
            connecting={b.connecting}
            error={b.connectionError}
            onRetry={b.retry}
          />
        ) : b.connected ? (
          <ErrorBoundary label="Workspace">
            <Workspace key={`${b.base}/${b.instance}`} navigation={navigator} />
          </ErrorBoundary>
        ) : (
          <LoadingState label="Connecting to local API…" />
        )}
      </ScreenTransition>
      {playLanding || !b.connected ? (
        <WorkspaceStatusBar
          hideConnectionIndicator={playLanding}
          connected={b.connected}
          activeApp={false}
        />
      ) : null}
    </View>
  );
}
function WorkspaceStatusBar(
  props: Omit<React.ComponentProps<typeof IntelStatusBar>, "clock">,
) {
  const metrics = useHostMetrics();
  const host = metrics.isError ? undefined : metrics.data;
  const [clock, setClock] = useState("");
  useEffect(() => {
    const update = () => setClock(new Date().toLocaleTimeString("en-US"));
    const timer = setInterval(update, 1000);
    return () => clearInterval(timer);
  }, []);
  return (
    <IntelStatusBar
      {...props}
      clock={clock}
      metrics={
        props.connected && host
          ? {
              cpuPercent: host.cpu_percent,
              memoryPercent: host.memory_percent,
              diskPercent: host.disk_percent,
              networkKBps: host.network_kbps,
            }
          : undefined
      }
    />
  );
}
function Workspace({ navigation }: { navigation: IntelNavigation }) {
  const route = useSyncExternalStore(
    navigation.subscribe,
    navigation.getSnapshot,
    navigation.getServerSnapshot,
  );
  const [category, setCategory] = useState<string>(INTEL_CATEGORIES[0]);
  const [day, setDay] = useState("");
  const [search, setSearch] = useState("");
  const [debouncedSearch, setDebouncedSearch] = useState("");
  useEffect(() => {
    const timer = setTimeout(() => {
      setDebouncedSearch(search);
    }, 300);
    return () => clearTimeout(timer);
  }, [search]);
  const [retainedArticle, setArticle] = useState<Article | null>(null);
  const location: Partial<ArticleLocation> | undefined =
    route.screen === "brief"
      ? route
      : route.screen === "topology"
        ? route.from
        : undefined;
  const matches =
    retainedArticle &&
    retainedArticle.run_id === location?.run_id &&
    retainedArticle.id === location?.article_id;
  const directBrief = useRead<Brief>(
    `articles/detail?${keyFor({ run_id: location?.run_id || "", id: location?.article_id || "" })}`,
    route.screen === "brief" &&
      !!location?.run_id &&
      !!location.article_id &&
      !matches,
  );
  const article = matches ? retainedArticle : directBrief.data?.article;
  const lastArticle = retainedArticle ?? directBrief.data?.article;
  const screen = route.screen;
  const seed = route.screen === "topology" ? route.seed : null;
  const navigate = navigation.navigate;
  const resolveTopology = useAction<{ seed: string }>("topology/resolve");
  function openArticle(next: Article) {
    setArticle(next);
    navigate({ screen: "brief", run_id: next.run_id, article_id: next.id });
  }
  const days = useRead<{ days: string[] }>("days", screen === "bulletin");
  const selectedDay = days.data?.days.includes(day)
    ? day
    : days.data?.days[0] || "";
  const counts = useRead<{ counts: Record<string, number> }>(
    `articles/counts?day=${encodeURIComponent(selectedDay)}`,
    screen === "bulletin" && !!selectedDay,
  );
  const list = useArticles(
    category,
    selectedDay,
    debouncedSearch,
    screen === "bulletin",
  );
  const articles = [
    ...new Map(
      (list.data?.pages.flatMap((page) => page.rows) || []).map((article) => [
        keyFor(article),
        article,
      ]),
    ).values(),
  ];
  function topology(next: Seed) {
    const direct = [next.Actor, next.Claim, next.Inference, next.Brief].find(
      (value): value is string => typeof value === "string" && !!value,
    );
    const from = article
      ? { run_id: article.run_id, article_id: article.id }
      : route.screen === "topology"
        ? route.from
        : undefined;
    const open = (seed: string) => {
      navigate({ screen: "topology", seed, ...(from ? { from } : {}) });
    };
    if (direct) {
      resolveTopology.reset();
      open(direct);
      return;
    }
    resolveTopology.mutate(
      { seed: next },
      { onSuccess: (result) => open(result.seed) },
    );
  }
  return (
    <>
      <View nativeID="intel-content-frame" style={s.contentFrame}>
        {resolveTopology.isPending ? (
          <LoadingState label="Finding retained Brain memory…" compact />
        ) : null}
        {resolveTopology.error ? (
          <Notice>{resolveTopology.error.message}</Notice>
        ) : null}
        <ScreenTransition fill screen={screen}>
          {screen === "home" ? (
            <HomeView onOpen={() => navigate({ screen: "bulletin" })} />
          ) : screen === "topology" ? (
            <View
              nativeID="intel-topology-workspace"
              style={{ minWidth: 0, width: "100%", maxWidth: "100%" }}
            >
              <Button
                onPress={() =>
                  navigate(
                    route.screen === "topology" && route.from
                      ? { screen: "brief", ...route.from }
                      : { screen: "bulletin" },
                  )
                }
              >
                {route.screen === "topology" && route.from
                  ? "Return to Brief"
                  : "Return to Bulletin"}
              </Button>
              <Topology seed={seed} onSeed={topology} pageStyle={s.page} />
            </View>
          ) : screen === "brief" && article ? (
            <ScrollView
              style={{ flex: 1, minHeight: 0 }}
              contentContainerStyle={{ flexGrow: 1 }}
            >
              <Focus
                key={`${article.run_id}/${article.id}`}
                article={article}
                onBack={() => navigate({ screen: "bulletin" })}
                onArticle={openArticle}
                onTopology={topology}
              />
            </ScrollView>
          ) : screen === "brief" ? (
            <View style={s.page}>
              <Button onPress={() => navigate({ screen: "bulletin" })}>
                Return to Bulletin
              </Button>
              {directBrief.error ? (
                <>
                  <Notice>{directBrief.error.message}</Notice>
                  <Button
                    onPress={() => {
                      void directBrief.refetch();
                    }}
                  >
                    Retry brief
                  </Button>
                </>
              ) : !location?.run_id || !location.article_id ? (
                <EmptyState title="No article selected">
                  Open an article from the Bulletin.
                </EmptyState>
              ) : (
                <BriefSkeleton />
              )}
            </View>
          ) : (
            <BulletinView
              categories={categories}
              category={category}
              onCategory={setCategory}
              search={search}
              onSearch={setSearch}
              days={days.data?.days || []}
              selectedDay={selectedDay}
              onDay={setDay}
              daysPending={days.isPending}
              daysSuccess={days.isSuccess}
              daysError={days.error}
              onRetryDays={() => {
                void days.refetch();
              }}
              articles={articles}
              counts={counts.data?.counts}
              list={list}
              onArticle={openArticle}
              onHome={() => navigate({ screen: "home" })}
              onReset={() => {
                setCategory(INTEL_CATEGORIES[0]);
                setDay("");
                setSearch("");
              }}
            />
          )}
        </ScreenTransition>
      </View>
      <WorkspaceStatusBar
        connected
        job={
          screen === "home" ? undefined : (
            <PageJobs
              screen={screen}
              article={screen === "brief" ? article : lastArticle}
              daysPending={days.isPending}
              daysError={days.error?.message}
              feedFetching={list.isFetching}
              feedError={
                list.isError && !list.isFetchNextPageError
                  ? list.error?.message
                  : list.isFetchNextPageError
                    ? list.error?.message
                    : undefined
              }
              resolving={resolveTopology.isPending}
              resolveError={resolveTopology.error?.message}
              seed={seed}
            />
          )
        }
        activeApp={screen !== "home"}
        onHome={() => navigate({ screen: "home" })}
        onIntel={() => navigate({ screen: "bulletin" })}
      />
    </>
  );
}

function stageJob(state: string, active: boolean): JobState {
  if (/fail|error|blocked/i.test(state)) return "failed";
  if (active || /^(running|extracting|processing)$/i.test(state))
    return "running";
  if (/queued|pending|waiting/i.test(state)) return "queued";
  if (/completed|succeeded|done|indexed/i.test(state)) return "succeeded";
  return "idle";
}

function PageJobs({
  screen,
  article,
  daysPending,
  daysError,
  feedFetching,
  feedError,
  resolving,
  resolveError,
  seed,
}: {
  screen: "bulletin" | "brief" | "topology" | "home";
  article?: Article | null;
  daysPending: boolean;
  daysError?: string;
  feedFetching: boolean;
  feedError?: string;
  resolving: boolean;
  resolveError?: string;
  seed: string | null;
}) {
  const local: StatusJob[] =
    screen === "bulletin"
      ? [
          {
            id: "days",
            label: "Retained days",
            state: daysError ? "failed" : daysPending ? "running" : "succeeded",
            detail: daysError ?? (daysPending ? "Loading" : "Ready"),
          },
          {
            id: "feed",
            label: "Article feed",
            state: feedError
              ? "failed"
              : feedFetching
                ? "running"
                : "succeeded",
            detail: feedError ?? (feedFetching ? "Loading" : "Ready"),
          },
        ]
      : screen === "topology"
        ? [
            {
              id: "topology",
              label: "Topology",
              state: resolveError
                ? "failed"
                : resolving
                  ? "running"
                  : seed
                    ? "succeeded"
                    : "idle",
              detail:
                resolveError ??
                (resolving ? "Finding memory" : seed ? "Open" : "No selection"),
            },
          ]
        : [];
  if (!article) return <JobDropUp jobs={local} />;
  return (
    <PageJobsWithBrief
      article={article}
      local={local}
      includeBrief={screen === "brief" || screen === "bulletin"}
    />
  );
}

function PageJobsWithBrief({
  article,
  local,
  includeBrief,
}: {
  article: Article;
  local: StatusJob[];
  includeBrief: boolean;
}) {
  const detail = useRead<Brief>(
    "articles/detail?" + keyFor(article),
    includeBrief,
  );
  const live = useBriefLive(article, detail.data?.job_id ?? "");
  const progress = live?.progress;
  const brief: StatusJob[] = !includeBrief
    ? []
    : progress?.stages.length
      ? [
          ...progress.stages.map((stage) => ({
            id: stage.operation,
            label: stage.label,
            detail: stage.detail || stage.row || stage.state,
            state: stageJob(stage.state, stage.active),
          })),
          {
            id: "indexing",
            label: "Indexing",
            detail:
              progress.indexing?.reason ||
              progress.indexing?.state ||
              "Not indexed",
            state:
              progress.indexing?.state === "indexed"
                ? "succeeded"
                : progress.indexing?.state === "failed"
                  ? "failed"
                  : progress.indexing?.state
                    ? "running"
                    : "idle",
          },
        ]
      : [
          {
            id: "brief",
            label: "Brief",
            detail: detail.data?.status || article.title,
            state: deriveBriefJobState({ live, detail: detail.data }),
          },
        ];
  return <JobDropUp jobs={[...local, ...brief]} />;
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
  const detail = useRead<Brief>("articles/detail?" + keyFor(a), true);
  const related = useRelated(a);
  const ensure = useAction<{ job_id: string }>("brief/ensure");
  const reload = useAction<{ job_id: string }>("brief/reload");
  const backend = useBackend();
  const live = useBriefLive(
    a,
    `${ensure.submittedAt}:${ensure.status}:${reload.submittedAt}:${reload.status}:${detail.data?.job_id || ""}`,
  );
  const admitted = useRef("");
  const admit = ensure.mutate;
  const alreadyRan = Boolean(detail.data?.job_id || detail.data?.publication);
  useEffect(() => {
    if (!backend.connected || !detail.isSuccess || alreadyRan) return;
    const key = JSON.stringify([
      backend.base,
      backend.instance,
      a.run_id,
      a.id,
    ]);
    if (admitted.current === key) return;
    admitted.current = key;
    // A brief with no retained run is admitted once. Revisiting a completed
    // run does not queue another chain; Reload does that explicitly.
    admit({ run: a.run_id, id: a.id });
  }, [
    a.run_id,
    a.id,
    alreadyRan,
    backend.base,
    backend.instance,
    backend.connected,
    detail.isSuccess,
    admit,
  ]);
  const d = detail.data;
  const shown = d?.article || a;
  const jobState = deriveBriefJobState({ ensure, reload, live, detail: d });
  const relatedContent = (
    <View style={{ gap: 16, minWidth: 0 }}>
      {related.isPending ? (
        <BriefSkeleton label="Loading related leads…" />
      ) : null}
      {related.isRefetching && !related.isFetchingNextPage ? (
        <LoadingState label="Refreshing related leads…" compact />
      ) : null}
      {related.error && <Notice>{related.error.message}</Notice>}
      <View
        style={{ flexDirection: "row", flexWrap: "wrap", gap: 24, minWidth: 0 }}
      >
        {related.data?.pages
          .flatMap((p) => p.rows)
          .filter(
            (row, index, rows) =>
              rows.findIndex(
                (other) =>
                  other.article.run_id === row.article.run_id &&
                  other.article.id === row.article.id,
              ) === index,
          )
          .map((r) => (
            <RelatedBriefCard
              key={JSON.stringify([r.article.run_id, r.article.id])}
              article={r.article}
              basis={r.basis}
              tentative={r.tentative}
              onPress={() => onArticle(r.article)}
            />
          ))}
      </View>
      {related.isSuccess &&
      related.data.pages.flatMap((p) => p.rows).length === 0 ? (
        <EmptyState title="No related leads">No retained leads.</EmptyState>
      ) : null}
      {related.isFetchingNextPage ? (
        <BriefSkeleton label="Loading more related leads…" />
      ) : null}
      {related.hasNextPage && (
        <Button
          disabled={related.isFetchingNextPage}
          onPress={() => {
            void related.fetchNextPage();
          }}
        >
          {related.isFetchNextPageError
            ? "Retry loading related leads"
            : "More related leads"}
        </Button>
      )}
      {related.isError && !related.isFetchNextPageError ? (
        <Button
          onPress={() => {
            void related.refetch();
          }}
        >
          Retry related leads
        </Button>
      ) : null}
    </View>
  );
  return (
    <BriefView
      article={a}
      brief={d}
      pending={detail.isPending}
      onBack={onBack}
      onTopology={onTopology}
      jobState={jobState}
      source={null}
      related={relatedContent}
      operations={
        <ContextRail
          a={a}
          d={d}
          shown={shown}
          detail={detail}
          related={related}
          ensure={ensure}
          reload={reload}
          backend={backend}
          jobState={jobState}
        />
      }
      notices={
        <>
          {live?.progress && !live.progress.complete ? (
            <View role="status" aria-live="polite" style={{ gap: 8 }}>
              <Label>Brief in progress</Label>
              {live.progress.stages.map((stage) => (
                <Copy key={stage.operation}>
                  {stage.label} · {stage.row}
                </Copy>
              ))}
              {!live.connected ? (
                <Notice>Live updates interrupted. Reconnecting…</Notice>
              ) : null}
            </View>
          ) : null}
          {detail.isError && (
            <>
              <Notice>{detail.error.message}</Notice>
              <Button
                onPress={() => {
                  void detail.refetch();
                }}
              >
                Retry brief
              </Button>
            </>
          )}
          {ensure.error && (
            <Notice>
              Could not start brief extraction: {ensure.error.message}
            </Notice>
          )}
          {detail.isPending ? <BriefSkeleton /> : null}
        </>
      }
    />
  );
}
const s = StyleSheet.create({
  desktop: {
    flex: 1,
    minHeight: 420,
    alignItems: "center",
    justifyContent: "center",
    padding: 24,
    gap: 24,
  },
  launcher: {
    ...(glass as object),
    borderWidth: 1,
    borderColor: c.accent,
    padding: 24,
    gap: 20,
    maxWidth: 440,
    width: "100%",
  },
  bulletinLayout: {
    width: "100%",
    maxWidth: 1320,
    alignSelf: "center",
    padding: 20,
    gap: 24,
  },
  categoryRail: {
    gap: 12,
    padding: 16,
    backgroundColor: c.glass,
    borderWidth: 1,
    borderColor: c.glassBorder,
  },
  bulletinContent: { flex: 1, minWidth: 0, gap: 16 },
  shell: { height: "100%", minHeight: 0, position: "relative" },
  contentFrame: {
    flex: 1,
    minHeight: 0,
    width: "100%",
    maxWidth: 1440,
    alignSelf: "center",
    paddingTop: 32,
    paddingBottom: 32,
    paddingHorizontal: 16,
  },
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
    borderTopWidth: 1,
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
  filters: {
    gap: 8,
    paddingTop: 12,
    borderTopWidth: 1,
    borderColor: c.glassBorder,
  },
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
  thumbnail: {
    flexShrink: 0,
    borderRadius: 2,
    backgroundColor: c.panel,
    borderWidth: 1,
    borderColor: c.glassBorder,
  },
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
  tagRow: { flexDirection: "row", flexWrap: "wrap", gap: 6 },
  telemetry: { textTransform: "uppercase", letterSpacing: 1 },
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
  lead: {
    ...(glass as object),
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    minHeight: 40,
    borderRadius: 2,
    paddingVertical: 10,
    paddingHorizontal: 12,
    minWidth: 0,
    borderWidth: 1,
    borderColor: c.glassBorder,
  },
  leadHover: {
    backgroundColor: c.accentSoft,
    borderColor: c.accent,
  },
  leadTitle: {
    fontFamily: fonts.body,
    fontSize: 14,
    lineHeight: 22,
    color: c.text,
    fontWeight: "600",
  },
  leadBasis: {
    fontFamily: fonts.mono,
    fontSize: 11,
    lineHeight: 18,
    color: c.muted,
  },
  leadChevron: {
    fontFamily: fonts.mono,
    fontSize: 16,
    color: c.accent,
    alignSelf: "center",
    marginLeft: "auto",
  },
});
