import React from "react";
import { describe, it, expect } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import {
  BackendProvider,
  BRIEF_FACETS,
  type NeighborhoodBrief,
  type BackendSettings,
} from "@argos/api-client";
import { NeighborhoodSummary } from "../src/topology/NeighborhoodSummary";
import { mswServer, testBase } from "../../../vitest.setup";

const item = {
  text: "The retained claim reports the port closure.",
  epistemic: "recorded" as const,
  node_ids: ["claim-1"],
  edge_ids: [],
  member_ids: [],
  receipts: [],
  basis: [1],
};
const brief: NeighborhoodBrief = {
  format_version: 2,
  basis_key: "captured-source-and-route",
  seed: { Claim: "claim-1" },
  seed_label: "Port claim",
  overview: item,
  connections: [],
  interpretation: [],
  limits: [],
  references: [{ n: 1, target: { Node: "claim-1" }, label: "Port claim" }],
  coverage: {
    status: "limited",
    depth: 1,
    cycle_scope: "AllRetained",
    admitted_nodes: 2,
    admitted_edges: 1,
    discovered_nodes: 4,
    discovered_edges: 3,
    omitted_nodes: 2,
    omitted_edges: 2,
    unresolved_endpoints: 0,
    exploration_exhausted: true,
    payload_reads_skipped: 0,
  },
  facets: BRIEF_FACETS.map((facet) => ({
    facet: facet.id,
    items: facet.id === "current_state" ? [item] : [],
    empty_reason:
      facet.id === "current_state"
        ? null
        : "No supported retained evidence for this facet.",
  })),
};
const settings: BackendSettings = {
  subscribe: () => () => {},
  getSnapshot: () => testBase,
  getServerSnapshot: () => testBase,
  setBase: () => {},
  createRequestId: () => "explicit-summary-action",
};
function setup(failure = false) {
  const writes: unknown[] = [];
  mswServer.use(
    http.get(`${testBase}/api/v1/health/ready`, () =>
      HttpResponse.json({
        ready: true,
        storage: true,
        service: "argos-intel",
        api_version: 1,
        instance_id: "summary-fixture",
      }),
    ),
    http.get(`${testBase}/api/v1/topology/summary`, () =>
      HttpResponse.json({ kind: "recorded", brief }),
    ),
    http.post(
      `${testBase}/api/v1/topology/summary/generate`,
      async ({ request }) => {
        writes.push(await request.json());
        return failure
          ? HttpResponse.json(
              { error: "Evidence changed; refresh topology before generating" },
              { status: 409 },
            )
          : HttpResponse.json({ kind: "pending", brief });
      },
    ),
  );
  render(
    <BackendProvider settings={settings}>
      <NeighborhoodSummary
        seed="claim-1"
        fallback={brief}
        nodes={[
          {
            id: "claim-1",
            label: "Port claim",
            kind: "Claim",
            detail: "",
            topology_seed: { Claim: "claim-1" },
          },
        ]}
        onSeed={() => {}}
      />
    </BackendProvider>,
  );
  return writes;
}

describe("Neighborhood summary authorization", () => {
  it("reads eight facets without queuing and sends one explicit captured-basis action", async () => {
    const writes = setup();
    for (const facet of BRIEF_FACETS)
      expect(
        screen.getByRole("heading", { name: facet.name }),
      ).toBeInTheDocument();
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Generate summary" }),
      ).toBeEnabled(),
    );
    expect(writes).toEqual([]);
    expect(screen.getByText(/2 of 4 discovered records/)).toHaveTextContent(
      "0 payload reads skipped",
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Generate summary" }),
    );
    await waitFor(() =>
      expect(writes).toEqual([{ seed: "claim-1", basis: brief.basis_key }]),
    );
    expect(
      await screen.findByRole("button", { name: "Generation pending" }),
    ).toBeDisabled();
    expect(
      screen.getByText(/retained claim reports the port closure/),
    ).toBeInTheDocument();
  });
  it("keeps recorded evidence visible after a stale-basis action is rejected", async () => {
    const writes = setup(true);
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Generate summary" }),
      ).toBeEnabled(),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Generate summary" }),
    );
    expect(
      await screen.findByText(/Generation was not accepted/),
    ).toHaveTextContent("Evidence changed");
    expect(writes).toHaveLength(1);
    expect(
      screen.getByText(/retained claim reports the port closure/),
    ).toBeInTheDocument();
  });
});
