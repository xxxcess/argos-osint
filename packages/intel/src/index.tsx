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
  useActorDossier,
  useAssociationLive,
  ApiError,
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
  type BriefStage,
  type Seed,
} from "@argos/api-client";
import {
  Button,
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
import { ActorDossierView } from "./views/actor/ActorDossierView";
import { Topology } from "./topology/Topology";
import { HomeView } from "./views/HomeView";
import { BulletinView } from "./views/BulletinView";
import { BriefProgressStepper } from "./views/brief/BriefProgressStepper";
import { ContextRail } from "./views/brief/ContextRail";
import { BriefSkeleton } from "./views/brief/BriefSkeleton";
import {
  deriveBriefJobState,
  indexingJobState,
  type JobState,
} from "./shell/jobState";
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
          <View style={{ gap: 12, padding: 24 }}>
            {b.connecting ? (
              <LoadingState label="Connecting to local API…" />
            ) : (
              <Notice>{b.connectionError || "Local API unavailable"}</Notice>
            )}
            <Button disabled={b.connecting} onPress={b.retry}>
              Connect / retry
            </Button>
          </View>
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
              platform: host.platform,
              chipset: host.chipset,
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
  const focus = route.screen === "topology" ? route.focus : undefined;
  const actorSource = route.screen === "actor" ? route.source : undefined;
  const actorBrief = useRead<Brief>(
    `articles/detail?${keyFor({ run_id: actorSource?.run_id || "", id: actorSource?.article_id || "" })}`,
    !!actorSource,
  );
  const extractedActor =
    actorSource && !actorBrief.isPlaceholderData
      ? actorBrief.data?.actors.find(
          (actor) => actor.id === actorSource.actor_id,
        )
      : undefined;
  const resolvedActorId =
    route.screen === "actor"
      ? route.id ||
        (typeof extractedActor?.topology_seed?.Actor === "string"
          ? extractedActor.topology_seed.Actor
          : null)
      : null;
  // A retained source actor ID may already identify a catalog row even when
  // article graph decoration has not produced a seed. The read endpoint alone
  // decides whether it is an active canonical actor; never infer it from a name.
  const dossierLookupId = resolvedActorId || extractedActor?.id || null;
  useAssociationLive(
    route.screen === "actor"
      ? route.from || route.source
      : route.screen === "topology"
        ? route.from || route.actorFrom?.from || route.actorFrom?.source
        : undefined,
  );
  const dossier = useActorDossier(dossierLookupId);
  const sourcePreview =
    !!actorSource &&
    !resolvedActorId &&
    dossier.error instanceof ApiError &&
    dossier.error.status === 404;
  const navigate = navigation.navigate;
  function openActor(actor: Brief["actors"][number]) {
    const id =
      typeof actor.topology_seed?.Actor === "string"
        ? actor.topology_seed.Actor
        : null;
    navigate({
      screen: "actor",
      id,
      ...(!id && article
        ? {
            source: {
              run_id: article.run_id,
              article_id: article.id,
              actor_id: actor.id,
            },
          }
        : {}),
      ...(article
        ? { from: { run_id: article.run_id, article_id: article.id } }
        : {}),
    });
  }
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
  const [feedSort, setFeedSort] = useState<"latest" | "hottest">("latest");
  const list = useArticles(
    category,
    selectedDay,
    debouncedSearch,
    screen === "bulletin",
    feedSort,
  );
  const articles = [
    ...new Map(
      (list.data?.pages.flatMap((page) => page.rows) || []).map((article) => [
        keyFor(article),
        article,
      ]),
    ).values(),
  ];
  function topology(next: Seed, focus?: string) {
    const direct = [next.Actor, next.Claim, next.Inference, next.Brief].find(
      (value): value is string => typeof value === "string" && !!value,
    );
    const from =
      route.screen === "actor"
        ? route.from
        : article
          ? { run_id: article.run_id, article_id: article.id }
          : route.screen === "topology"
            ? route.from
            : undefined;
    const open = (seed: string) => {
      navigate({
        screen: "topology",
        seed,
        ...(focus && seed.startsWith("brief:") ? { focus } : {}),
        ...(from ? { from } : {}),
        ...(route.screen === "actor"
          ? {
              actorFrom: {
                id: route.id,
                source: route.source,
                from: route.from,
              },
            }
          : route.screen === "topology" && route.actorFrom
            ? { actorFrom: route.actorFrom }
            : {}),
      });
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
              style={{
                flex: 1,
                minHeight: 0,
                minWidth: 0,
                width: "100%",
                maxWidth: "100%",
              }}
            >
              <View
                style={{
                  flexDirection: "row",
                  flexWrap: "wrap",
                  alignItems: "center",
                  justifyContent: "space-between",
                  gap: 8,
                }}
              >
                <Button
                  onPress={() =>
                    navigate(
                      route.screen === "topology" && route.actorFrom
                        ? { screen: "actor", ...route.actorFrom }
                        : route.screen === "topology" && route.from
                          ? { screen: "brief", ...route.from }
                          : { screen: "bulletin" },
                    )
                  }
                >
                  {route.screen === "topology" && route.actorFrom
                    ? "Return to Actor"
                    : route.screen === "topology" && route.from
                      ? "Return to Brief"
                      : "Return to Bulletin"}
                </Button>
                <View
                  style={{ flexDirection: "row", flexWrap: "wrap", gap: 8 }}
                >
                  {(
                    [
                      ["Neighbors", "intel-topology-neighborhood"],
                      ["Nodes", "intel-topology-nodes"],
                      ["Connections", "intel-topology-connections"],
                    ] as const
                  ).map(([label, id]) => (
                    <Button
                      key={id}
                      onPress={() =>
                        document.getElementById(id)?.scrollIntoView({
                          behavior: "smooth",
                          block: "start",
                        })
                      }
                    >
                      {label}
                    </Button>
                  ))}
                </View>
              </View>
              <ScrollView
                nativeID="intel-topology-content-scroll"
                style={{ flex: 1, minHeight: 0 }}
                contentContainerStyle={{ flexGrow: 1 }}
              >
                <Topology
                  seed={seed}
                  focus={focus}
                  onSeed={topology}
                  pageStyle={s.page}
                />
              </ScrollView>
            </View>
          ) : route.screen === "actor" ? (
            <ScrollView
              nativeID="intel-actor-scroll"
              style={{ flex: 1, minHeight: 0 }}
              contentContainerStyle={{ flexGrow: 1 }}
            >
              <ActorDossierView
                dossier={dossier.data}
                extractedActor={sourcePreview ? extractedActor : undefined}
                sourceArticle={actorBrief.data?.article}
                pending={
                  dossier.isLoading ||
                  actorBrief.isLoading ||
                  actorBrief.isPlaceholderData
                }
                refreshing={dossier.isFetching && !!dossier.data}
                error={dossier.error?.message || actorBrief.error?.message}
                missing={
                  (!route.id && !route.source) ||
                  (!!route.source && actorBrief.isSuccess && !extractedActor) ||
                  (dossier.error instanceof ApiError &&
                    dossier.error.status === 404)
                }
                fromBrief={!!route.from}
                onBack={() =>
                  navigate(
                    route.from
                      ? { screen: "brief", ...route.from }
                      : { screen: "bulletin" },
                  )
                }
                onRetry={() => {
                  if (dossierLookupId) void dossier.refetch();
                  else void actorBrief.refetch();
                }}
                onTopology={topology}
                onArticle={openArticle}
              />
            </ScrollView>
          ) : screen === "brief" && article ? (
            <ScrollView
              nativeID="intel-brief-scroll"
              style={{ flex: 1, minHeight: 0 }}
              contentContainerStyle={{ flexGrow: 1 }}
            >
              <Focus
                key={`${article.run_id}/${article.id}`}
                article={article}
                onBack={() => navigate({ screen: "bulletin" })}
                onArticle={openArticle}
                onTopology={topology}
                onActor={openActor}
              />
            </ScrollView>
          ) : screen === "brief" ? (
            <ScrollView
              nativeID="intel-brief-scroll"
              style={{ flex: 1, minHeight: 0 }}
              contentContainerStyle={{ flexGrow: 1, gap: 12 }}
            >
              {directBrief.error ||
              !location?.run_id ||
              !location.article_id ? (
                <Button onPress={() => navigate({ screen: "bulletin" })}>
                  Return to Bulletin
                </Button>
              ) : null}
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
                <BriefSkeleton
                  onBack={() => navigate({ screen: "bulletin" })}
                />
              )}
            </ScrollView>
          ) : (
            <BulletinView
              sort={feedSort}
              onSort={setFeedSort}
              categories={categories}
              category={category}
              onCategory={setCategory}
              search={search}
              onSearch={setSearch}
              days={days.data?.days || []}
              selectedDay={selectedDay}
              onDay={setDay}
              daysPending={days.isLoading}
              daysSuccess={days.isSuccess}
              daysError={days.error}
              onRetryDays={() => {
                void days.refetch();
              }}
              articles={articles}
              counts={counts.data?.counts}
              list={list}
              onArticle={openArticle}
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
              actorFetching={dossier.isFetching || actorBrief.isFetching}
              actorError={dossier.error?.message || actorBrief.error?.message}
              article={screen === "brief" ? article : lastArticle}
              daysPending={days.isLoading}
              daysError={days.error?.message}
              feedFetching={list.isFetching}
              feedLoading={list.isLoading}
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
  actorFetching,
  actorError,
  article,
  daysPending,
  daysError,
  feedFetching,
  feedLoading,
  feedError,
  resolving,
  resolveError,
  seed,
}: {
  screen: "bulletin" | "brief" | "actor" | "topology" | "home";
  actorFetching: boolean;
  actorError?: string;
  article?: Article | null;
  daysPending: boolean;
  daysError?: string;
  feedFetching: boolean;
  feedLoading: boolean;
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
            detail:
              feedError ??
              (feedLoading ? "Loading" : feedFetching ? "Refreshing" : "Ready"),
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
  if (screen === "actor")
    return (
      <JobDropUp
        jobs={[
          {
            id: "actor-read",
            label: "Actor dossier",
            state: actorError
              ? "failed"
              : actorFetching
                ? "running"
                : "succeeded",
            detail: actorError || (actorFetching ? "Reading actor" : "Ready"),
          },
        ]}
      />
    );
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
            state: indexingJobState(progress.indexing?.state),
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
  onActor,
  onBack,
  onArticle,
  onTopology,
}: {
  article: Article;
  onActor: (actor: Brief["actors"][number]) => void;
  onBack: () => void;
  onArticle: (a: Article) => void;
  onTopology: (s: Seed, focus?: string) => void;
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
  let stages: BriefStage[] | undefined;
  if (live && live.progress && d && live.progress.job_id === d.job_id) {
    stages = live.progress.stages;
  }
  const shown = d?.article || a;
  const jobState = deriveBriefJobState({ ensure, reload, live, detail: d });
  const relatedContent = (
    <View style={{ gap: 16, minWidth: 0 }}>
      {!related.data && related.isLoading ? (
        <LoadingState label="Loading related leads…" compact />
      ) : null}
      {related.error && <Notice>{related.error.message}</Notice>}
      <View style={{ gap: 12, minWidth: 0, width: "100%" }}>
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
      {related.data &&
      related.data.pages.flatMap((p) => p.rows).length === 0 ? (
        <EmptyState title="No related leads">No retained leads.</EmptyState>
      ) : null}
      {related.isFetchingNextPage ? (
        <LoadingState label="Loading more related leads…" compact />
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
      onActor={onActor}
      brief={d}
      stages={stages}
      pending={detail.isLoading && !d}
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
          linksStage={stages?.find(
            (stage) => stage.operation === "intel_brief_links",
          )}
        />
      }
      notices={
        <>
          {stages && stages.length > 0 ? (
            <BriefProgressStepper
              stages={stages}
              state={jobState}
              connected={live?.connected ?? false}
            />
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
    overflow: "hidden",
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
    fontSize: 18,
    lineHeight: 25,
    fontWeight: "700",
    color: c.text,
    letterSpacing: 2,
    textTransform: "uppercase",
  },
  wordmark: {
    fontFamily: fonts.mono,
    fontSize: 12,
    letterSpacing: 1,
    textTransform: "uppercase",
    lineHeight: 19,
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
    fontSize: 12,
    color: c.muted,
    lineHeight: 20,
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
    fontSize: 28,
    lineHeight: 36,
    fontWeight: "600",
    color: c.text,
    letterSpacing: -0.5,
  },
  headline: { fontSize: 20, lineHeight: 29, fontWeight: "600", color: c.text },
  readerTitle: {
    fontSize: 27,
    lineHeight: 36,
    fontWeight: "600",
    color: c.text,
    letterSpacing: -0.4,
  },
  readerCopy: { fontSize: 17, lineHeight: 30, color: c.text, maxWidth: 640 },
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
    fontSize: 15,
    lineHeight: 21,
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
    fontSize: 15,
    lineHeight: 23,
    color: c.text,
    fontWeight: "600",
  },
  leadBasis: {
    fontFamily: fonts.mono,
    fontSize: 12,
    lineHeight: 19,
    color: c.muted,
  },
  leadChevron: {
    fontFamily: fonts.mono,
    fontSize: 17,
    color: c.accent,
    alignSelf: "center",
    marginLeft: "auto",
  },
});
