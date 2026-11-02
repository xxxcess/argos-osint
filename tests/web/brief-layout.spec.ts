import { test, expect, type Page } from "@playwright/test";
import { installLiveFixture } from "./live-fixture";

const captureDir = process.env.ARGOS_WEB_CAPTURE_DIR || ".agent-scratch/web-ui";

const article = {
  run_id: "fixture-run",
  id: "fixture-article",
  title: "Retained evidence report",
  description: "Stored article summary.",
  category: "geopolitical",
  published_at: "2026-11-01T12:00:00Z",
  source_name: "Fixture Wire",
  source_domain: "example.test",
  author: "",
  image_url: "",
  url: "https://example.test/report",
};
const brief = {
  article,
  revision: "brief-complete",
  source_revision: "source",
  job_id: "completed-job",
  actors: [],
  claims: [
    {
      id: "claim",
      text: "Stored claim",
      classification: "fact",
      admiralty: "B2",
      confidence: 0.7,
    },
  ],
  inferences: [
    {
      id: "inference",
      text: "Stored inference must render",
      topology_seed: { Inference: "node-0" },
    },
  ],
  topology_seed: { Inference: "node-0" },
  output: null,
  body: null,
  links: [],
  relations: [],
  lifecycle: "active",
  status: "Published",
  publication: [1, {}, {}],
  indexing: { state: "indexed" },
};
const graph = {
  root_id: "node-0",
  root_label: "Inference 0",
  root_kind: "inference",
  nodes: Array.from({ length: 36 }, (_, i) => ({
    id: `node-${i}`,
    kind: "inference",
    label: `Inference ${i}`,
    detail: Array.from(
      { length: 30 },
      (_, j) =>
        `Retained insight paragraph ${j}: source evidence and reasoning stay readable.`,
    ).join("\n\n"),
    is_root: i === 0,
    min_hops: i === 0 ? 0 : 1,
    topology_seed: { Inference: `node-${i}` },
  })),
  edges: Array.from({ length: 35 }, (_, i) => ({
    id: `edge-${i}`,
    from: "node-0",
    to: `node-${i + 1}`,
    kind: "Premise",
    label: "supports",
  })),
  findings: { recorded: [], coverage: [], gaps: [], active_inferences: [] },
  has_continuation: false,
};
async function setup(
  page: Page,
  holdDetail = false,
  snapshot: () => unknown = () => brief,
) {
  await installLiveFixture(page);
  let release!: () => void;
  const pending = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route("**/api/v1/**", async (route) => {
    const path = new URL(route.request().url()).pathname.split("/api/v1/")[1];
    if (path === "articles/detail" && holdDetail) await pending;
    const payload =
      path === "health/ready"
        ? {
            ready: true,
            storage: true,
            service: "argos-intel",
            api_version: 1,
            instance_id: "scroll-review",
          }
        : path === "articles/detail"
          ? snapshot()
          : path === "articles/related"
            ? { rows: [], next_cursor: null }
            : path === "topology"
              ? graph
              : path === "days"
                ? { days: [] }
                : {};
    await route.fulfill({ json: payload });
  });
  return release;
}
async function fixedBar(page: Page) {
  const box = (await page.locator("#intel-status-bar").boundingBox())!;
  expect(
    Math.abs(box.y + box.height - (await page.evaluate(() => innerHeight))),
  ).toBeLessThanOrEqual(1);
}

test("completed brief replaces its skeleton with stored evidence", async ({
  page,
}) => {
  const release = await setup(page, true);
  await page.goto("/brief?run=fixture-run&id=fixture-article");
  try {
    await expect(
      page.getByRole("status", { name: "Loading Focus Brief…", exact: true }),
    ).toBeVisible();
    await page.screenshot({
      path: `${captureDir}/brief-skeleton.png`,
    });
  } finally {
    release();
  }
  await expect(page.getByText("Stored claim", { exact: true })).toBeVisible();
  await expect(
    page.getByText("Stored inference must render", { exact: true }),
  ).toBeVisible();
  await expect(page.getByRole("status", { name: /Loading/ })).toHaveCount(0);
  await fixedBar(page);
  await page.screenshot({
    path: `${captureDir}/brief-loaded.png`,
    fullPage: false,
  });
});

test("catalog matching never blocks stored insights and refreshes when ready", async ({
  page,
}) => {
  let ready = false;
  await setup(page, false, () => ({
    ...brief,
    actors_pending: !ready,
    actors: ready
      ? [
          {
            id: "catalog-actor-ready",
            display_name: "Ready catalog actor",
            type_hint: "organization",
          },
        ]
      : [],
  }));
  await page.goto("/brief?run=fixture-run&id=fixture-article");
  await expect(
    page.getByText("Stored inference must render", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("status", { name: "Matching catalog actors…" }),
  ).toBeVisible();
  await expect(
    page.getByRole("status", { name: "Loading retained claims…" }),
  ).toHaveCount(0);
  ready = true;
  const live = await installLiveFixture(page);
  live.brief({
    run: article.run_id,
    id: article.id,
    job_id: "completed-job",
    stages: [],
    indexing: { state: "indexed", reason: "" },
    complete: true,
    refresh: true,
  });
  await expect(
    page.getByRole("button", { name: "Explore actor Ready catalog actor" }),
  ).toBeVisible();
  await expect(
    page.getByRole("status", { name: "Matching catalog actors…" }),
  ).toHaveCount(0);
});

for (const width of [1280, 390]) {
  test(`topology insight lists scroll without moving the bottom navigation at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 844 });
    await setup(page);
    await page.goto("/topology?seed=node-0");
    const list = page.locator("#intel-topology-node-scroll");
    await expect(list).toBeVisible();
    await expect
      .poll(() => list.evaluate((el) => el.scrollHeight - el.clientHeight))
      .toBeGreaterThan(0);
    await fixedBar(page);
    await list.scrollIntoViewIfNeeded();
    await list.evaluate((el) => {
      el.scrollTop = el.scrollHeight;
    });
    const graphBefore = (await page
      .locator("#intel-topology-viewport")
      .boundingBox())!;
    await list.hover();
    await page.mouse.wheel(0, 900);
    await page.evaluate(
      () =>
        new Promise<void>((resolve) => requestAnimationFrame(() => resolve())),
    );
    const graphAfter = (await page
      .locator("#intel-topology-viewport")
      .boundingBox())!;
    expect(Math.abs(graphAfter.y - graphBefore.y)).toBeLessThanOrEqual(1);
    await fixedBar(page);
    await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(0);
    const nodes = page.locator("#intel-topology-node-scroll");
    await nodes.scrollIntoViewIfNeeded();
    await expect
      .poll(() => nodes.evaluate((el) => el.scrollHeight - el.clientHeight))
      .toBeGreaterThan(0);
    await nodes.evaluate((el) => {
      el.scrollTop = el.scrollHeight;
    });
    const connections = page.locator("#intel-topology-connection-scroll");
    await connections.scrollIntoViewIfNeeded();
    await connections.evaluate((el) => {
      el.scrollTop = el.scrollHeight;
    });
    await expect(
      page.getByText("Inference 0 → supports → Inference 35", { exact: true }),
    ).toBeVisible();
    const nodesBox = (await nodes.boundingBox())!;
    const barBox = (await page.locator("#intel-status-bar").boundingBox())!;
    expect(nodesBox.y + nodesBox.height).toBeLessThanOrEqual(barBox.y);
    await fixedBar(page);
    await page.screenshot({
      path: `${captureDir}/topology-${width}.png`,
      fullPage: false,
    });
  });
}

test("live stages replace retained output until completed and show failures", async ({
  page,
}) => {
  await setup(page);
  const live = await installLiveFixture(page);
  live.metrics({
    cpu_percent: 12,
    memory_percent: 40,
    disk_percent: 50,
    network_kbps: 3376,
    platform: "macOS 26.0",
    chipset: "x86 · 16 cores",
  });
  await page.goto("/brief?run=fixture-run&id=fixture-article");
  await expect(
    page.getByText("Stored inference must render", { exact: true }),
  ).toBeVisible();
  const progress = (state: string, detail: string) =>
    live.brief({
      run: article.run_id,
      id: article.id,
      job_id: "completed-job",
      stages: [
        {
          operation: "intel_brief_inferences",
          label: "Inferences",
          state,
          row: state,
          active: state === "running",
          detail,
        },
      ],
      indexing: { state: "indexed", reason: "" },
      complete: state !== "running",
      refresh: false,
    });
  progress("running", "Waiting for model");
  await expect(
    page.getByRole("status", { name: "Inferences: running", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Stored inference must render", { exact: true }),
  ).toHaveCount(0);
  await expect(page.getByText("Stored claim", { exact: true })).toBeVisible();
  const server = page.getByRole("button", {
    name: "macOS 26.0, x86 · 16 cores",
    exact: true,
  });
  await expect(server).toBeVisible();
  await expect(page.getByText("x86 · 16 cores", { exact: true })).toBeVisible();
  const sizes = await page.evaluate(() =>
    [
      document.querySelector("#intel-status-api"),
      document.querySelector("#intel-status-metric-cpu"),
      [...document.querySelectorAll("#intel-status-bar div")].find(
        (el) => el.textContent === "x86 · 16 cores",
      ),
    ].map((el) => el && getComputedStyle(el).fontSize),
  );
  expect(sizes).toEqual(["18px", "18px", "18px"]);
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(page.getByLabel("API connected")).toBeVisible();
  const apiBox = (await page.getByLabel("API connected").boundingBox())!;
  expect(apiBox.x + apiBox.width).toBeLessThanOrEqual(390);
  await fixedBar(page);
  await page.screenshot({
    path: `${captureDir}/server-chip-390.png`,
  });
  await page.setViewportSize({ width: 1280, height: 844 });
  await page.screenshot({
    path: `${captureDir}/brief-stage-running.png`,
  });
  progress("failed", "Provider unavailable");
  await expect(
    page
      .getByRole("alert")
      .filter({ hasText: "Inferences: Provider unavailable" }),
  ).toBeVisible();
  progress("completed", "Ready");
  await expect(
    page.getByText("Stored inference must render", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("status", { name: "Inferences: running", exact: true }),
  ).toHaveCount(0);
  await fixedBar(page);
});

test("only matching-generation stages control claims, inferences and links", async ({
  page,
}) => {
  await setup(page);
  const live = await installLiveFixture(page);
  await page.goto("/brief?run=fixture-run&id=fixture-article");
  await expect(page.getByText("Stored claim", { exact: true })).toBeVisible();
  const stages = [
    {
      operation: "intel_brief_claims",
      label: "Claims",
      state: "queued",
      row: "Queued",
      detail: "",
      active: false,
    },
    {
      operation: "intel_brief_inferences",
      label: "Inferences",
      state: "failed",
      row: "Failed",
      detail: "Model unavailable",
      active: false,
    },
    {
      operation: "intel_brief_links",
      label: "Links",
      state: "running",
      row: "Running",
      detail: "",
      active: true,
    },
  ];
  const progress = {
    run: article.run_id,
    id: article.id,
    job_id: "other-generation",
    stages,
    indexing: null,
    complete: false,
    refresh: false,
  };
  live.brief(progress);
  await page
    .getByRole("button", { name: "Background jobs", exact: true })
    .click();
  await expect(page.getByRole("menu")).toContainText("Model unavailable");
  await page
    .getByRole("button", { name: "Background jobs", exact: true })
    .click();
  await expect(page.getByText("Stored claim", { exact: true })).toBeVisible();
  await expect(
    page.getByText("Stored inference must render", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("status", { name: "Claims: Queued", exact: true }),
  ).toHaveCount(0);
  live.brief({ ...progress, job_id: brief.job_id });
  await expect(
    page.getByRole("status", { name: "Claims: Queued", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("status", { name: "Links: Running", exact: true }),
  ).toBeVisible();
  await expect(
    page
      .getByRole("alert")
      .filter({ hasText: "Inferences: Model unavailable" }),
  ).toBeVisible();
  await expect(page.getByText("Stored claim", { exact: true })).toHaveCount(0);
  await expect(
    page.getByText("Stored inference must render", { exact: true }),
  ).toHaveCount(0);
  live.brief({
    ...progress,
    job_id: brief.job_id,
    complete: true,
    stages: stages.map((stage) => ({
      ...stage,
      state: "completed",
      row: "Completed",
      detail: "",
      active: false,
    })),
  });
  await expect(page.getByText("Stored claim", { exact: true })).toBeVisible();
  await expect(
    page.getByText("Stored inference must render", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("No retained output", { exact: true }),
  ).toBeVisible();
  live.disconnect();
  await expect(page.getByText("Stored claim", { exact: true })).toBeVisible();
  await expect(
    page.getByRole("status", { name: /Claims:|Links:/ }),
  ).toHaveCount(0);
});
