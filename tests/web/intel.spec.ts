import { installLiveFixture } from "./live-fixture";
import { test, expect, type Page } from "@playwright/test";
async function gotoBulletin(page: Page, url = "/") {
  await page.goto(url);
  const launcher = page.getByRole("button", {
    name: "Open Intel",
    exact: true,
  });
  await expect(launcher).toBeVisible({ timeout: 35000 });
  await expect(
    page.getByRole("button", { name: "Bulletin", exact: true }),
  ).toHaveCount(0);
  await launcher.click();
  await expect(
    page.getByRole("heading", { name: "Intelligence bulletin", exact: true }),
  ).toBeVisible();
}
async function expectFixedStatusBar(page: Page) {
  const bar = page.locator("#intel-status-bar");
  await expect(bar).toBeVisible();
  const box = (await bar.boundingBox())!;
  const scale = await page
    .locator("html")
    .evaluate((el) => Number(getComputedStyle(el).zoom) || 1);
  expect(box.height).toBe(64 * scale);
  expect(
    Math.abs(box.y + box.height - (await page.evaluate(() => innerHeight))),
  ).toBeLessThanOrEqual(1);
}
test.use({
  reducedMotion: "reduce",
  video:
    process.env.ARGOS_WEB_RECORD_VIDEO === "1" ? "on" : "retain-on-failure",
});
const captureDir = process.env.ARGOS_WEB_CAPTURE_DIR || ".agent-scratch/web-ui";
const runtimeErrors = new WeakMap<Page, string[]>();
test.beforeEach(async ({ page }) => {
  await installLiveFixture(page);
  const errors: string[] = [];
  runtimeErrors.set(page, errors);
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (message) => {
    if (
      message.type() === "error" &&
      // These fixtures deliberately abort API/image requests and return 503s.
      // Keep browser transport diagnostics separate from application errors.
      !message.text().startsWith("Failed to load resource:")
    )
      errors.push(message.text());
  });
});
test.afterEach(async ({ page }) => {
  expect(runtimeErrors.get(page)).toEqual([]);
});
const fixtureCors = {
  "access-control-allow-origin": `http://127.0.0.1:${process.env.ARGOS_WEB_TEST_PORT || 3100}`,
};
function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
const a = {
  id: "a1",
  run_id: "r1",
  title: "Harbor negotiations enter a new phase",
  description: "Officials meet to discuss regional shipping access.",
  source_name: "Fixture Wire",
  source_domain: "example.test",
  published_at: "2026-10-25T12:00:00Z",
  category: "geopolitical",
  image_url: "",
  url: "https://example.test/news",
  author: "Fixture reporter",
};
async function fixtures(
  page: Page,
  bodyAvailable = true,
  imageUrl = "",
  articleOverrides: Partial<typeof a> = {},
  apiOrigin = `http://127.0.0.1:${process.env.ARGOS_WEB_TEST_PORT || 3100}`,
) {
  const article = { ...a, ...articleOverrides };
  let online = true;
  const mutations: string[] = [];
  const reloads: string[] = [];
  const seeds: unknown[] = [];
  const requests: string[] = [];
  const admissions: { run: string; id: string }[] = [];
  let admissionFails = false;
  await page.route(`${apiOrigin}/api/v1/**`, async (route) => {
    const url = new URL(route.request().url());
    if (!online) {
      await route.abort();
      return;
    }
    const path = url.pathname.replace("/api/v1/", "");
    requests.push(path);
    let body: unknown = {};
    if (path === "health/ready")
      body = {
        service: "argos-intel",
        api_version: 1,
        ready: true,
        storage: true,
        instance_id: "fixture-instance",
      };
    else if (path === "actors/detail")
      body = {
        id: "actor1",
        display_name: "Port authority",
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
    else if (path === "host/metrics")
      body = {
        cpu_percent: null,
        memory_percent: null,
        disk_percent: null,
        network_kbps: null,
      };
    else if (path === "days") body = { days: ["2026-10-25"] };
    else if (path === "articles")
      body = {
        rows:
          url.searchParams.get("search") === "missing"
            ? []
            : [
                { ...article, image_url: imageUrl },
                ...(imageUrl
                  ? [
                      {
                        ...article,
                        id: "a2",
                        title: "Another regional signal",
                        image_url: imageUrl,
                      },
                    ]
                  : []),
              ],
        next_cursor: null,
        revision: "r1",
      };
    else if (path === "articles/detail")
      body = {
        article,
        topology_seed: { Actor: "actor1" },
        inferences: [
          {
            id: "i1",
            text: "Access may improve.",
            topology_seed: { Inference: "inference1" },
          },
        ],
        source_revision: "fixture-source",
        actors: [
          {
            id: "actor1",
            display_name: "Port authority",
            type_hint: "organization",
            topology_seed: { Actor: "actor1" },
          },
        ],
        claims: [
          {
            id: "c1",
            topology_seed: { Claim: "claim1" },
            text: "Shipping talks resumed on Monday.",
            confidence: 0.8,
            admiralty: "B2",
            classification: "fact",
          },
        ],
        body: bodyAvailable
          ? {
              body_markdown:
                "The full retained article describes the negotiations and their regional context.",
            }
          : null,
        lifecycle: "active",
        status: "Published",
        revision: "r2",
        publication: [
          1,
          { claims: [] },
          { inferences: [{ text: "Access may improve." }] },
        ],
        relations: [],
      };
    else if (path === "articles/related")
      body = {
        rows: [
          {
            article: { ...a, id: "a2", title: "Earlier shipping agreement" },
            basis: ["shared actor"],
            tentative: true,
          },
        ],
        next_cursor: null,
      };
    else if (path === "topology/resolve") {
      const seed = route.request().postDataJSON().seed;
      seeds.push(seed);
      body = {
        seed:
          [seed.Actor, seed.Claim, seed.Inference].find(
            (value) => typeof value === "string",
          ) || "brain-memory-fixture",
      };
    } else if (path === "topology") {
      body = {
        root_label: "Port authority",
        nodes: [
          {
            id: "n1",
            label: "Port authority",
            kind: "entity",
            detail: "Recorded actor",
            topology_seed: { Actor: "actor1" },
          },
        ],
        edges: [],
        findings: { gaps: [] },
        has_continuation: false,
      };
    } else if (path === "brief/ensure") {
      mutations.push(path);
      admissions.push(route.request().postDataJSON());
      if (admissionFails) {
        await route.fulfill({
          status: 503,
          json: { error: "Admission temporarily unavailable" },
          headers: {
            "access-control-allow-origin": `http://127.0.0.1:${process.env.ARGOS_WEB_TEST_PORT || 3100}`,
          },
        });
        return;
      }
      body = { job_id: "job1" };
    } else if (path === "brief/reload") {
      mutations.push(path);
      reloads.push(route.request().postDataJSON().request_id);
      body = { job_id: "job1" };
    } else if (route.request().method() === "POST") {
      mutations.push(path);
      body = { job_id: "job1" };
    }
    await route.fulfill({
      json: body,
      headers: {
        "access-control-allow-origin": `http://127.0.0.1:${process.env.ARGOS_WEB_TEST_PORT || 3100}`,
      },
    });
  });
  return {
    offline: () => {
      online = false;
    },
    online: () => {
      online = true;
    },
    setDescription: (value: string) => {
      article.description = value;
    },
    mutations,
    reloads,
    admissions,
    failAdmission: (failed: boolean) => {
      admissionFails = failed;
    },
    seeds,
    requests,
  };
}
test("opening and revisiting a published brief does not queue another run", async ({
  page,
}) => {
  const f = await fixtures(page);
  await gotoBulletin(page);
  await page.getByRole("button", { name: /^Open Focus Brief:/ }).click();
  await expect(
    page.getByRole("button", { name: "Ensure / resume brief" }),
  ).toBeEnabled();
  await expect.poll(() => f.admissions).toEqual([]);
  await expect(
    page.getByRole("button", { name: "Ensure / resume brief" }),
  ).toBeEnabled();
  const reads = () =>
    f.requests.filter((path) => path === "articles/detail").length;
  const initial = reads();
  f.setDescription("Manually refreshed brief");
  await page.getByRole("button", { name: "Refresh retained brief" }).click();
  await expect(
    page.getByText("Manually refreshed brief", { exact: true }),
  ).toBeVisible();
  await expect.poll(reads).toBeGreaterThan(initial);
  const refreshed = reads();
  f.setDescription("Push refreshed brief");
  const live = await installLiveFixture(page);
  live.brief({
    run: "r1",
    id: "a1",
    job_id: null,
    stages: [],
    indexing: null,
    complete: true,
    refresh: true,
  });
  await expect(
    page.getByText("Push refreshed brief", { exact: true }),
  ).toBeVisible();
  await expect.poll(reads).toBeGreaterThan(refreshed);
  expect(f.admissions).toEqual([]);
  await page
    .getByRole("button", {
      name: "Open Focus Brief for related lead: Earlier shipping agreement",
    })
    .click();
  await expect.poll(() => f.admissions).toEqual([]);
  await page.getByRole("button", { name: "Return to Bulletin" }).click();
  await page.getByRole("button", { name: /^Open Focus Brief:/ }).click();
  await expect.poll(() => f.admissions).toEqual([]);
  expect(
    f.requests.filter((path) => /^(jobs|logs|brief\/stages)/.test(path)),
  ).toEqual([]);
});

test("automatic admission errors are visible on narrow Brief and can be retried", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  const f = await fixtures(page);
  f.failAdmission(true);
  await gotoBulletin(page);
  await page.getByRole("button", { name: /^Open Focus Brief:/ }).click();
  await page.getByRole("button", { name: "Ensure / resume brief" }).click();
  await expect(
    page
      .getByRole("alert")
      .filter({ hasText: "Could not start brief extraction" }),
  ).toBeVisible();
  expect(f.admissions).toEqual([{ run: "r1", id: "a1" }]);
  f.failAdmission(false);
  await page.screenshot({
    path: `${captureDir}/brief-admission-error.png`,
    fullPage: true,
  });
  await page.getByRole("button", { name: "Ensure / resume brief" }).click();
  await expect.poll(() => f.admissions.length).toBe(2);
  await expect(
    page
      .getByRole("alert")
      .filter({ hasText: "Could not start brief extraction" }),
  ).toHaveCount(0);
});

for (const width of [1440, 768, 390, 320]) {
  test(`feed thumbnail stays right of article text at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 1100 });
    await page.route("https://images.example.test/story.svg", (route) =>
      route.fulfill({
        contentType: "image/svg+xml",
        body: '<svg xmlns="http://www.w3.org/2000/svg" width="480" height="360"><rect width="480" height="360" fill="#243f50"/><path d="M0 220L160 170 310 225 480 180V360H0" fill="#447783"/><path d="M90 195H370L320 260H135Z" fill="#b4bfc1"/><path d="M205 120H275V195H205Z" fill="#dde5db"/></svg>',
      }),
    );
    await fixtures(page, true, "https://images.example.test/story.svg");
    await gotoBulletin(page);
    const card = page.getByTestId("article-card-a1");
    const image = card.getByRole("img", {
      name: `Article image: ${a.title}`,
      exact: true,
    });
    await expect(image).toBeVisible();
    await expect
      .poll(() => image.evaluate((img: HTMLImageElement) => img.naturalWidth))
      .toBeGreaterThan(0);
    const imageBox = (await image.boundingBox())!;
    const titleBox = (await card
      .getByText(a.title, { exact: true })
      .boundingBox())!;
    const secondary = page.getByTestId("article-card-a2");
    const secondaryImage = secondary.getByRole("img", {
      name: "Article image: Another regional signal",
      exact: true,
    });
    await expect(secondaryImage).toBeVisible();
    const secondaryImageBox = (await secondaryImage.boundingBox())!;
    const secondaryTextBox = (await secondary
      .getByText("Another regional signal", { exact: true })
      .boundingBox())!;
    expect(secondaryTextBox.x + secondaryTextBox.width).toBeLessThan(
      secondaryImageBox.x,
    );
    expect(titleBox.x + titleBox.width).toBeLessThan(imageBox.x);
    expect(titleBox.x + titleBox.width).toBeLessThanOrEqual(width);
    expect(
      await page.evaluate(() => document.documentElement.scrollWidth),
      JSON.stringify(
        await page.evaluate(() =>
          Array.from(document.querySelectorAll("body *"))
            .filter(
              (el) => el.getBoundingClientRect().right > window.innerWidth,
            )
            .map((el) => ({
              tag: el.tagName,
              text: el.textContent?.slice(0, 60),
              right: el.getBoundingClientRect().right,
            }))
            .slice(-15),
        ),
      ),
    ).toBeLessThanOrEqual(width);
    await expect(card).toHaveRole("button");
    await expect(card).toHaveAccessibleName(`Open Focus Brief: ${a.title}`);
    if (width === 1440 || width === 320)
      await page.screenshot({
        path: `${captureDir}/web-feed-${width}.png`,
        fullPage: true,
      });
    await card.click();
    await expect(
      page.getByRole("button", { name: "Return to Bulletin" }),
    ).toBeVisible();
  });
}

for (const [name, imageUrl] of [
  ["missing", ""],
  ["unsafe", "javascript:alert(1)"],
  ["failed", "https://images.example.test/broken.png"],
]) {
  test(`feed images collapse for ${name} sources`, async ({ page }) => {
    await page.route("https://images.example.test/broken.png", (route) =>
      route.abort(),
    );
    await fixtures(page, true, imageUrl);
    await gotoBulletin(page);
    const card = page.getByTestId("article-card-a1");
    await expect(card).toBeVisible();
    await expect(card.getByRole("img")).toHaveCount(0);
    await expect(card.getByText(a.title, { exact: true })).toBeVisible();
    await expect(card).toHaveRole("button");
    await expect(card).toHaveAccessibleName(`Open Focus Brief: ${a.title}`);
  });
}

test("gates the workspace when the API is unavailable", async ({ page }) => {
  await page.route("**/api/v1/**", (r) => r.abort());
  await page.goto("/");
  await expect(
    page.getByRole("status", {
      name: /Local API unavailable|Waiting for workspace readiness/,
    }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Intelligence bulletin", exact: true }),
  ).toHaveCount(0);
  await expect(page.getByRole("textbox", { name: "API endpoint" })).toHaveCount(
    0,
  );
  await page.getByRole("button", { name: "Retry connection checks" }).click();
  await page.screenshot({
    path: `${captureDir}/connection-error.png`,
    fullPage: true,
  });
});
test("browse, search, brief, related, topology and reconnect", async ({
  page,
}) => {
  test.setTimeout(90000);
  await page.clock.install();
  const f = await fixtures(page);
  await gotoBulletin(page);
  await expect(
    page.getByRole("heading", { name: "Intelligence bulletin", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Jobs", exact: true }),
  ).toHaveCount(0);
  for (const cat of [
    "Geopolitical",
    "Economic",
    "Military",
    "Information",
    "Stability",
    "Technology",
  ])
    await expect(
      page.getByRole("button", { name: cat, exact: true }),
    ).toBeVisible();
  await page.getByRole("textbox", { name: "Search articles" }).fill("missing");
  await expect(
    page.getByText("No articles match this day, category and search."),
  ).toBeVisible();
  await page.screenshot({
    path: `${captureDir}/empty-search.png`,
    fullPage: true,
  });
  await page.getByRole("textbox", { name: "Search articles" }).fill("");
  await page.getByRole("button", { name: /^Open Focus Brief:/ }).click();
  await expect.poll(() => f.admissions).toEqual([]);
  await expect(
    page
      .locator("#intel-brief-reader")
      .getByText("Officials meet to discuss regional shipping access."),
  ).toBeVisible();
  await expect(
    page.getByText(
      "The full retained article describes the negotiations and their regional context.",
    ),
  ).toHaveCount(0);
  await expect(
    page.getByText("Related leads · not corroboration", { exact: true }),
  ).toBeVisible();
  await page.screenshot({
    path: `${captureDir}/web-focus-wide.png`,
    fullPage: true,
  });
  await page.getByRole("button", { name: "Ensure / resume brief" }).click();
  await expect.poll(() => f.admissions.length).toBe(1);
  await page
    .getByRole("button", { name: "Reload article / new generation" })
    .click();
  await expect.poll(() => f.mutations).toContain("brief/reload");
  expect(f.reloads[0]).toMatch(
    /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
  );
  await page
    .getByRole("button", { name: "Explore actor Port authority" })
    .click();
  await expect(page).toHaveURL(/\/actor\?id=actor1$/);
  await page.getByRole("button", { name: "OPEN ACTOR TOPOLOGY" }).click();
  await expect(
    page.getByRole("button", {
      name: "Select node Port authority, entity",
      exact: true,
    }),
  ).toBeVisible();
  await expect(page).toHaveURL(/\/topology\?seed=actor1$/);
  await page
    .getByRole("button", { name: "Return to Actor", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Return to Brief", exact: true })
    .click();
  await page.getByRole("button", { name: "Launch 3D topology" }).click();
  await expect(page).toHaveURL(/\/topology\?seed=actor1$/);
  await expect(
    page.getByRole("button", {
      name: "Select node Port authority, entity",
      exact: true,
    }),
  ).toBeVisible();
  f.offline();
  await page.clock.fastForward(60_000);
  await expect(
    page.getByRole("alert").filter({ hasText: "Failed to fetch" }),
  ).toBeVisible();
  await expect(page.getByLabel("API offline", { exact: true })).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Jobs", exact: true }),
  ).toHaveCount(0);
  f.online();
  await page.getByRole("button", { name: "Connect / retry" }).click();
  await expect(
    page.getByRole("button", {
      name: "Select node Port authority, entity",
      exact: true,
    }),
  ).toBeVisible();
  await expect(page).toHaveURL(/\/topology\?seed=actor1$/);
  await page.getByRole("button", { name: "Bulletin", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Intelligence bulletin", exact: true }),
  ).toBeVisible();
  expect(
    f.requests.filter((path) =>
      /^(jobs|logs)(\/|$)|^brief\/stages$/.test(path),
    ),
  ).toEqual([]);
  await page.screenshot({
    path: `${captureDir}/web-wide.png`,
    fullPage: true,
  });
});
test("narrow Focus Brief evidence and static hosting policy", async ({
  page,
  request,
}) => {
  await page.setViewportSize({ width: 320, height: 844 });
  await fixtures(page);
  await gotoBulletin(page);
  await page.getByRole("button", { name: /^Open Focus Brief:/ }).click();
  await expect(
    page.getByRole("heading", { name: "IDENTIFIED ACTORS", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Shipping talks resumed on Monday."),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Ensure / resume brief" }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: `${captureDir}/web-narrow.png`,
    fullPage: true,
  });
  const response = await request.get("/");
  expect(response.status()).toBe(200);
  if (process.env.ARGOS_WEB_PROFILE !== "dev") {
    expect(response.headers()["content-security-policy"]).toContain(
      "frame-ancestors 'none'",
    );
    expect(response.headers()["x-content-type-options"]).toBe("nosniff");
  }
});

test("metadata excerpt is not presented as a retained full body", async ({
  page,
}) => {
  await fixtures(page, false);
  await gotoBulletin(page);
  await page.getByRole("button", { name: /^Open Focus Brief:/ }).click();
  await expect(
    page
      .locator("#intel-brief-reader")
      .getByText("Officials meet to discuss regional shipping access."),
  ).toBeVisible();
  await expect(page.getByText(/Full article body/)).toHaveCount(0);
});

test("selection controls support keyboard activation and visible focus", async ({
  page,
}) => {
  await fixtures(page);
  await gotoBulletin(page);
  const economic = page.getByRole("button", { name: "Economic", exact: true });
  await economic.focus();
  await economic.press("Enter");
  await expect(economic).toHaveAttribute("aria-pressed", "true");
  await expect(
    page.getByRole("button", { name: "Geopolitical", exact: true }),
  ).toHaveAttribute("aria-pressed", "false");
  await expect(
    page.getByRole("button", { name: /^Open Focus Brief:/ }),
  ).toBeVisible();
  const search = page.getByRole("textbox", { name: "Search articles" });
  await search.focus();
  await search.press("Tab");
  // Header search precedes the category rail in the reference layout and DOM.
  // Tab visits controls in that order; it must not jump over the rail.
  for (const name of [
    "Geopolitical",
    "Economic",
    "Military",
    "Information",
    "Stability",
    "Technology",
  ]) {
    const category = page.getByRole("button", { name, exact: true });
    await expect(category).toBeFocused();
    expect(
      await category.evaluate((el) => getComputedStyle(el).outlineStyle),
    ).toBe("solid");
    await category.press("Tab");
  }
  const day = page.getByRole("button", { name: /Retained day 2026-10-25/ });
  await expect(day).toBeFocused();
  expect(await day.evaluate((el) => getComputedStyle(el).outlineStyle)).toBe(
    "solid",
  );
  await day.press("Tab");
  for (const name of ["Latest", "Hottest"]) {
    const sort = page.getByRole("button", { name, exact: true });
    await expect(sort).toBeFocused();
    expect(await sort.evaluate((el) => getComputedStyle(el).outlineStyle)).toBe(
      "solid",
    );
    await sort.press("Tab");
  }
  const refresh = page.getByRole("button", {
    name: "Refresh feed",
    exact: true,
  });
  await expect(refresh).toBeFocused();
  await refresh.press("Tab");
  const open = page.getByRole("button", { name: /^Open Focus Brief:/ });
  await expect(open).toBeFocused();
  await page.screenshot({
    path: `${captureDir}/keyboard-focus.png`,
    fullPage: true,
  });
  await open.press("Space");
  await expect(
    page.getByRole("heading", { name: "IDENTIFIED ACTORS" }),
  ).toBeVisible();
});

for (const width of [1440, 768, 390, 320]) {
  test(`workspace wraps long content and preserves all brief sections at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 1000 });
    await fixtures(page, true, "", {
      title:
        "Regional shipping negotiations resume as port authorities outline a longer-term agreement on access and security",
      source_name: "International Maritime and Regional Affairs Review",
      description:
        "Officials met to discuss regional shipping access. The agreement remains under review, with further talks planned between port authorities and local representatives. Reported commitments have not yet been independently verified.",
    });
    await gotoBulletin(page);
    const noOverflow = async () =>
      expect(
        await page.evaluate(() => document.documentElement.scrollWidth),
      ).toBeLessThanOrEqual(width);
    await expect(page.getByTestId("article-card-a1")).toBeVisible();
    await noOverflow();
    await page.screenshot({
      path: `${captureDir}/bulletin-${width}.png`,
      fullPage: true,
    });
    await page.getByRole("button", { name: /^Open Focus Brief:/ }).click();
    for (const title of [
      "IDENTIFIED ACTORS",
      "RETAINED CLAIMS",
      "ANALYTICAL INFERENCES",
      "RETAINED BRIEF CONTROLS",
    ])
      await expect(
        page.getByRole("heading", { name: title, exact: true }),
      ).toBeVisible();
    await noOverflow();
    await page.screenshot({
      path: `${captureDir}/brief-${width}.png`,
      fullPage: true,
    });
    await page.getByRole("button", { name: "Launch 3D topology" }).click();
    await expect(
      page.getByRole("button", {
        name: "Select node Port authority, entity",
        exact: true,
      }),
    ).toBeVisible();
    await expect(page.locator("canvas").first()).toBeVisible({
      timeout: 20000,
    });
    await page.screenshot({
      path: `${captureDir}/topology-3d-1280.png`,
      fullPage: true,
    });
    await page.waitForTimeout(800);
    await noOverflow();
    await page.screenshot({
      path: `${captureDir}/topology-${width}.png`,
      fullPage: true,
    });
  });
}

test("browser routes use only Next API and discard legacy Rust endpoint settings", async ({
  page,
}) => {
  await page.addInitScript(() =>
    localStorage.setItem("argos.backend", "http://127.0.0.1:8099"),
  );
  const state = await fixtures(page);
  const requests: string[] = [];
  page.on("request", (request) => {
    if (request.url().includes("/api/v1/")) requests.push(request.url());
  });
  await gotoBulletin(page, "/?api=http://127.0.0.1:8099");
  await expect
    .poll(() => new URL(page.url()).searchParams.has("api"))
    .toBe(false);
  expect(state.requests).toContain("health/ready");
  expect(
    requests.every((url) => new URL(url).origin === new URL(page.url()).origin),
  ).toBe(true);
  expect(
    await page.evaluate(() => localStorage.getItem("argos.backend")),
  ).toBeNull();
});

test("blocked browser storage still permits the server-managed workspace", async ({
  page,
}) => {
  await page.addInitScript(() => {
    Storage.prototype.setItem = () => {
      throw new DOMException("blocked", "SecurityError");
    };
    Storage.prototype.getItem = () => {
      throw new DOMException("blocked", "SecurityError");
    };
  });
  await fixtures(page);
  await gotoBulletin(page);
  await expect(
    page.getByRole("heading", { name: "Intelligence bulletin" }),
  ).toBeVisible();
});

test("loading days and a verified empty cycle are distinct states", async ({
  page,
}) => {
  await fixtures(page);
  const days = deferred();
  await page.route("**/api/v1/days", async (route) => {
    await days.promise;
    await route.fulfill({ json: { days: [] }, headers: fixtureCors });
  });
  await gotoBulletin(page);
  await expect(
    page.getByRole("status", {
      name: "Loading bulletin…",
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "No articles yet" }),
  ).toHaveCount(0);
  await page.screenshot({
    path: `${captureDir}/days-loading.png`,
    fullPage: true,
  });
  days.resolve();
  await expect(
    page.getByRole("heading", { name: "No articles yet" }),
  ).toBeVisible();
  await expect(page.getByRole("progressbar")).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: /^Open Focus Brief:/ }),
  ).toHaveCount(0);
  await page.screenshot({
    path: `${captureDir}/days-empty.png`,
    fullPage: true,
  });
});

test("pending articles never show a false empty result", async ({ page }) => {
  await fixtures(page);
  const articles = deferred();
  await page.route(/\/api\/v1\/articles\?/, async (route) => {
    await articles.promise;
    await route.fulfill({
      json: { rows: [], next_cursor: null, revision: "empty" },
      headers: fixtureCors,
    });
  });
  await gotoBulletin(page);
  await expect(
    page.getByRole("status", { name: "Loading bulletin…", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("No articles match this day, category and search."),
  ).toHaveCount(0);
  await page.screenshot({
    path: `${captureDir}/articles-loading.png`,
    fullPage: true,
  });
  articles.resolve();
  await expect(
    page.getByRole("heading", { name: "NO MATCHING INTELLIGENCE" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: /^Open Focus Brief:/ }),
  ).toHaveCount(0);
});

test("brief and related loading states preserve numeric zero and verified empty evidence", async ({
  page,
}) => {
  await fixtures(page);
  const detail = deferred(),
    leads = deferred();
  await page.route(/\/api\/v1\/articles\/detail\?/, async (route) => {
    await detail.promise;
    await route.fulfill({
      json: {
        article: a,
        actors: [],
        claims: [],
        confidence: { score: 0, provisional: false },
        lifecycle: "active",
        status: "Published",
        publication: null,
        links: [],
        revision: "zero",
      },
      headers: fixtureCors,
    });
  });
  await page.route(/\/api\/v1\/articles\/related\?/, async (route) => {
    await leads.promise;
    await route.fulfill({
      json: { rows: [], next_cursor: null },
      headers: fixtureCors,
    });
  });
  await gotoBulletin(page);
  await page.getByRole("button", { name: /^Open Focus Brief:/ }).click();
  await expect(
    page.getByRole("status", { name: "Loading extracted evidence…" }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "No published claims" }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("progressbar", { name: "Loading related leads…" }),
  ).toBeVisible();
  await page.screenshot({
    path: `${captureDir}/brief-loading.png`,
    fullPage: true,
  });
  detail.resolve();
  leads.resolve();
  await expect(page.getByText("0%", { exact: true })).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "No actors extracted" }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "No published claims" }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "No related leads" }),
  ).toBeVisible();
  await page.screenshot({
    path: `${captureDir}/brief-empty-zero.png`,
    fullPage: true,
  });
});

for (const changeCategory of [false, true]) {
  test(`scroll pagination is guarded and ${changeCategory ? "isolates a changed category" : "deduplicates occurrences"}`, async ({
    page,
  }) => {
    await fixtures(page);
    const next = deferred();
    const cursors: string[] = [];
    await page.route(/\/api\/v1\/articles\?/, async (route) => {
      const url = new URL(route.request().url());
      const category = url.searchParams.get("category")!;
      const cursor = url.searchParams.get("cursor") || "";
      cursors.push(`${category}:${cursor}`);
      if (category === "economic") {
        await route.fulfill({
          json: {
            rows: [
              { ...a, id: "economic", category, title: "Economic signal" },
            ],
            next_cursor: null,
            revision: "economic",
          },
          headers: fixtureCors,
        });
      } else if (!cursor) {
        await route.fulfill({
          json: {
            rows: Array.from({ length: 12 }, (_, i) => ({
              ...a,
              id: `page-${i}`,
              title: `Signal ${i}`,
            })),
            next_cursor: "opaque-next",
            revision: "page-one",
          },
          headers: fixtureCors,
        });
      } else {
        await next.promise;
        await route.fulfill({
          json: {
            rows: [11, 12].map((i) => ({
              ...a,
              id: `page-${i}`,
              title: `Signal ${i}`,
            })),
            next_cursor: null,
            revision: "page-two",
          },
          headers: fixtureCors,
        });
      }
    });
    await gotoBulletin(page);
    await expect(page.getByTestId("article-card-page-0")).toBeVisible();
    await expect(page.getByTestId("article-card-page-11")).toBeAttached();
    const scroller = page.getByTestId("article-list-scroll");
    await scroller.evaluate((el) => {
      el.scrollTop = el.scrollHeight;
    });
    await expect
      .poll(
        () => cursors.filter((c) => c === "geopolitical:opaque-next").length,
      )
      .toBe(1);
    await expect(
      page.getByRole("progressbar", {
        name: "Loading more articles…",
        exact: true,
      }),
    ).toBeVisible();
    for (let i = 0; i < 5; i++)
      await scroller.evaluate((el) => el.dispatchEvent(new Event("scroll")));
    expect(
      cursors.filter((c) => c === "geopolitical:opaque-next"),
    ).toHaveLength(1);
    await page.screenshot({
      path: `${captureDir}/next-page-loading-${changeCategory}.png`,
      fullPage: true,
    });
    if (changeCategory) {
      await page.getByRole("button", { name: "Economic", exact: true }).click();
      await expect(
        page.getByText("Economic signal", { exact: true }),
      ).toBeVisible();
    }
    next.resolve();
    if (changeCategory) {
      await expect(page.getByTestId("article-card-page-12")).toHaveCount(0);
      await expect(
        page.getByText("Economic signal", { exact: true }),
      ).toBeVisible();
    } else {
      await expect(page.getByTestId("article-card-page-12")).toBeVisible();
      await expect(page.getByTestId("article-card-page-11")).toHaveCount(1);
      await expect(
        page.getByRole("button", { name: /^Open Focus Brief:/ }),
      ).toHaveCount(13);
      await expect(
        page.getByRole("button", { name: "Load more articles" }),
      ).toHaveCount(0);
    }
  });
}

test("background refresh keeps readable bulletin rows", async ({ page }) => {
  await page.clock.install();
  await fixtures(page);
  const refresh = deferred();
  let reads = 0;
  await page.route(/\/api\/v1\/articles\?/, async (route) => {
    if (++reads > 1) await refresh.promise;
    await route.fulfill({
      json: {
        rows: [{ ...a, title: reads > 1 ? "Refreshed signal" : a.title }],
        next_cursor: null,
        revision: String(reads),
      },
      headers: fixtureCors,
    });
  });
  await gotoBulletin(page);
  await expect(page.getByText(a.title, { exact: true })).toBeVisible();
  await page.clock.fastForward(60_000);
  await expect.poll(() => reads).toBeGreaterThan(1);
  await expect(
    page.getByRole("progressbar", {
      name: "Refreshing bulletin…",
      exact: true,
    }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("status", { name: "Loading bulletin…", exact: true }),
  ).toHaveCount(0);
  await expect(page.getByText(a.title, { exact: true })).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "NO MATCHING INTELLIGENCE" }),
  ).toHaveCount(0);
  refresh.resolve();
  await expect(
    page.getByText("Refreshed signal", { exact: true }),
  ).toBeVisible();
});

test("topology distinguishes no selection, loading and an empty graph", async ({
  page,
}) => {
  await fixtures(page);
  const graph = deferred();
  await page.route("**/api/v1/topology?**", async (route) => {
    await graph.promise;
    await route.fulfill({
      json: {
        root_label: "Empty source",
        nodes: [],
        edges: [],
        findings: { gaps: [] },
        has_continuation: false,
      },
      headers: fixtureCors,
    });
  });
  await gotoBulletin(page);
  await page.getByRole("button", { name: /^Open Focus Brief:/ }).click();
  await page.getByRole("button", { name: "Launch 3D topology" }).click();
  await expect(
    page.getByRole("status", { name: "Loading topology…", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "No retained connections" }),
  ).toHaveCount(0);
  graph.resolve();
  await expect(
    page.getByRole("heading", { name: "No retained connections" }),
  ).toBeVisible();
});

test("failed scroll pages retain rows and require an explicit retry", async ({
  page,
}) => {
  await fixtures(page);
  let requests = 0;
  await page.route(/\/api\/v1\/articles\?/, async (route) => {
    const cursor = new URL(route.request().url()).searchParams.get("cursor");
    if (cursor && ++requests <= 2) {
      await route.fulfill({
        status: 503,
        json: { error: "Page unavailable" },
        headers: fixtureCors,
      });
      return;
    }
    await route.fulfill({
      json: {
        rows: cursor
          ? [{ ...a, id: "recovered", title: "Recovered page" }]
          : Array.from({ length: 12 }, (_, i) => ({
              ...a,
              id: `retry-${i}`,
              title: `Retry signal ${i}`,
            })),
        next_cursor: cursor ? null : "retry-cursor",
        revision: cursor ? "retry-two" : "retry-one",
      },
      headers: fixtureCors,
    });
  });
  await gotoBulletin(page);
  await expect(page.getByTestId("article-card-retry-0")).toBeVisible();
  await expect(page.getByTestId("article-card-retry-11")).toBeAttached();
  const scroller = page.getByTestId("article-list-scroll");
  await scroller.evaluate((el) => {
    el.scrollTop = el.scrollHeight;
  });
  await expect(page.getByText(/Could not load more articles/)).toBeVisible();
  await expect(page.getByTestId("article-card-retry-11")).toBeVisible();
  for (let i = 0; i < 5; i++)
    await scroller.evaluate((el) => el.dispatchEvent(new Event("scroll")));
  expect(requests).toBe(2);
  await page.getByRole("button", { name: "Retry loading articles" }).click();
  await expect(page.getByTestId("article-card-recovered")).toBeVisible();
  expect(requests).toBe(3);
  await expect(
    page.getByRole("button", { name: /^Open Focus Brief:/ }),
  ).toHaveCount(13);
});

for (const width of [1440, 390]) {
  test(`inferences are the last evidence section at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 1000 });
    await fixtures(page);
    await gotoBulletin(page);
    await page.getByRole("button", { name: /^Open Focus Brief:/ }).click();
    const section = page.getByTestId("reader-inferences");
    await expect(section.getByText("Access may improve.")).toBeVisible();
    await expect(section).toBeVisible();
    const headings = await page
      .locator("#intel-brief [role=heading], #intel-brief h1, #intel-brief h2")
      .allTextContents();
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
    const reader = (await page.locator("#intel-brief-reader").boundingBox())!;
    const context = (await page.locator("#intel-context-rail").boundingBox())!;
    if (width < 900)
      expect(context.y).toBeGreaterThanOrEqual(reader.y + reader.height - 1);
    else expect(context.x).toBeGreaterThanOrEqual(reader.x + reader.width - 1);
    await expect(
      page.getByRole("heading", { name: "RETAINED CLAIMS", exact: true }),
    ).toBeVisible();
    await page.screenshot({
      path: `${captureDir}/inferences-reader-${width}.png`,
      fullPage: true,
    });
  });
}

test("topology graph explorer selects, filters and stays within 320px", async ({
  page,
}) => {
  test.setTimeout(60000);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await fixtures(page);
  await page.route("**/api/v1/topology?**", async (route) => {
    await route.fulfill({
      json: {
        root_label: "Harbor",
        nodes: [
          {
            id: "n1",
            label: "Port authority",
            kind: "entity",
            detail: "Recorded actor",
            topology_seed: { Actor: "actor1" },
          },
          {
            id: "n2",
            label: "Shipping claim",
            kind: "claim",
            detail: "Claim detail",
          },
          {
            id: "n3",
            label: "Harbor article",
            kind: "article",
            detail: "Article detail",
          },
        ],
        edges: [
          { id: "e1", from: "n1", to: "n2", label: "asserts" },
          { id: "e2", from: "n3", to: "n2", label: "supports" },
        ],
        findings: { gaps: [] },
        has_continuation: false,
      },
      headers: fixtureCors,
    });
  });
  await page.setViewportSize({ width: 1280, height: 900 });
  await gotoBulletin(page);
  await page.getByRole("button", { name: "Bulletin", exact: true }).click();
  await page.getByRole("button", { name: /^Open Focus Brief:/ }).click();
  await page.getByRole("button", { name: "Launch 3D topology" }).click();
  await expect(
    page.getByRole("button", { name: "Launch 3D topology" }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Return to Brief" }),
  ).toBeVisible();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  for (const name of ["Zoom in", "Zoom out", "Pause layout", "Graph filters"])
    await expect(page.getByRole("button", { name, exact: true })).toHaveCount(
      0,
    );
  await expect(page.getByText("Showing 2 of 3 nodes")).toBeVisible();
  await expect(
    page.getByRole("button", {
      name: "Select node Port authority, entity",
      exact: true,
    }),
  ).toBeVisible();
  await expect(page.locator("canvas").first()).toBeVisible({ timeout: 20000 });
  await page.locator("canvas").first().scrollIntoViewIfNeeded();
  await page
    .locator("canvas")
    .first()
    .screenshot({ path: `${captureDir}/topology-3d-canvas-1280.png` });
  await page
    .getByRole("button", { name: "Select node Shipping claim, claim" })
    .click();
  await expect(
    page.getByRole("button", {
      name: "Select node Shipping claim, claim",
      exact: true,
    }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(
    page.getByRole("button", {
      name: "Select node Port authority, entity",
      exact: true,
    }),
  ).toHaveAttribute("aria-pressed", "false");
  await expect(
    page
      .getByRole("list", { name: "Topology nodes", exact: true })
      .getByRole("button"),
  ).toHaveCount(3);
  await page.getByRole("button", { name: "Show article nodes" }).click();
  await expect(page.getByText("Showing 2 of 3 nodes")).toBeVisible();
  const graphCanvas = page.locator("canvas").first();
  const canvasBounds = (await graphCanvas.boundingBox())!;
  const emptyCorner = { x: canvasBounds.width - 12, y: 12 };
  await graphCanvas.hover({ position: emptyCorner });
  // The engine raycasts pointer movement on its render loop before click dispatch.
  await page.evaluate(
    () =>
      new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      ),
  );
  await graphCanvas.click({ position: emptyCorner });
  await expect(
    page.getByRole("group", { name: /^Topology graph with/ }),
  ).toHaveAccessibleName(/No node selected/);
  await page.getByRole("button", { name: "Show claim nodes" }).click();
  await page.getByRole("button", { name: "Show entity nodes" }).click();
  await expect(
    page.getByRole("heading", { name: "No nodes match the filters" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Reset filters" }).first().click();
  await expect(page.getByText("Showing 3 of 3 nodes")).toBeVisible();
  await page.setViewportSize({ width: 320, height: 800 });
  // Reproduce the engine's stale pre-resize inline dimensions synchronously,
  // rather than waiting for RN onLayout to repair them before measuring.
  const staleGraphBounds = await page.evaluate(() => {
    const scene = document.querySelector<HTMLElement>(
      "#intel-topology-graph .scene-container",
    )!;
    const canvas = scene.querySelector("canvas")!;
    const oldSceneWidth = scene.style.width;
    const oldCanvasWidth = canvas.style.width;
    try {
      scene.style.width = "648px";
      canvas.style.width = "648px";
      const sceneBox = scene.getBoundingClientRect();
      const canvasBox = canvas.getBoundingClientRect();
      const parentBox = scene.parentElement!.getBoundingClientRect();
      const viewportBox = document
        .querySelector("#intel-topology-viewport")!
        .getBoundingClientRect();
      return {
        sceneFits: sceneBox.width <= parentBox.width,
        canvasFits: canvasBox.width <= sceneBox.width,
        wrapperFits: parentBox.width <= viewportBox.width,
      };
    } finally {
      scene.style.width = oldSceneWidth;
      canvas.style.width = oldCanvasWidth;
    }
  });
  expect(staleGraphBounds).toEqual({
    sceneFits: true,
    canvasFits: true,
    wrapperFits: true,
  });
  await expect(page.getByText("Showing 3 of 3 nodes")).toBeVisible();
  // Graph display boxes must fit synchronously above. Document scroll extent
  // also includes browser/RN resize work, so assert its settled state with a
  // retrying condition, not a one-shot read during that asynchronous work.
  // A persistent overflow never satisfies this exact-zero condition.
  await expect
    .poll(
      () =>
        page.evaluate(
          () => document.documentElement.scrollWidth - window.innerWidth,
        ),
      {
        message: "320px topology document settles without horizontal overflow",
      },
    )
    .toBe(0);
  // Confirm zero survives subsequent rendered frames, rather than accepting
  // just a single successful poll while resize/layout work is still pending.
  const renderedOverflow = await page.evaluate(async () => {
    const samples: number[] = [];
    for (let frame = 0; frame < 3; frame++) {
      await new Promise<void>((resolve) =>
        requestAnimationFrame(() => resolve()),
      );
      samples.push(document.documentElement.scrollWidth - innerWidth);
    }
    return samples;
  });
  expect(renderedOverflow).toEqual([0, 0, 0]);
  await page.getByRole("button", { name: "Return to Brief" }).click();
  await expect(
    page.getByRole("heading", { name: a.title, exact: true }),
  ).toBeVisible();
  await expect(page.locator("#intel-topology-viewport")).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Launch 3D topology" }),
  ).toBeVisible();
});

test("desktop launcher returns to the retained intelligence bulletin", async ({
  page,
}) => {
  await fixtures(page);
  await gotoBulletin(page);
  await expect(page.getByText(a.title)).toBeVisible();
  await page.getByRole("button", { name: "server", exact: true }).click();
  await page.getByRole("button", { name: "Open Intel", exact: true }).click();
  await expect(page.getByText(a.title)).toBeVisible();
  await page.getByRole("textbox", { name: "Search articles" }).fill("missing");
  await expect(page.getByText(a.title)).toHaveCount(0);
});

test("global All follows one server cursor stream and retains rows across a failed scroll page", async ({
  page,
}) => {
  // This case exercises scrolling, not a sidebar-height dependency. A compact
  // sidebar may let two short records fit at the default 1000px viewport.
  await page.setViewportSize({ width: 1440, height: 400 });
  await fixtures(page);
  const first = {
    ...a,
    id: "z",
    title: "Server first global occurrence",
    category: "military",
  };
  const second = {
    ...a,
    id: "a",
    title: "Server second global occurrence",
    category: "economic",
  };
  const third = {
    ...second,
    run_id: "r2",
    title: "Same global article in another run",
    published_at: "2026-10-24T12:00:00Z",
  };
  const cursor = JSON.stringify([
    second.published_at,
    "2026-10-25T14:00:00Z",
    second.id,
    second.run_id,
  ]);
  const reads: { category: string | null; cursor: string | null }[] = [];
  let failNext = true;
  await page.route("**/api/v1/articles**", async (route) => {
    const url = new URL(route.request().url());
    if (!url.pathname.endsWith("/articles")) {
      await route.fallback();
      return;
    }
    const category = url.searchParams.get("category");
    const next = url.searchParams.get("cursor");
    reads.push({ category, cursor: next });
    if (category !== "geopolitical") {
      await route.fulfill({
        json: { rows: [a], next_cursor: null, revision: "specific" },
        headers: fixtureCors,
      });
      return;
    }
    if (next && failNext) {
      await route.fulfill({
        status: 503,
        json: { error: "Global page unavailable" },
        headers: fixtureCors,
      });
      return;
    }
    await route.fulfill({
      json: {
        rows: next ? [second, third] : [first, second],
        next_cursor: next ? null : cursor,
        revision: "global",
      },
      headers: fixtureCors,
    });
  });
  await gotoBulletin(page);
  await expect(page.getByText(first.title)).toBeVisible();
  await expect(
    page.getByRole("button", { name: /^Open Focus Brief:/ }),
  ).toHaveCount(2);
  const cards = page.getByRole("button", { name: /^Open Focus Brief:/ });
  await expect(cards).toHaveCount(2);
  expect(
    await cards.evaluateAll((elements) =>
      elements.map((element) => element.getAttribute("aria-label")),
    ),
  ).toEqual([first, second].map((row) => `Open Focus Brief: ${row.title}`));
  expect(
    await page
      .getByTestId("article-list-scroll")
      .evaluate((element) => element.scrollHeight - element.clientHeight),
  ).toBeGreaterThan(0);
  await page.getByTestId("article-list-scroll").evaluate((element) => {
    element.scrollTop = element.scrollHeight;
  });
  const retry = page.getByRole("button", {
    name: "Retry loading articles",
    exact: true,
  });
  await expect(retry).toBeVisible();
  await expect(
    page.getByRole("alert").filter({ hasText: "Global page unavailable" }),
  ).toBeVisible();
  await expect(cards).toHaveCount(2);
  await expect(
    page.getByRole("heading", { name: "NO MATCHING INTELLIGENCE" }),
  ).toHaveCount(0);
  failNext = false;
  await retry.click();
  await expect(
    page.getByRole("button", { name: /^Open Focus Brief:/ }),
  ).toHaveCount(3);
  await expect(cards).toHaveCount(3);
  expect(
    await cards.evaluateAll((elements) =>
      elements.map((element) => element.getAttribute("aria-label")),
    ),
  ).toEqual(
    [first, second, third].map((row) => `Open Focus Brief: ${row.title}`),
  );
  expect(
    reads.some(
      (read) => read.category === "geopolitical" && read.cursor === null,
    ),
  ).toBe(true);
  expect(
    reads
      .filter(
        (read) => read.category === "geopolitical" && read.cursor !== null,
      )
      .every((read) => read.cursor === cursor),
  ).toBe(true);
  await expect(
    page.getByRole("button", { name: "Load more articles", exact: true }),
  ).toHaveCount(0);
});

for (const width of [1440, 320]) {
  test(`offline shell hydrates without recovery at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.route("**/api/v1/health/ready", (route) =>
      route.fulfill({
        json: {
          ready: false,
          storage: false,
          service: "argos-intel",
          api_version: 1,
          instance_id: "offline-hydration",
        },
        headers: fixtureCors,
      }),
    );
    await page.goto("/");
    await expect(
      page.getByRole("status", {
        name: "Waiting for workspace readiness",
        exact: true,
      }),
    ).toBeVisible();
    await expect(page.getByLabel("API offline", { exact: true })).toHaveCount(
      0,
    );
    await expectFixedStatusBar(page);
    await expect(
      page.getByRole("button", { name: "Retry connection checks" }),
    ).toBeEnabled();
    if (width >= 1100)
      await expect(page.locator("#intel-status-metric-cpu")).toHaveText(
        "CPU —",
      );
    else await expect(page.locator("#intel-status-metric-cpu")).toHaveCount(0);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth - innerWidth,
      ),
    ).toBeLessThanOrEqual(0);
    // The suite's pageerror/console gate must remain empty, including #418.
  });
}

for (const width of [1536, 1440, 768, 390, 320]) {
  test(`ready entry is Home with keyboard launch and return at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 1000 });
    await fixtures(page);
    await page.goto("/");
    await expect(
      page.getByRole("button", { name: "Open Intel", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByLabel("API connected", { exact: true }),
    ).toBeVisible();
    await expect(page.getByText(a.title)).toHaveCount(0);
    const launcher = page.getByRole("button", {
      name: "Open Intel",
      exact: true,
    });
    await expect(launcher).toHaveCount(1);
    await expect(launcher).toBeVisible();
    await expectFixedStatusBar(page);
    const frame = (await page.locator("#intel-content-frame").boundingBox())!;
    await expect(page.getByLabel("API connected", { exact: true })).toHaveCSS(
      "font-size",
      width >= 600 ? "18px" : "13px",
    );
    expect(frame.width).toBeLessThanOrEqual(1440);
    if (width === 1536) expect(frame.width).toBe(1440);
    const launcherBox = (await launcher.boundingBox())!;
    expect(
      Math.abs(launcherBox.x + launcherBox.width / 2 - width / 2),
    ).toBeLessThanOrEqual(2);
    expect(
      Math.abs(launcherBox.y + launcherBox.height / 2 - (1000 - 64) / 2),
    ).toBeLessThanOrEqual(8);
    await expect(
      page.getByRole("button", { name: "Bulletin", exact: true }),
    ).toHaveCount(0);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: `${captureDir}/web-home-${width}.png`,
      fullPage: true,
    });
    if (width === 1536) {
      const globe = page.getByTestId("intel-globe");
      expect(
        await globe.evaluate((element) => getComputedStyle(element).filter),
      ).toContain("18px");
      await launcher.hover();
      await expect
        .poll(() =>
          globe.evaluate((element) => getComputedStyle(element).filter),
        )
        .toContain("24px");
      await page.mouse.move(0, 0);
    }
    await launcher.focus();
    await expect
      .poll(() =>
        page
          .getByTestId("intel-globe")
          .evaluate((element) => getComputedStyle(element).filter),
      )
      .toContain("24px");
    await page.keyboard.press("Enter");
    await expect(
      page.getByRole("heading", { name: "Intelligence bulletin" }),
    ).toBeVisible();
    await expect(page.getByText(a.title)).toBeVisible();
    await expectFixedStatusBar(page);
    await page.getByRole("textbox", { name: "Search articles" }).fill("harbor");
    await page.getByRole("button", { name: "server", exact: true }).click();
    const returning = page.getByRole("button", {
      name: "Open Intel",
      exact: true,
    });
    await returning.focus();
    await returning.press("Space");
    await expect(
      page.getByRole("textbox", { name: "Search articles" }),
    ).toHaveValue("harbor");
    await expectFixedStatusBar(page);
  });
}

for (const width of [1440, 768, 390, 320]) {
  test(`bulletin date filter sits above category articles at ${width}px`, async ({
    page,
  }) => {
    await fixtures(page);
    await page.setViewportSize({ width, height: 1100 });
    await gotoBulletin(page);
    for (const name of ["Geopolitical", "Economic"]) {
      await page.getByRole("button", { name, exact: true }).click();
      const feed = page.locator("#intel-article-feed");
      const day = feed.getByRole("button", { name: /^Retained day / });
      const article = feed
        .getByRole("button", { name: /^Open Focus Brief:/ })
        .first();
      await expect(day).toBeVisible();
      await expect(article).toBeVisible();
      const dayBox = (await day.boundingBox())!;
      const articleBox = (await article.boundingBox())!;
      expect(dayBox.y + dayBox.height).toBeLessThan(articleBox.y);
      await expect(page.getByText(/^\d+ loaded$/)).toHaveCount(0);
      await expect(
        page.getByRole("heading", { name: /GEOPOLITICAL/ }),
      ).toHaveCount(0);
      const closedTop = articleBox.y;
      await day.click();
      const overlay = page.locator("#intel-day-overlay");
      await expect(overlay).toBeVisible();
      await expect
        .poll(
          async () =>
            await overlay.evaluate((el) => getComputedStyle(el).position),
        )
        .toBe("absolute");
      const openBox = (await article.boundingBox())!;
      expect(openBox.y).toBe(closedTop);
      if (name === "Geopolitical")
        await page.screenshot({
          path: `${captureDir}/bulletin-day-menu-${width}.png`,
          fullPage: false,
        });
      await overlay
        .getByRole("button", { name: "Day 2026-10-25", exact: true })
        .click();
      await expect(overlay).toHaveCount(0);
    }
    await page.screenshot({
      path: `${captureDir}/bulletin-filter-${width}.png`,
      fullPage: true,
    });
  });
}

for (const referenceWidth of [1440, 1280, 768, 390, 320]) {
  test(`topology reference tiles, labeled edges and compact controls at ${referenceWidth}px`, async ({
    page,
  }) => {
    test.setTimeout(120000);
    await page.emulateMedia({ reducedMotion: "reduce" });
    await fixtures(page);
    await page.route("**/api/v1/topology?**", (route) =>
      route.fulfill({
        headers: fixtureCors,
        json: {
          root_label: "Financial Routing Hack",
          nodes: [
            {
              id: "claim",
              label: "Financial Routing Hack",
              kind: "claim",
              detail: "Fixture retained claim",
            },
            {
              id: "actor",
              label: "APT-29",
              kind: "actor",
              detail: "Fixture actor",
            },
            {
              id: "inference",
              label: "State Sponsorship",
              kind: "inference",
              detail: "Fixture inference",
            },
            {
              id: "forum",
              label: "Dark Web Forum Post",
              kind: "claim",
              detail: "Fixture forum claim",
            },
            {
              id: "link",
              label: "t.me/intel_leak_channel",
              kind: "link",
              detail: "Fixture link",
            },
            {
              id: "hidden-second",
              label: "Hidden second hop",
              kind: "actor",
              detail: "Indirect context",
            },
            {
              id: "isolated",
              label: "Unrelated inference",
              kind: "inference",
              detail: "Disconnected context",
            },
            {
              id: "topic",
              label: "Hidden topic",
              kind: "topic",
              detail: "Excluded kind",
            },
          ],
          edges: [
            { id: "executed", from: "actor", to: "claim", label: "executed" },
            { id: "funds", from: "inference", to: "actor", label: "funds" },
            {
              id: "suggests",
              from: "claim",
              to: "inference",
              label: "suggests",
            },
            {
              id: "corroborates",
              from: "forum",
              to: "claim",
              label: "corroborates",
            },
            { id: "leaked", from: "claim", to: "link", label: "leaked via" },
            {
              id: "second-hop",
              from: "actor",
              to: "hidden-second",
              label: "indirect",
            },
            { id: "topic-edge", from: "claim", to: "topic", label: "category" },
          ],
          findings: { gaps: [] },
          has_continuation: false,
        },
      }),
    );
    await page.setViewportSize({ width: referenceWidth, height: 1100 });
    await gotoBulletin(page);
    await page.getByRole("button", { name: /^Open Focus Brief:/ }).click();
    await page.getByRole("button", { name: "Launch 3D topology" }).click();
    await expect(page.locator("html")).toHaveCSS("zoom", "0.75");
    expect(
      (await page.locator("#intel-status-bar").boundingBox())!.height,
    ).toBe(48);
    const viewport = page.locator("#intel-topology-viewport");
    await expect(
      page.getByText("Brain / Topology", { exact: true }),
    ).toHaveCount(0);
    await expect(
      page.getByRole("heading", {
        name: "Financial Routing Hack",
        exact: true,
      }),
    ).toHaveCount(0);
    await expect(viewport.locator("[data-tile=node]")).toHaveCount(5, {
      timeout: 15000,
    });
    const targetTile = viewport.locator(
      '[data-role="focus"] [data-tile="node"]',
    );
    await expect(targetTile).toHaveCSS("animation-name", "none");
    await page.emulateMedia({ reducedMotion: "no-preference" });
    await expect
      .poll(() =>
        targetTile.evaluate((el) => getComputedStyle(el).animationName),
      )
      .toContain("intel-topology-focus");
    await page.emulateMedia({ reducedMotion: "reduce" });
    for (const width of [referenceWidth]) {
      await page.setViewportSize({ width, height: 1100 });
      await viewport.scrollIntoViewIfNeeded();
      // A canvas element existing does not prove its projected tiles are in view.
      await expect
        .poll(async () =>
          viewport.evaluate((el) => {
            const bounds = el.getBoundingClientRect();
            return [...el.querySelectorAll("[data-tile=node]")].every(
              (tile) => {
                const r = tile.getBoundingClientRect();
                return (
                  r.top >= bounds.top &&
                  r.bottom <= bounds.bottom &&
                  r.left >= bounds.left &&
                  r.right <= bounds.right
                );
              },
            );
          }),
        )
        .toBe(true);
      const hud = page.getByRole("group", {
        name: "Neighbor depth",
        exact: true,
      });
      await expect(hud).toBeVisible();
      const legend = page.getByRole("group", {
        name: "Graph legend",
        exact: true,
      });
      for (const kind of ["actor", "claim", "inference", "link"]) {
        const icon = legend.getByRole("img", {
          name: `${kind} icon`,
          exact: true,
        });
        await expect(icon).toBeVisible();
        await expect
          .poll(() => icon.evaluate((el: HTMLImageElement) => el.naturalWidth))
          .toBe(24);
        const color = await legend
          .getByText(kind.toUpperCase(), { exact: true })
          .evaluate((el) => getComputedStyle(el).color);
        expect(
          await viewport
            .locator(`[data-kind="${kind}"] svg path`)
            .first()
            .evaluate((el) => getComputedStyle(el).stroke),
        ).toBe(color);
        expect(
          await viewport
            .locator(`[data-kind="${kind}"] .intel-topology-node-label`)
            .first()
            .evaluate((el) => getComputedStyle(el).color),
        ).toBe(color);
      }
      await expect(viewport.locator(".intel-topology-edge-label")).toHaveCount(
        0,
      );
      await expect(viewport.locator("button")).toHaveCount(0);
      await expect(
        page.getByRole("button", { name: "Clear selection" }),
      ).toHaveCount(0);
      await expect(
        page.getByRole("button", { name: "Show all node types" }),
      ).toHaveCount(0);
      const controls = page.locator("#intel-topology-neighborhood");
      const controlsBox = (await controls.boundingBox())!;
      const viewportBox = (await viewport.boundingBox())!;
      expect(controlsBox.y).toBeGreaterThan(viewportBox.y + viewportBox.height);
      expect(controlsBox.width).toBeCloseTo(viewportBox.width, 0);
      const depthBox = (await page
        .getByRole("group", { name: "Neighbor depth", exact: true })
        .boundingBox())!;
      const kindsBox = (await page
        .getByRole("group", { name: "Node kinds", exact: true })
        .boundingBox())!;
      expect(kindsBox.y).toBeGreaterThan(depthBox.y + depthBox.height);
      const inset = await controls.evaluate((el) => {
        const style = getComputedStyle(el);
        return (
          parseFloat(style.paddingLeft) + parseFloat(style.borderLeftWidth)
        );
      });
      expect(depthBox.x).toBeCloseTo(controlsBox.x + inset, 0);
      expect(kindsBox.x).toBeCloseTo(controlsBox.x + inset, 0);
      const nodesBox = (await page
        .locator("#intel-topology-nodes")
        .boundingBox())!;
      const layoutBox = (await page
        .locator("#intel-topology-layout")
        .boundingBox())!;
      expect(nodesBox.y).toBeGreaterThan(layoutBox.y + layoutBox.height);
      expect(nodesBox.x).toBeCloseTo(layoutBox.x, 0);
      expect(nodesBox.width).toBeCloseTo(layoutBox.width, 0);
      await expect(viewport.locator("#intel-topology-axes")).toHaveCount(0);
      await page.screenshot({
        fullPage: true,
        path: `${captureDir}/topology-reference-${width}.png`,
      });
      await controls.screenshot({
        path: `${captureDir}/topology-controls-${width}.png`,
      });
      await page.locator("#intel-topology-nodes").screenshot({
        path: `${captureDir}/topology-nodes-${width}.png`,
      });
      await expect
        .poll(() =>
          page.evaluate(
            () => document.documentElement.scrollWidth - innerWidth,
          ),
        )
        .toBe(0);
    }
    await page.getByRole("button", { name: "2nd DEG", exact: true }).click();
    await expect(viewport.locator("[data-tile=node]")).toHaveCount(6);
    await expect(viewport.locator(".intel-topology-edge-label")).toHaveCount(0);
    await page
      .getByRole("button", { name: "Show actor nodes", exact: true })
      .click();
    await expect(viewport.locator('[data-kind="actor"]')).toHaveCount(0);
    await expect(
      page.getByRole("button", { name: "Select node APT-29, actor" }),
    ).toHaveCount(0);
  });
}

test("brief source image fits its full aspect ratio and bulletin has no details link", async ({
  page,
}) => {
  const imageUrl = "https://images.example.test/brief.svg";
  await page.route(imageUrl, (route) =>
    route.fulfill({
      contentType: "image/svg+xml",
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="480" height="360"><rect width="480" height="360" fill="#243f50"/><rect x="8" y="8" width="464" height="344" fill="none" stroke="#5fd39a" stroke-width="16"/><circle cx="240" cy="180" r="80" fill="#a46bf0"/><text x="240" y="48" text-anchor="middle" fill="white" font-size="28">TOP EDGE</text><text x="240" y="330" text-anchor="middle" fill="white" font-size="28">BOTTOM EDGE</text></svg>',
    }),
  );
  await fixtures(page, true, imageUrl, { image_url: imageUrl });
  await gotoBulletin(page);
  await expect(page.getByText(/VIEW_DETAILS/)).toHaveCount(0);
  const card = page.getByTestId("article-card-a1");
  const category = await card
    .getByText("GEOPOLITICAL", { exact: true })
    .boundingBox();
  const date = await card
    .getByText("2026-10-25", { exact: true })
    .boundingBox();
  const displayScale = await page
    .locator("html")
    .evaluate((el) => Number(getComputedStyle(el).zoom));
  expect(
    date!.y > category!.y + category!.height ||
      date!.x - category!.x - category!.width >= 12 * displayScale,
  ).toBe(true);
  await card.screenshot({ path: `${captureDir}/bulletin-spacing.png` });
  await page
    .getByRole("button", { name: `Open Focus Brief: ${a.title}`, exact: true })
    .click();
  const image = page.getByRole("img", { name: a.title, exact: true });
  for (const width of [1280, 390]) {
    await page.setViewportSize({ width, height: 1100 });
    await expect(image).toBeVisible();
    await expect
      .poll(() => image.evaluate((el: HTMLImageElement) => el.naturalWidth))
      .toBe(480);
    await image.scrollIntoViewIfNeeded();
    await image.screenshot({ path: `${captureDir}/brief-image-${width}.png` });
    const dimensions = await image.evaluate((el) => {
      const box = el.getBoundingClientRect();
      return {
        width: box.width,
        height: box.height,
        fit: getComputedStyle(el.previousElementSibling!).backgroundSize,
      };
    });
    expect(dimensions.fit).toBe("contain");
    expect(
      Math.abs(
        dimensions.height -
          Math.min(520 * displayScale, (dimensions.width * 3) / 4),
      ),
    ).toBeLessThan(1);
  }
});

test("Rust host metrics reach the status bar and refresh failures remain honest", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await fixtures(page);
  const live = await installLiveFixture(page);
  live.metrics({
    cpu_percent: 37,
    memory_percent: 62,
    disk_percent: 48,
    network_kbps: 0,
  });
  await gotoBulletin(page);
  const bar = page.locator("#intel-status-bar");
  await expect(bar.locator("#intel-status-metric-cpu")).toHaveText("CPU 37%");
  await expect(bar.locator("#intel-status-metric-memory")).toHaveText(
    "MEM 62%",
  );
  await expect(bar.locator("#intel-status-metric-disk")).toHaveText("DSK 48%");
  await expect(
    bar.getByRole("group", { name: "NET 0 KB/s", exact: true }),
  ).toBeVisible();
  await bar.screenshot({ path: `${captureDir}/host-metrics-status.png` });
  live.metrics({
    cpu_percent: 0,
    memory_percent: 62,
    disk_percent: 48,
    network_kbps: 0,
  });
  await expect(bar.locator("#intel-status-metric-cpu")).toHaveText("CPU 0%", {
    timeout: 12000,
  });
  live.disconnect();
  await expect(bar.locator("#intel-status-metric-cpu")).toHaveText("CPU —", {
    timeout: 15000,
  });
  await expect(bar.getByLabel("API connected")).toBeVisible();
});

test("Home, Bulletin, Brief and inference Topology routes survive reload and history", async ({
  page,
}) => {
  test.setTimeout(60000);
  await page.emulateMedia({ reducedMotion: "reduce" });
  const state = await fixtures(page);
  await page.route("**/api/v1/topology?**", async (route) =>
    route.fulfill({
      json: {
        root_label: "State Sponsorship",
        nodes: [
          {
            id: "brain-memory-fixture",
            is_root: true,
            kind: "inference",
            label: "State Sponsorship",
            detail: "Retained analytical inference",
            topology_seed: { Inference: "brain-memory-fixture" },
          },
          {
            id: "claim-fixture",
            kind: "claim",
            label: "Retained claim",
            detail: "Recorded claim",
          },
        ],
        edges: [
          {
            id: "suggests",
            from: "claim-fixture",
            to: "brain-memory-fixture",
            label: "suggests",
          },
        ],
        findings: { gaps: [] },
        has_continuation: false,
      },
    }),
  );
  await gotoBulletin(page, "/home");
  await expect(page).toHaveURL(/\/bulletin$/);
  await page.getByRole("button", { name: /^Open Focus Brief:/ }).click();
  await expect(page).toHaveURL(/\/brief\?run=r1&id=a1$/);
  await page.reload();
  await expect(
    page.getByRole("heading", { name: a.title, exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Launch 3D topology" }).click();
  await expect(page).toHaveURL(/\/topology\?seed=actor1$/);
  await expect(
    page.getByRole("button", {
      name: "Select node State Sponsorship, inference",
    }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Show inference nodes" }),
  ).toBeVisible();
  await page.goBack();
  await expect(
    page.getByRole("heading", { name: a.title, exact: true }),
  ).toBeVisible();
  await page.goForward();
  await expect(
    page.getByRole("button", {
      name: "Select node State Sponsorship, inference",
      exact: true,
    }),
  ).toBeVisible();
  await page.reload();
  await expect(
    page.getByRole("button", {
      name: "Select node State Sponsorship, inference",
    }),
  ).toBeVisible();
  expect(new URL(page.url()).searchParams.has("api")).toBe(false);
  expect([...new URL(page.url()).searchParams.keys()]).toEqual(["seed"]);
  expect(new URL(page.url()).searchParams.get("seed")).toBe("actor1");
  await page
    .getByRole("button", { name: "Return to Brief", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: a.title, exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Return to Bulletin", exact: true })
    .click();
  await expect(page).toHaveURL(/\/bulletin$/);
  await page.getByRole("button", { name: "server", exact: true }).click();
  await expect(page).toHaveURL(/\/home$/);
  expect(state.requests).toContain("articles/detail");
});

test("each indexed brief actor, claim and inference opens its own memory seed", async ({
  page,
}) => {
  test.setTimeout(60000);
  await fixtures(page);
  const requests: string[] = [];
  await page.route("**/api/v1/topology?**", (route) => {
    const id = new URL(route.request().url()).searchParams.get("seed")!;
    requests.push(id);
    const kind = id.startsWith("claim")
      ? "claim"
      : id.startsWith("inference")
        ? "inference"
        : "entity";
    return route.fulfill({
      json: {
        root_label: id,
        nodes: [
          {
            id,
            kind,
            label: id,
            detail: "Retained Brain memory",
            is_root: true,
            topology_seed: null,
          },
        ],
        edges: [],
        findings: { gaps: [] },
        has_continuation: false,
      },
    });
  });
  await gotoBulletin(page);
  await page.getByRole("button", { name: /^Open Focus Brief:/ }).click();
  for (const [name, id] of [
    ["Explore actor Port authority", "actor1"],
    ["Explore claim: Shipping talks resumed on Monday.", "claim1"],
    ["Explore inference: Access may improve.", "inference1"],
  ]) {
    const action = page.getByRole("button", { name, exact: true });
    await expect(action).toBeEnabled();
    await action.click();
    if (id === "actor1") {
      await expect(page).toHaveURL(/\/actor\?id=actor1$/);
      await page.getByRole("button", { name: "OPEN ACTOR TOPOLOGY" }).click();
    }
    await expect
      .poll(() => new URL(page.url()).searchParams.get("seed"))
      .toBe(id);
    await expect(
      page.getByRole("button", {
        name: `Select node ${id}, ${id.startsWith("claim") ? "claim" : id.startsWith("inference") ? "inference" : "entity"}`,
        exact: true,
      }),
    ).toBeVisible();
    if (id === "actor1")
      await page
        .getByRole("button", { name: "Return to Actor", exact: true })
        .click();
    await page
      .getByRole("button", { name: "Return to Brief", exact: true })
      .click();
    await expect(
      page.getByRole("heading", { name: a.title, exact: true }),
    ).toBeVisible();
  }
  expect([...new Set(requests)]).toEqual(["actor1", "claim1", "inference1"]);
});

test("manual topology zoom survives updates and node drag cancellation stays safe", async ({
  page,
}) => {
  test.setTimeout(60000);
  await fixtures(page);
  await page.route("**/api/v1/topology?**", (route) =>
    route.fulfill({
      headers: fixtureCors,
      json: {
        root_label: "Actor neighborhood",
        nodes: [
          {
            id: "actor1",
            label: "Actor target",
            kind: "actor",
            detail: "Target",
            is_root: true,
          },
          {
            id: "claim1",
            label: "Direct claim",
            kind: "claim",
            detail: "Direct neighbor",
          },
          {
            id: "inference1",
            label: "Indirect inference",
            kind: "inference",
            detail: "Second hop",
          },
        ],
        edges: [
          { id: "direct", from: "actor1", to: "claim1", label: "asserts" },
          {
            id: "indirect",
            from: "claim1",
            to: "inference1",
            label: "suggests",
          },
        ],
        findings: { gaps: [] },
        has_continuation: false,
      },
    }),
  );
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.setViewportSize({ width: 1280, height: 1100 });
  await page.goto("/topology?seed=actor1");
  const viewport = page.locator("#intel-topology-viewport");
  const focus = viewport.locator('[data-role="focus"] [data-tile="node"]');
  await expect(focus).toBeVisible({ timeout: 20000 });
  await expect
    .poll(() =>
      focus.evaluate(
        (el) =>
          el
            .getAnimations()
            .find(
              (animation) =>
                (animation as CSSAnimation).animationName ===
                "intel-topology-arrive",
            )?.playState,
      ),
    )
    .toBe("finished");
  const zoom = page.locator("#intel-topology-neighborhood").getByText(/^ZOOM:/);
  async function stableZoom() {
    let previous = "";
    let stable = 0;
    await expect
      .poll(
        async () => {
          const current = await zoom.innerText();
          stable = current === previous ? stable + 1 : 0;
          previous = current;
          return stable;
        },
        { timeout: 15000, intervals: [250, 500] },
      )
      .toBeGreaterThanOrEqual(2);
    return previous;
  }
  const initial = await stableZoom();
  // Empty graph space scrolls the page; wheel over a node controls graph zoom.
  const target = (await focus.boundingBox())!;
  // The CSS2D tile is decorative: the canvas owns hit testing and wheel input.
  await page.mouse.move(
    target.x + target.width / 2,
    target.y + target.height / 2,
  );
  await page.mouse.wheel(0, -250);
  await expect(zoom).not.toHaveText(initial);
  const manual = await stableZoom();
  await page.setViewportSize({ width: 1200, height: 1100 });
  await page
    .getByRole("button", { name: "Show claim nodes", exact: true })
    .click();
  expect(await stableZoom()).toBe(manual);
  await page
    .getByRole("button", { name: "Show claim nodes", exact: true })
    .click();
  expect(await stableZoom()).toBe(manual);

  // Filter controls sit below the graph and scroll it out of view on activation.
  await focus.scrollIntoViewIfNeeded();
  const tile = (await focus.boundingBox())!;
  const center = { x: tile.x + tile.width / 2, y: tile.y + tile.height / 2 };
  const canvas = viewport.locator("canvas").first();
  // Reentrant pointer cancellation can occur while the graph library dispatches
  // its synthetic pointerup during drag-end cleanup (for example on pointer leave).
  await canvas.evaluate((element) => {
    element.ownerDocument.addEventListener(
      "pointerup",
      (event) => {
        if (!event.isTrusted)
          element.dispatchEvent(new PointerEvent("pointerleave"));
      },
      { once: true },
    );
  });
  await page.mouse.move(center.x, center.y);
  await page.mouse.down();
  await expect(canvas).toHaveClass(/grabbable/);
  await page.mouse.move(center.x + 35, center.y + 20, { steps: 6 });
  await page.mouse.up();
  await expect(canvas).not.toHaveClass(/grabbable/);
  await expect(focus).toBeVisible();
  await expect(
    page.getByText("Cannot read properties of undefined"),
  ).toHaveCount(0);
});

test("Bulletin and completed Brief stay quiet between deliberate REST refreshes", async ({
  page,
}) => {
  await page.clock.install();
  const state = await fixtures(page);
  const live = await installLiveFixture(page);
  await gotoBulletin(page);
  await expect(
    page.getByRole("button", { name: /^Open Focus Brief:/ }),
  ).toBeVisible();
  const reads = () =>
    state.requests.filter(
      (path) => path !== "health/ready" && path !== "capabilities",
    ).length;
  const bulletinReads = reads();
  await page.clock.runFor(20_000);
  expect(reads()).toBe(bulletinReads);
  expect(state.requests.filter((path) => path === "host/metrics")).toEqual([]);
  await page.getByRole("button", { name: /^Open Focus Brief:/ }).click();
  await expect(page.getByText("Access may improve.").first()).toBeVisible();
  await expect
    .poll(() =>
      live.subscriptions.some(
        (subscription) => subscription.brief?.id === a.id,
      ),
    )
    .toBe(true);
  let previous = -1;
  let stable = 0;
  await expect
    .poll(() => {
      const count = reads();
      stable = count === previous ? stable + 1 : 0;
      previous = count;
      return stable;
    })
    .toBeGreaterThanOrEqual(2);
  const briefReads = reads();
  await page.clock.runFor(20_000);
  expect(reads()).toBe(briefReads);
  live.metrics({
    cpu_percent: 42,
    memory_percent: 62,
    disk_percent: 48,
    network_kbps: 0,
  });
  await expect(page.locator("#intel-status-metric-cpu")).toHaveText("CPU 42%");
  expect(reads()).toBe(briefReads);
});

test("pending Brief follows push progress, resyncs after reconnect and stops REST polling", async ({
  page,
}) => {
  test.setTimeout(60000);
  await page.clock.install();
  const state = await fixtures(page);
  const live = await installLiveFixture(page);
  let ready = false;
  let detailReads = 0;
  await page.route("**/api/v1/articles/detail?**", (route) => {
    detailReads++;
    return route.fulfill({
      headers: fixtureCors,
      json: {
        article: a,
        actors: [],
        inferences: [],
        relations: [],
        claims: ready
          ? [
              {
                id: "c1",
                text: "Fresh retained claim from completed extraction",
                classification: "reported",
                confidence: 70,
                admiralty: "B2",
                topology_seed: { Claim: "c1" },
              },
            ]
          : [],
        body: null,
        body_pending: !ready,
        output: null,
        lifecycle: ready ? "completed" : "running",
        status: ready ? "Published" : "Extracting",
        revision: ready ? "ready" : "pending",
        publication: ready ? [1, {}, {}] : null,
        job_id: "j1",
        indexing: {
          state: ready ? "indexed" : "not_indexed",
          reason: ready ? "" : "Waiting",
        },
        topology_seed: ready ? { Claim: "c1" } : null,
      },
    });
  });
  const progress = {
    run: a.run_id,
    id: a.id,
    job_id: "j1",
    stages: [
      {
        operation: "intel_brief_body",
        label: "Article body",
        state: "running",
        row: "Running",
        active: true,
        detail: "",
      },
    ],
    indexing: null,
    complete: false,
    refresh: true,
  };
  live.brief(progress);
  await gotoBulletin(page);
  await page.getByRole("button", { name: /^Open Focus Brief:/ }).click();
  await expect(
    page.getByText("Brief in progress", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Article body · Running", { exact: true }),
  ).toBeVisible();
  await expect.poll(() => detailReads).toBeGreaterThan(0);
  let previous = -1;
  let stable = 0;
  await expect
    .poll(() => {
      stable = previous === detailReads ? stable + 1 : 0;
      previous = detailReads;
      return stable;
    })
    .toBeGreaterThanOrEqual(2);
  const waitingReads = detailReads;
  await page.clock.runFor(10_000);
  expect(detailReads).toBe(waitingReads);
  await page.screenshot({
    path: `${captureDir}/brief-live-pending.png`,
    fullPage: true,
  });
  live.disconnect();
  await expect(
    page.getByText("Live updates interrupted. Reconnecting…"),
  ).toBeVisible();
  ready = true;
  live.brief({
    ...progress,
    stages: [
      {
        ...progress.stages[0],
        state: "completed",
        row: "Completed",
        active: false,
      },
    ],
    indexing: { state: "indexed", reason: "" },
    complete: true,
  });
  live.reconnect();
  await page.clock.runFor(2500);
  await expect(
    page.getByText("Fresh retained claim from completed extraction"),
  ).toBeVisible();
  await expect(
    page.getByText("Brief in progress", { exact: true }),
  ).toHaveCount(0);
  const completedReads = detailReads;
  await page.clock.runFor(20_000);
  expect(detailReads).toBe(completedReads);
  expect(state.requests.filter((path) => path === "host/metrics")).toEqual([]);
  await page
    .getByRole("button", { name: "Return to Bulletin", exact: true })
    .click();
  await expect
    .poll(() => live.subscriptions.at(-1)?.brief)
    .toMatchObject({ run: "r1", id: "a1" });
  await page
    .getByRole("button", { name: "Background jobs", exact: true })
    .click();
  const jobs = page.getByRole("menu");
  await expect(jobs.getByText("Article body", { exact: true })).toBeVisible();
  await expect(jobs.getByText("Completed", { exact: true })).toBeVisible();
  await expect(jobs.getByText("indexed", { exact: true })).toBeVisible();
});

test.describe("revamp connection scene", () => {
  test("landing orbit remains accessible while readiness is pending", async ({
    page,
  }) => {
    await page.emulateMedia({ reducedMotion: "no-preference" });
    test.setTimeout(60000);
    await fixtures(page);
    const ready = deferred();
    await page.route("**/api/v1/health/ready", async (route) => {
      await ready.promise;
      await route.fulfill({
        json: {
          service: "argos-intel",
          api_version: 1,
          ready: true,
          storage: true,
          instance_id: "fixture-instance",
        },
      });
    });
    await page.goto("/");
    await expect(
      page.getByRole("status", { name: "Checking local API…" }),
    ).toBeVisible();
    await expect(page.locator("canvas")).toHaveCount(1);
    await expect
      .poll(() =>
        page
          .locator("#intel-landing-status")
          .evaluate((node) => Number(getComputedStyle(node).opacity)),
      )
      .toBeGreaterThan(0.95);
    const stage = (await page.locator("#intel-landing").boundingBox())!;
    const canvas = (await page.locator("canvas").boundingBox())!;
    const bar = (await page.locator("#intel-status-bar").boundingBox())!;
    expect(stage.x).toBe(0);
    expect(stage.y).toBe(0);
    expect(
      Math.abs(stage.width - (await page.evaluate(() => innerWidth))),
    ).toBeLessThanOrEqual(1);
    expect(
      Math.abs(
        stage.height + bar.height - (await page.evaluate(() => innerHeight)),
      ),
    ).toBeLessThanOrEqual(1);
    expect(Math.abs(canvas.width - stage.width)).toBeLessThanOrEqual(1);
    expect(Math.abs(canvas.height - stage.height)).toBeLessThanOrEqual(1);

    await page.screenshot({
      path: `${captureDir}/landing-orbit.png`,
      fullPage: true,
    });
    await expect(
      page.getByRole("button", { name: "Retry connection checks" }),
    ).toHaveCount(0);
    // Advance a real scene through arrival and acceleration for the retained video.
    await page.waitForTimeout(1000);
    ready.resolve();
    for (const [index, label] of [
      "Local API reachable",
      "Intel service identified",
      "API compatible",
      "Storage ready",
      "Workspace readiness passed",
    ].entries()) {
      await expect(
        page.getByRole("status", { name: label, exact: true }),
      ).toBeVisible({ timeout: 5000 });
      await expect(
        page
          .getByRole("list", { name: "Connection checks" })
          .getByRole("listitem"),
      ).toHaveCount(index + 1);
      await expect(
        page.getByRole("button", { name: "Open Intel", exact: true }),
      ).toHaveCount(0);
    }
    await expect(
      page.getByRole("status", {
        name: "All connections healthy",
        exact: true,
      }),
    ).toBeVisible({ timeout: 5000 });
    await page.screenshot({
      path: `${captureDir}/landing-core-green.png`,
      fullPage: true,
    });
    await expect(
      page.getByRole("status", { name: "Workspace ready", exact: true }),
    ).toBeVisible({ timeout: 5000 });
    await page.screenshot({
      path: `${captureDir}/landing-core-blue.png`,
      fullPage: true,
    });
    await expect(
      page.getByRole("button", { name: "Open Intel", exact: true }),
    ).toBeVisible({ timeout: 10000 });
  });
  test("landing fallback honors reduced motion without WebGL", async ({
    page,
  }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    await page.addInitScript(() => {
      Object.defineProperty(window, "WebGL2RenderingContext", {
        value: undefined,
      });
    });
    await fixtures(page);
    const ready = deferred();
    await page.route("**/api/v1/health/ready", async (route) => {
      await ready.promise;
      await route.fulfill({
        json: {
          service: "argos-intel",
          api_version: 1,
          ready: true,
          storage: true,
          instance_id: "fixture-instance",
        },
      });
    });
    await page.goto("/");
    await expect(
      page.getByRole("status", { name: "Checking local API…" }),
    ).toBeVisible();
    await expect(page.locator("canvas")).toHaveCount(0);
    await page.screenshot({
      path: `${captureDir}/landing-static-fallback.png`,
      fullPage: true,
    });
    ready.resolve();
    await expect(
      page.getByRole("button", { name: "Open Intel", exact: true }),
    ).toBeVisible();
  });
});

test("bulletin skeleton reserves the loaded card height", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await fixtures(page);
  const ready = deferred();
  await page.route(/\/api\/v1\/articles\?/, async (route) => {
    await ready.promise;
    await route.fallback();
  });
  await gotoBulletin(page);
  const skeleton = page.getByTestId("bulletin-skeleton").first();
  await expect(skeleton).toBeVisible();
  const height = (await skeleton.boundingBox())!.height;
  ready.resolve();
  const card = page.getByTestId("article-card-a1");
  await expect(card).toBeVisible();
  expect(
    Math.abs((await card.boundingBox())!.height - height),
  ).toBeLessThanOrEqual(1);
});

for (const width of [1440, 390]) {
  test(`bulletin latest and hottest controls use server ordering at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 1000 });
    await fixtures(page);
    const seen: URLSearchParams[] = [];
    await page.route("**/api/v1/articles?**", async (route) => {
      const params = new URL(route.request().url()).searchParams;
      seen.push(params);
      const latest = {
        ...a,
        id: "new",
        title: "Latest retained story",
        temperature: 0.1,
      };
      const hot = {
        ...a,
        id: "hot",
        title: "Hottest retained story",
        temperature: 0.9,
      };
      await route.fulfill({
        json: {
          rows:
            params.get("sort") === "hottest" ? [hot, latest] : [latest, hot],
          next_cursor: null,
          revision: "sort-fixture",
        },
      });
    });
    await page.goto("/bulletin");
    await expect(
      page.getByRole("heading", { name: "Intelligence bulletin", exact: true }),
    ).toBeVisible();
    const cards = page.getByRole("button", { name: /^Open Focus Brief:/ });
    await expect(cards.first()).toHaveAccessibleName(/Latest retained story/);
    const dayBox = (await page
      .getByRole("button", { name: /^Retained day/ })
      .boundingBox())!;
    const latestBox = (await page
      .getByRole("button", { name: "Latest", exact: true })
      .boundingBox())!;
    if (width >= 900)
      expect(latestBox.x - dayBox.x - dayBox.width).toBeLessThan(20);
    await page.getByRole("button", { name: "Hottest", exact: true }).click();
    await expect(cards.first()).toHaveAccessibleName(/Hottest retained story/);
    expect(
      seen.filter((p) => p.get("sort") === "hottest")[0].get("cursor"),
    ).toBeNull();
    await page.screenshot({
      path: `${captureDir}/bulletin-hottest-${width}.png`,
      fullPage: true,
    });
    await page.getByRole("button", { name: "Latest", exact: true }).click();
    await expect(cards.first()).toHaveAccessibleName(/Latest retained story/);
  });
}

test("HTTP article images are allowed by the image CSP and render", async ({
  page,
}) => {
  const image = "http://images.fixture.test/article.svg";
  await fixtures(page, true, image);
  await page.route(image, (route) =>
    route.fulfill({
      contentType: "image/svg+xml",
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><rect width="80" height="40" fill="green"/></svg>',
    }),
  );
  await page.goto("/bulletin");
  await expect(
    page.getByRole("heading", { name: "Intelligence bulletin", exact: true }),
  ).toBeVisible();
  const img = page
    .locator('img[src="http://images.fixture.test/article.svg"]')
    .first();
  await expect(img).toBeVisible();
  await expect
    .poll(() => img.evaluate((el) => (el as HTMLImageElement).naturalWidth))
    .toBe(80);
});
