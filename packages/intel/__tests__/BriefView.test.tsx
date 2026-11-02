import React from "react";
import { describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import {
  BackendProvider,
  type Article,
  type Brief,
  type BriefStage,
} from "@argos/api-client";
import { BriefView, RelatedBriefCard } from "../src/views/BriefView";
import { IntelApp } from "../src";
import { createMemoryNavigation } from "../src/navigation";
import { mswServer, testBase } from "../../../vitest.setup";
import { paint } from "../../../vitest.test-support";
import { intelColors } from "../src/shell/intel-theme";

// @tag:intel-brief @tag:testing — retained evidence, real zero and seed contracts.
const article: Article = {
  id: "raw-id",
  run_id: "run-1",
  title: "Retained report",
  description: "Recorded summary",
  category: "geopolitical",
  source_name: "Fixture Wire",
  source_domain: "example.test",
  author: "Reporter",
  url: "https://example.test/report",
  image_url: "",
  published_at: "2026-10-29T12:00:00Z",
};
const brief: Brief = {
  article,
  source_revision: "source-r1",
  topology_seed: { Claim: "claim-memory" },
  revision: "brief-r1",
  actors: [
    {
      id: "local-actor",
      display_name: "Harbor authority",
      topology_seed: { Actor: "actor-memory" },
      type_hint: "organization",
    },
  ],
  claims: [
    {
      id: "claim-local",
      topology_seed: { Claim: "claim-memory" },
      text: "[claim · supports] Retained statement",
      confidence: 0,
      admiralty: "B2",
      classification: "fact",
    },
    {
      id: "inference-local",
      topology_seed: { Inference: "inference-memory" },
      text: "[inference · references] Possible outcome",
      confidence: null,
      admiralty: "F6",
      classification: "inference",
    },
  ],
  body: null,
  output: null,
  publication: null,
  lifecycle: "complete",
  status: "Published",
  relations: [],
  confidence: { score: 0, provisional: false, claims: 1, inferences: 1 },
  job_id: "job-r1",
};
function View({
  data = brief,
  pending = false,
  onTopology = vi.fn(),
  onActor = vi.fn(),
  stages,
}: {
  data?: Brief;
  stages?: BriefStage[];
  pending?: boolean;
  onTopology?: ReturnType<typeof vi.fn>;
  onActor?: ReturnType<typeof vi.fn>;
}) {
  return (
    <BriefView
      article={article}
      brief={data}
      pending={pending}
      stages={stages}
      onBack={vi.fn()}
      onTopology={onTopology}
      onActor={onActor}
      related={null}
      operations={null}
      notices={null}
      source={null}
    />
  );
}
describe("Focus Brief", () => {
  it("keeps actors then retained claims then inferences, without implying corroboration or fabricating fields", () => {
    render(<View />);
    const headings = screen
      .getAllByRole("heading")
      .map((element) => element.textContent);
    expect(headings).toEqual(
      expect.arrayContaining([
        "IDENTIFIED ACTORS",
        "RETAINED CLAIMS",
        "ANALYTICAL INFERENCES",
        "CORRELATED INTELLIGENCE REPORTS",
      ]),
    );
    expect(headings.indexOf("IDENTIFIED ACTORS")).toBeLessThan(
      headings.indexOf("RETAINED CLAIMS"),
    );
    expect(headings.indexOf("RETAINED CLAIMS")).toBeLessThan(
      headings.indexOf("ANALYTICAL INFERENCES"),
    );
    expect(headings.indexOf("ANALYTICAL INFERENCES")).toBeLessThan(
      headings.indexOf("CORRELATED INTELLIGENCE REPORTS"),
    );
    expect(
      within(screen.getByTestId("reader-inferences")).getByText(
        "Possible outcome",
      ),
    ).toBeVisible();
    expect(screen.getByText("[A]")).toBeVisible();
    expect(
      screen.queryByText(/\[claim ·|\[inference ·/),
    ).not.toBeInTheDocument();
    expect(screen.queryByText("raw-id")).not.toBeInTheDocument();
    expect(
      screen.queryByText(/TLP:|REF-000|COOLING DOWN|CORROBORATED CLAIMS/),
    ).not.toBeInTheDocument();
  });
  it.each<[number, string]>([
    [0, intelColors.danger],
    [39, intelColors.danger],
    [40, intelColors.warning],
    [69, intelColors.warning],
    [70, intelColors.accent],
  ])("renders real confidence %s with its threshold color", (score, color) => {
    render(
      <View data={{ ...brief, confidence: { ...brief.confidence!, score } }} />,
    );
    expect(paint(screen.getByText(`${score}%`, { exact: true }), "color")).toBe(
      color.toLowerCase(),
    );
  });
  it("distinguishes unavailable confidence from zero and respects canonical empty inferences", () => {
    render(
      <View
        data={{ ...brief, confidence: null, output: { inferences: [] } }}
      />,
    );
    const header = screen.getByRole("heading", {
      name: "Retained report",
    }).parentElement!;
    expect(within(header).getByText("B2", { exact: true })).toBeVisible();
    expect(within(header).getByText("F6", { exact: true })).toBeVisible();
    expect(screen.queryByText("Not assessed")).not.toBeInTheDocument();
    expect(
      within(screen.getByTestId("reader-inferences")).getByRole("heading", {
        name: "No retained output",
      }),
    ).toBeVisible();
    expect(screen.queryByText("Possible outcome")).not.toBeInTheDocument();
  });
  it("distinguishes pending evidence from verified empty and unavailable data", () => {
    const props = {
      article,
      onBack: vi.fn(),
      onTopology: vi.fn(),
      related: null,
      operations: null,
      notices: null,
      source: null,
    };
    const { rerender } = render(<BriefView {...props} pending />);
    expect(
      screen.getByRole("status", { name: "Loading extracted evidence…" }),
    ).toBeVisible();
    expect(
      screen.queryByRole("heading", { name: "No actors extracted" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("heading", { name: "No published claims" }),
    ).not.toBeInTheDocument();
    rerender(
      <BriefView
        {...props}
        pending={false}
        brief={{ ...brief, actors: [], claims: [], output: null }}
      />,
    );
    expect(
      screen.getByRole("heading", { name: "No actors extracted" }),
    ).toBeVisible();
    expect(
      screen.getByRole("heading", { name: "No published claims" }),
    ).toBeVisible();
    rerender(<BriefView {...props} pending={false} />);
    expect(
      screen.getAllByRole("heading", { name: "Evidence unavailable" }),
    ).toHaveLength(2);
    expect(
      screen.getByRole("heading", { name: "Inferences unavailable" }),
    ).toBeVisible();
  });
  it("keeps a completed snapshot and verified empty sections visible while pending", () => {
    const { rerender } = render(<View pending data={brief} />);
    expect(screen.getByText("Retained statement")).toBeVisible();
    expect(screen.getByText("Possible outcome")).toBeVisible();
    expect(screen.queryAllByRole("status", { name: /Loading/ })).toHaveLength(
      0,
    );

    rerender(
      <View
        pending
        data={{
          ...brief,
          actors: [],
          claims: [],
          inferences: [],
          output: null,
        }}
      />,
    );
    expect(
      screen.getByRole("heading", { name: "No actors extracted" }),
    ).toBeVisible();
    expect(
      screen.getByRole("heading", { name: "No published claims" }),
    ).toBeVisible();
    expect(
      screen.getByRole("heading", { name: "No retained output" }),
    ).toBeVisible();
    expect(screen.queryAllByRole("status", { name: /Loading/ })).toHaveLength(
      0,
    );
  });

  it("renders stored insights while catalog actors are still matching", () => {
    render(<View data={{ ...brief, actors: [], actors_pending: true }} />);
    expect(screen.getByText("Retained statement")).toBeVisible();
    expect(screen.getByText("Possible outcome")).toBeVisible();
    expect(
      screen.getByRole("status", { name: "Matching catalog actors…" }),
    ).toBeVisible();
    expect(
      screen.queryByRole("heading", { name: "No actors extracted" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByText("No actors", { exact: true }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("status", { name: "Loading retained claims…" }),
    ).not.toBeInTheDocument();
  });

  it("sends retained actor identity/revision, supplied claim seed and article seed on real presses", async () => {
    const user = userEvent.setup();
    const onTopology = vi.fn();
    const onActor = vi.fn();
    const supplied = { Claim: "published-claim" };
    render(
      <View
        data={{
          ...brief,
          claims: [{ ...brief.claims[0], topology_seed: supplied }],
        }}
        onTopology={onTopology}
        onActor={onActor}
      />,
    );
    screen
      .getByRole("button", { name: "Explore actor Harbor authority" })
      .focus();
    await user.keyboard("{Enter}");
    expect(onActor).toHaveBeenLastCalledWith(brief.actors[0]);
    await user.click(
      screen.getByRole("button", { name: "Explore claim: Retained statement" }),
    );
    expect(onTopology).toHaveBeenLastCalledWith(supplied);
    screen.getByRole("button", { name: "Launch 3D topology" }).focus();
    await user.keyboard(" ");
    expect(onTopology).toHaveBeenLastCalledWith({ Claim: "claim-memory" });
  });
  it("opens extracted actors in dossiers while claims and inferences retain topology navigation", async () => {
    const user = userEvent.setup();
    const onTopology = vi.fn();
    const onActor = vi.fn();
    const extracted = { Brief: "brief:run-1/raw-id" };
    render(
      <View
        data={{
          ...brief,
          topology_seed: null,
          actors: brief.actors.map((actor) => ({
            ...actor,
            topology_seed: null,
          })),
          claims: brief.claims.map((claim) => ({
            ...claim,
            topology_seed: null,
          })),
        }}
        onTopology={onTopology}
        onActor={onActor}
      />,
    );
    expect(
      screen.queryByText(/Awaiting Brain indexing/),
    ).not.toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Explore claim: Retained statement" }),
    );
    expect(onTopology).toHaveBeenLastCalledWith(extracted, "claim:claim-local");
    await user.click(
      screen.getByRole("button", {
        name: "Explore inference: Possible outcome",
      }),
    );
    expect(onTopology).toHaveBeenLastCalledWith(
      extracted,
      "inference:inference-local",
    );
    await user.click(
      screen.getByRole("button", { name: "Explore actor Harbor authority" }),
    );
    expect(onActor).toHaveBeenCalledWith({
      ...brief.actors[0],
      topology_seed: null,
    });
    await user.click(
      screen.getByRole("button", { name: "Launch 3D topology" }),
    );
    expect(onTopology).toHaveBeenLastCalledWith(extracted);
  });
  it("opens related cards with keyboard activation and preserves basis/tentative attribution", async () => {
    const user = userEvent.setup();
    const onPress = vi.fn();
    render(
      <RelatedBriefCard
        article={article}
        basis={["shared actor"]}
        tentative
        onPress={onPress}
      />,
    );
    expect(
      screen.getByText(/shared actor · tentative.*not corroboration/),
    ).toBeVisible();
    const card = screen.getByRole("button", {
      name: "Open Focus Brief for related lead: Retained report",
    });
    card.focus();
    await user.keyboard("{Enter}");
    expect(onPress).toHaveBeenCalledOnce();
  });
  it("navigates a related occurrence that already ran without admitting another chain", async () => {
    const navigation = createMemoryNavigation();
    navigation.navigate({
      screen: "brief",
      run_id: article.run_id,
      article_id: article.id,
    });
    const admissions: unknown[] = [];
    const next = {
      ...article,
      id: "related-id",
      run_id: "run-2",
      title: "Related report",
    };
    mswServer.use(
      http.get(`${testBase}/api/v1/health/ready`, () =>
        HttpResponse.json({
          ready: true,
          storage: true,
          service: "argos-intel",
          api_version: 1,
          instance_id: "brief-phase3",
        }),
      ),
      http.get(`${testBase}/api/v1/days`, () =>
        HttpResponse.json({ days: ["2026-10-29"] }),
      ),
      http.get(`${testBase}/api/v1/articles`, () =>
        HttpResponse.json({
          rows: [article],
          next_cursor: null,
          revision: "rows",
        }),
      ),
      http.get(`${testBase}/api/v1/articles/detail`, ({ request }) =>
        HttpResponse.json({
          ...brief,
          article:
            new URL(request.url).searchParams.get("id") === next.id
              ? next
              : article,
        }),
      ),
      http.get(`${testBase}/api/v1/articles/related`, ({ request }) =>
        HttpResponse.json({
          rows:
            new URL(request.url).searchParams.get("id") === next.id
              ? []
              : [{ article: next, basis: ["shared actor"], tentative: true }],
          next_cursor: null,
        }),
      ),
      http.post(`${testBase}/api/v1/brief/ensure`, async ({ request }) => {
        admissions.push(await request.json());
        return HttpResponse.json({ job_id: "job" });
      }),
    );
    const user = userEvent.setup();
    render(
      <BackendProvider
        settings={{
          subscribe: () => () => {},
          getSnapshot: () => testBase,
          getServerSnapshot: () => testBase,
          setBase: () => {},
          createRequestId: () => "request-1",
        }}
      >
        <IntelApp navigation={navigation} />
      </BackendProvider>,
    );
    await screen.findByRole("heading", { name: article.title });
    await user.click(
      await screen.findByRole("button", {
        name: "Open Focus Brief for related lead: Related report",
      }),
    );
    await screen.findByRole("heading", { name: "Related report" });
    await vi.waitFor(() => expect(admissions).toEqual([]));
  });
});

it("uses each section's stage independently and restores retained content on completion", async () => {
  const stage = (
    operation: string,
    state: string,
    label: string,
  ): BriefStage => ({
    operation,
    state,
    label,
    row: state,
    active: state === "running",
    detail: state === "failed" ? "Provider unavailable" : "",
  });
  const { rerender } = render(
    <View
      stages={[
        stage("intel_brief_claims", "running", "Claims"),
        stage("intel_brief_inferences", "failed", "Inferences"),
      ]}
    />,
  );
  expect(
    await screen.findByRole("status", { name: "Claims: running" }),
  ).toBeVisible();
  expect(screen.getByRole("alert")).toHaveTextContent(
    "Inferences: Provider unavailable",
  );
  expect(screen.queryByText("Retained statement")).toBeNull();
  expect(screen.queryByText("Possible outcome")).toBeNull();
  expect(
    screen.getByRole("button", { name: "Explore actor Harbor authority" }),
  ).toBeVisible();
  rerender(
    <View
      stages={[
        stage("intel_brief_claims", "completed", "Claims"),
        stage("intel_brief_inferences", "completed", "Inferences"),
      ]}
    />,
  );
  expect(screen.getByText("Retained statement")).toBeVisible();
  expect(screen.getByText("Possible outcome")).toBeVisible();
  expect(
    screen.queryByRole("status", { name: /Claims:|Inferences:/ }),
  ).toBeNull();
  expect(screen.queryByRole("alert")).toBeNull();
});

it("keeps one actor section and opens dossier actions for canonical and extracted actors", async () => {
  const onActor = vi.fn(),
    onTopology = vi.fn();
  const props = {
    article,
    brief,
    pending: false,
    onBack: vi.fn(),
    onActor,
    onTopology,
    related: null,
    operations: null,
    notices: null,
    source: null,
  };
  const { rerender } = render(<BriefView {...props} />);
  await userEvent.click(
    screen.getByRole("button", { name: "Explore actor Harbor authority" }),
  );
  expect(onActor).toHaveBeenLastCalledWith(brief.actors[0]);
  expect(
    screen.queryByRole("button", { name: "Extracted actor Harbor authority" }),
  ).toBeNull();
  expect(onTopology).not.toHaveBeenCalled();
  const extracted = { ...brief.actors[0], topology_seed: null };
  rerender(<BriefView {...props} brief={{ ...brief, actors: [extracted] }} />);
  await userEvent.click(
    screen.getByRole("button", { name: "Explore actor Harbor authority" }),
  );
  expect(onActor).toHaveBeenLastCalledWith(extracted);
  expect(onTopology).not.toHaveBeenCalled();
});

it("keeps an empty actor section loading until cleaned-body matching finishes", () => {
  const pending = {
    ...brief,
    actors: [],
    actors_pending: true,
    actors_body_pending: true,
  };
  const { rerender } = render(<View data={pending} />);
  expect(
    screen.getByRole("status", {
      name: "Checking cleaned article for actors…",
    }),
  ).toBeVisible();
  expect(screen.queryByText("No actors extracted")).toBeNull();
  rerender(
    <View
      data={{ ...pending, actors_pending: false, actors_body_pending: false }}
    />,
  );
  expect(screen.getByText("No actors extracted")).toBeVisible();
});
it("keeps metadata actors visible and appends cleaned-body matches without replacing them", () => {
  const initial = { ...brief, actors_body_pending: true };
  const { rerender } = render(<View data={initial} />);
  expect(
    screen.getByRole("button", { name: "Explore actor Harbor authority" }),
  ).toBeVisible();
  expect(
    screen.getByRole("progressbar", {
      name: "Checking cleaned article for more actors…",
    }),
  ).toBeVisible();
  const bodyActor = {
    id: "body-actor",
    display_name: "Body-only organization",
    type_hint: "organization",
    topology_seed: { Actor: "body-canonical" },
  };
  rerender(
    <View
      data={{
        ...initial,
        actors_body_pending: false,
        actors: [...brief.actors, bodyActor],
      }}
    />,
  );
  expect(
    screen
      .getAllByRole("button", { name: /^Explore actor / })
      .map((button) => button.getAttribute("aria-label")),
  ).toEqual([
    "Explore actor Harbor authority",
    "Explore actor Body-only organization",
  ]);
});
it("does not label a failed body cleanup as a verified empty actor collection", () => {
  render(
    <View data={{ ...brief, actors: [], actors_body_unavailable: true }} />,
  );
  expect(screen.getByText("Actor detection incomplete")).toBeVisible();
  expect(screen.queryByText("No actors extracted")).toBeNull();
});

it("shows zero confidence above identified actors with the terminal grade", () => {
  render(<View data={{ ...brief, confidence_grade: "B2" }} />);
  expect(screen.getByText("Brief rating 0/100")).toBeInTheDocument();
  expect(screen.getByLabelText("Admiralty grade B2")).toHaveTextContent("██");
  const confidence = screen.getByRole("button", { name: "ARTICLE CONFIDENCE" });
  const actors = screen.getByRole("button", { name: /IDENTIFIED ACTORS/ });
  expect(
    confidence.compareDocumentPosition(actors) &
      Node.DOCUMENT_POSITION_FOLLOWING,
  ).toBeTruthy();
});

it("updates generated inference totals independently of scored inference eligibility", () => {
  const initial = {
    ...brief,
    confidence: { score: null, provisional: true, claims: 1, inferences: 0 },
    inferences: [],
  };
  const { rerender } = render(<View data={initial} />);
  expect(screen.getByText("1 claims · 0 inferences")).toBeVisible();
  rerender(
    <View
      data={{
        ...initial,
        inferences: [{ id: "new", text: "New generated inference" }],
      }}
    />,
  );
  expect(screen.getByText("1 claims · 1 inferences")).toBeVisible();
  expect(screen.getByText("New generated inference")).toBeVisible();
});
