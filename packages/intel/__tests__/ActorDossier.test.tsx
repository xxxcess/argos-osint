import React from "react";
import { it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ActorDossier } from "@argos/api-client";
import { ActorDossierView } from "../src/views/actor/ActorDossierView";

const empty: ActorDossier = {
  id: "canonical",
  display_name: "Same name",
  subtype: "organization",
  identity_state: "provisional",
  catalog_source_id: null,
  catalog_type: null,
  description: null,
  aliases: [],
  metadata: [],
  claims: [],
  briefs: [],
  loaded_claims: 0,
  loaded_briefs: 0,
  gaps: [],
  truncated: false,
};
const callbacks = () => ({
  onBack: vi.fn(),
  onRetry: vi.fn(),
  onTopology: vi.fn(),
  onArticle: vi.fn(),
});
it("renders verified zero/legacy states and canonical topology action", async () => {
  const props = callbacks();
  render(
    <ActorDossierView
      {...props}
      dossier={empty}
      pending={false}
      refreshing={false}
      missing={false}
      fromBrief
    />,
  );
  expect(screen.getByText("Showing 0 retained claim references")).toBeVisible();
  expect(screen.getByText("No known aliases recorded")).toBeVisible();
  expect(screen.getByText(/recorded organization identity/)).toBeVisible();
  await userEvent.click(
    screen.getByRole("button", { name: "OPEN ACTOR TOPOLOGY" }),
  );
  expect(props.onTopology).toHaveBeenCalledWith({ Actor: "canonical" });
  await userEvent.click(
    screen.getByRole("button", { name: "Return to Brief" }),
  );
  expect(props.onBack).toHaveBeenCalledOnce();
});
it("keeps initial skeleton, missing actor and retry failures distinct", async () => {
  const props = callbacks();
  const { rerender } = render(
    <ActorDossierView
      {...props}
      pending
      refreshing={false}
      missing={false}
      fromBrief={false}
    />,
  );
  expect(screen.getByLabelText("Loading Actor Dossier…")).toBeVisible();
  expect(screen.queryByText("No retained article mentions")).toBeNull();
  rerender(
    <ActorDossierView
      {...props}
      pending={false}
      refreshing={false}
      missing
      fromBrief={false}
    />,
  );
  expect(screen.getByText("Actor unavailable")).toBeVisible();
  rerender(
    <ActorDossierView
      {...props}
      pending={false}
      refreshing={false}
      missing={false}
      error="Read failed"
      fromBrief={false}
    />,
  );
  await userEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(props.onRetry).toHaveBeenCalledOnce();
});
it("retains snapshot on refresh failures and labels partial coverage", () => {
  render(
    <ActorDossierView
      {...callbacks()}
      dossier={{ ...empty, truncated: true }}
      pending={false}
      refreshing
      error="Refresh failed"
      missing={false}
      fromBrief={false}
    />,
  );
  expect(screen.getByText("Same name")).toBeVisible();
  expect(screen.getByText("Refresh failed")).toBeVisible();
  expect(
    screen.getByText("Partial neighborhood—open topology to explore."),
  ).toBeVisible();
});

it("shows extracted actors in a dossier without inventing catalog data or recorded associations", () => {
  const article = {
    id: "a",
    run_id: "r",
    title: "Source brief",
    description: "Retained source",
    category: "geopolitical",
    published_at: "2026-11-01",
    source_name: "Fixture",
    source_domain: "example.test",
    image_url: "",
    url: "",
    author: "",
  };
  render(
    <ActorDossierView
      {...callbacks()}
      pending={false}
      refreshing={false}
      missing={false}
      fromBrief
      extractedActor={{
        id: "local-actor",
        display_name: "Unbound name",
        type_hint: "person",
      }}
      sourceArticle={article}
    />,
  );
  expect(screen.getByText("ACTOR DOSSIER")).toBeVisible();
  expect(screen.getByText("Canonical ID · Not yet linked")).toBeVisible();
  expect(
    screen.getByText(
      "Recorded associations unavailable until an identity is linked",
    ),
  ).toBeVisible();
  expect(screen.queryByText(/Showing 0 retained/)).toBeNull();
});

it("keeps claims, premise inferences and article mentions distinct, and collapses provenance", async () => {
  const props = callbacks();
  const claim = {
    node: {
      id: "claim",
      label: "A recorded claim",
      revision: "r1",
      topology_seed: { Claim: "claim" },
    },
    relationship: {
      id: "ref",
      rationale: "Recorded subject",
      status: "recorded",
      receipts: ["private-test-receipt"],
      actor_reference: { role: "subject", binding_state: "confirmed" },
    },
    recorded_at: "2026-11-01",
    article: null,
  };
  const inference = {
    ...claim,
    node: {
      ...claim.node,
      id: "inference",
      label: "A premise inference",
      topology_seed: { Inference: "inference" },
    },
    relationship: {
      ...claim.relationship,
      id: "infer-ref",
      actor_reference: { role: "premise", binding_state: "confirmed" },
    },
  };
  render(
    <ActorDossierView
      {...props}
      dossier={{
        ...empty,
        claims: [claim],
        loaded_claims: 1,
        inferences: [inference],
        loaded_inferences: 1,
      }}
      pending={false}
      refreshing={false}
      missing={false}
      fromBrief
    />,
  );
  expect(screen.getByText("Showing 1 retained claim references")).toBeVisible();
  expect(
    screen.getByText("Showing 1 retained inference references"),
  ).toBeVisible();
  expect(screen.queryByText("private-test-receipt")).toBeNull();
  await userEvent.click(
    screen.getByRole("button", { name: "Provenance for A recorded claim" }),
  );
  expect(screen.getByText("private-test-receipt")).toBeVisible();
  await userEvent.click(
    screen.getByRole("button", {
      name: "Explore inference: A premise inference",
    }),
  );
  expect(props.onTopology).toHaveBeenCalledWith({ Inference: "inference" });
});
