import { test, expect, type Page } from "@playwright/test";
import { installLiveFixture } from "./live-fixture";

const article = {
  id: "a",
  run_id: "r",
  title: "Recorded article mention",
  description: "A retained description.",
  category: "geopolitical",
  published_at: "2026-11-01",
  source_name: "Fixture",
  source_domain: "example.test",
  image_url: "",
  url: "",
  author: "",
};
const dossier = {
  id: "actor-1",
  display_name: "An actor with a long catalog identity name",
  subtype: "organization",
  identity_state: "provisional",
  catalog_source_id: "fixture:1",
  catalog_type: "government",
  description: "Attributed catalog description.",
  aliases: ["An unusually long alias " + "a".repeat(160)],
  metadata: [{ key: "source_datasets", value: "Offline fixture" }],
  claims: [
    {
      node: {
        id: "claim-1",
        label: "A recorded claim",
        revision: "1",
        topology_seed: { Claim: "claim-1" },
      },
      relationship: {
        id: "ref-1",
        rationale: "Recorded actor reference",
        status: "recorded",
        receipts: ["receipt-1"],
        actor_reference: { role: "subject", binding_state: "provisional" },
      },
      recorded_at: "2026-11-01",
      article: null,
    },
  ],
  briefs: [
    {
      node: { id: "article:r/a", revision: "1" },
      relationship: { id: "mention-1", receipts: ["article:r/a@1"] },
      article,
    },
  ],
  inferences: [
    {
      node: {
        id: "inference-1",
        label: "A separately recorded inference based on the claim",
        revision: "1",
        topology_seed: { Inference: "inference-1" },
      },
      relationship: {
        id: "inference-ref",
        rationale: "Premise actor association",
        status: "recorded",
        receipts: ["snapshot:fixture"],
        actor_reference: { role: "premise", binding_state: "resolved" },
      },
      recorded_at: null,
      article: null,
    },
  ],
  loaded_inferences: 1,
  loaded_claims: 1,
  loaded_briefs: 1,
  gaps: [],
  truncated: false,
};
async function fixtures(page: Page, state = "loaded") {
  await installLiveFixture(page);
  await page.route("**/api/v1/**", async (route) => {
    const url = new URL(route.request().url());
    const path = url.pathname.replace("/api/v1/", "");
    let body: unknown = {};
    if (path === "health/ready")
      body = {
        ready: true,
        storage: true,
        service: "argos-intel",
        api_version: 1,
        instance_id: "dossier-fixture",
      };
    else if (path === "actors/detail") {
      if (state === "extracted")
        return route.fulfill({
          status: 404,
          json: { error: "Resource not found" },
        });
      if (state === "error")
        return route.fulfill({
          status: 503,
          json: { error: "Read unavailable" },
        });
      body =
        state === "empty"
          ? {
              ...dossier,
              aliases: [],
              briefs: [],
              claims: [],
              inferences: [],
              loaded_inferences: 0,
              loaded_claims: 0,
              loaded_briefs: 0,
              description: null,
            }
          : dossier;
    } else if (path === "topology")
      body = {
        root_id: "actor-1",
        root_label: dossier.display_name,
        nodes: [],
        edges: [],
        findings: { gaps: [] },
        has_continuation: false,
      };
    else if (path === "articles/detail")
      body = {
        article,
        revision: "1",
        actors: [
          {
            id: "local",
            display_name: dossier.display_name,
            type_hint: "government",
            topology_seed:
              state === "unindexed" || state === "extracted"
                ? null
                : { Actor: "actor-1" },
          },
        ],
        claims: [],
        lifecycle: "active",
        status: "completed",
        publication: null,
        output: null,
        body: null,
        relations: [],
        job_id: "retained-job",
      };
    else if (path === "articles/related")
      body = { rows: [], next_cursor: null };
    await route.fulfill({ json: body });
  });
}
for (const width of [1440, 900, 899, 768, 390, 320]) {
  test(`dossier wraps retained evidence at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 1000 });
    await page.emulateMedia({ reducedMotion: "reduce" });
    await fixtures(page);
    await page.goto("/actor?id=actor-1");
    await expect(
      page.getByText("ACTOR DOSSIER", { exact: true }),
    ).toBeVisible();
    await expect(
      page.getByText("Showing 1 retained claim references"),
    ).toBeVisible();
    const cta = page.getByRole("button", { name: "OPEN ACTOR TOPOLOGY" });
    if (width < 900) {
      const name = (await page
        .getByRole("heading", { name: dossier.display_name, exact: true })
        .boundingBox())!;
      const action = (await cta.boundingBox())!;
      expect(action.y).toBeGreaterThanOrEqual(name.y + name.height);
    }
    await cta.scrollIntoViewIfNeeded();
    await expect(cta).toBeVisible();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBeTruthy();
    await page.screenshot({
      path: `.agent-scratch/actor-dossier/${width}.png`,
      fullPage: true,
    });
    await cta.click();
    await expect(page).toHaveURL(/topology\?seed=actor-1/);
    await page
      .getByRole("button", { name: "Return to Actor", exact: true })
      .click();
    await expect(page).toHaveURL(/actor\?id=actor-1/);
    await page.reload();
    await expect(
      page.getByText("ACTOR DOSSIER", { exact: true }),
    ).toBeVisible();
  });
}
test("Brief actor entry, linked occurrence, browser history and return context", async ({
  page,
}) => {
  await fixtures(page);
  await page.goto("/brief?run=r&id=a");
  await page
    .getByRole("button", {
      name: `Explore actor ${dossier.display_name}`,
      exact: true,
    })
    .click();
  await expect(page).toHaveURL(/actor\?id=actor-1/);
  await page.getByRole("button", { name: "OPEN ACTOR TOPOLOGY" }).click();
  await page
    .getByRole("button", { name: "Return to Actor", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Return to Brief", exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", {
      name: `Open Focus Brief: ${article.title}`,
    })
    .click();
  await expect(page).toHaveURL(/brief\?run=r&id=a/);
  await page.goBack();
  await expect(page).toHaveURL(/actor\?id=actor-1/);
  await page.goForward();
  await expect(page).toHaveURL(/brief\?run=r&id=a/);
});
for (const state of ["empty", "error"])
  test(`dossier ${state} state`, async ({ page }) => {
    await fixtures(page, state);
    await page.goto("/actor?id=actor-1");
    await expect(
      page.getByText(
        state === "empty"
          ? "No recorded actor–claim references"
          : "Read unavailable",
        { exact: true },
      ),
    ).toBeVisible();
    await page.screenshot({
      path: `.agent-scratch/actor-dossier/${state}.png`,
      fullPage: true,
    });
  });

for (const state of ["unindexed", "extracted"])
  test(`actor click opens a dossier with no graph seed (${state})`, async ({
    page,
  }) => {
    await fixtures(page, state);
    await page.goto("/brief?run=r&id=a");
    const actor = page.getByRole("button", {
      name: `Explore actor ${dossier.display_name}`,
      exact: true,
    });
    await expect(actor).toHaveCount(1);
    await expect(
      page.getByRole("button", { name: /^Extracted actor / }),
    ).toHaveCount(0);
    await actor.click();
    await expect(page).toHaveURL(/actor\?run=r&article=a&actor=local/);
    await expect(
      page.getByText("ACTOR DOSSIER", { exact: true }),
    ).toBeVisible();
    if (state === "extracted")
      await expect(
        page.getByText("Canonical ID · Not yet linked"),
      ).toBeVisible();
    else await expect(page.getByText("Canonical ID · actor-1")).toBeVisible();
    await page.reload();
    await expect(
      page.getByText("ACTOR DOSSIER", { exact: true }),
    ).toBeVisible();
  });

test("an open dossier refreshes after automatic retention removes associations", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.clock.install();
  await fixtures(page);
  await page.goto("/actor?id=actor-1");
  await expect(
    page.getByText("A recorded claim", { exact: true }),
  ).toBeVisible();
  await page.route("**/api/v1/actors/detail?**", (route) =>
    route.fulfill({
      json: {
        ...dossier,
        claims: [],
        inferences: [],
        briefs: [],
        loaded_claims: 0,
        loaded_inferences: 0,
        loaded_briefs: 0,
      },
    }),
  );
  await page.clock.fastForward(60_001);
  await expect(
    page.getByText("No recorded actor–claim references", { exact: true }),
  ).toBeVisible();
  await expect(page.getByText("A recorded claim", { exact: true })).toHaveCount(
    0,
  );
  await expect(
    page.getByText(
      "Showing 0 retained article mentions · mention is not corroboration",
    ),
  ).toBeVisible();
});

test("an open topology drops an insight removed by automatic retention", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.clock.install();
  await fixtures(page);
  const root = {
    id: "actor-1",
    label: "Retained actor",
    kind: "entity",
    detail: "Catalog identity",
    is_root: true,
    topology_seed: { Actor: "actor-1" },
  };
  let expired = false;
  await page.route("**/api/v1/topology?**", (route) =>
    route.fulfill({
      json: {
        root_id: "actor-1",
        root_label: root.label,
        nodes: expired
          ? [root]
          : [
              root,
              {
                id: "expired-claim",
                label: "Insight removed by retention",
                kind: "claim",
                detail: "Recorded subject association",
                is_root: false,
                topology_seed: { Claim: "expired-claim" },
              },
            ],
        edges: expired
          ? []
          : [
              {
                id: "ref",
                from: "actor-1",
                to: "expired-claim",
                label: "subject",
                kind: "concerns",
              },
            ],
        findings: { gaps: [] },
        has_continuation: false,
      },
    }),
  );
  await page.goto("/topology?seed=actor-1");
  await expect(
    page.getByText("Insight removed by retention", { exact: true }).first(),
  ).toBeVisible();
  expired = true;
  await page.clock.fastForward(60_001);
  await expect(
    page.getByText("Insight removed by retention", { exact: true }),
  ).toHaveCount(0);
});
