import React from "react";
import { describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { BackendProvider, type Article, type Brief } from "@argos/api-client";
import { BriefView, RelatedBriefCard } from "../src/views/BriefView";
import { IntelApp } from "../src";
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
      text: "Retained statement",
      confidence: 0,
      admiralty: "B2",
      classification: "fact",
    },
    {
      id: "inference-local",
      topology_seed: { Inference: "inference-memory" },
      text: "Possible outcome",
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
}: {
  data?: Brief;
  pending?: boolean;
  onTopology?: ReturnType<typeof vi.fn>;
}) {
  return (
    <BriefView
      article={article}
      brief={data}
      pending={pending}
      onBack={vi.fn()}
      onTopology={onTopology}
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
    expect(screen.getByText("B2")).toBeVisible();
    expect(screen.getByText("F6")).toBeVisible();
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
  it("sends retained actor identity/revision, supplied claim seed and article seed on real presses", async () => {
    const user = userEvent.setup();
    const onTopology = vi.fn();
    const supplied = { Claim: "published-claim" };
    render(
      <View
        data={{
          ...brief,
          claims: [{ ...brief.claims[0], topology_seed: supplied }],
        }}
        onTopology={onTopology}
      />,
    );
    screen
      .getByRole("button", { name: "Explore actor Harbor authority" })
      .focus();
    await user.keyboard("{Enter}");
    expect(onTopology).toHaveBeenLastCalledWith({ Actor: "actor-memory" });
    await user.click(
      screen.getByRole("button", { name: "Explore claim: Retained statement" }),
    );
    expect(onTopology).toHaveBeenLastCalledWith(supplied);
    screen.getByRole("button", { name: "Launch 3D topology" }).focus();
    await user.keyboard(" ");
    expect(onTopology).toHaveBeenLastCalledWith({ Claim: "claim-memory" });
  });
  it("opens extracted claims, actors and inferences before indexing assigns a memory", async () => {
    const user = userEvent.setup();
    const onTopology = vi.fn();
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
      />,
    );
    expect(
      screen.queryByText(/Awaiting Brain indexing/),
    ).not.toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Explore claim: Retained statement" }),
    );
    expect(onTopology).toHaveBeenLastCalledWith(extracted);
    await user.click(
      screen.getByRole("button", {
        name: "Explore inference: Possible outcome",
      }),
    );
    expect(onTopology).toHaveBeenLastCalledWith(extracted);
    await user.click(
      screen.getByRole("button", { name: "Explore actor Harbor authority" }),
    );
    expect(onTopology).toHaveBeenLastCalledWith(extracted);
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
        <IntelApp />
      </BackendProvider>,
    );
    await user.click(await screen.findByRole("button", { name: "Open Intel" }));
    await user.click(
      await screen.findByRole("button", {
        name: "Open Focus Brief: Retained report",
      }),
    );
    await user.click(
      await screen.findByRole("button", {
        name: "Open Focus Brief for related lead: Related report",
      }),
    );
    await screen.findByRole("heading", { name: "Related report" });
    await vi.waitFor(() => expect(admissions).toEqual([]));
  });
});
