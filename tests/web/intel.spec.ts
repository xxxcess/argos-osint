import { test, expect, type Page } from "@playwright/test";
const runtimeErrors = new WeakMap<Page, string[]>();
test.beforeEach(async ({ page }) => {
  const errors: string[] = [];
  runtimeErrors.set(page, errors);
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (message) => {
    if (
      message.type() === "error" &&
      /content security policy|hydration|unexpected text node/i.test(
        message.text(),
      )
    )
      errors.push(message.text());
  });
});
test.afterEach(async ({ page }) => {
  expect(runtimeErrors.get(page)).toEqual([]);
});
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
async function fixtures(page: Page, bodyAvailable = true, imageUrl = "") {
  let online = true;
  const mutations: string[] = [];
  const seeds: unknown[] = [];
  const requests: string[] = [];
  const admissions: { run: string; id: string }[] = [];
  let admissionFails = false;
  await page.route("http://127.0.0.1:8080/api/v1/**", async (route) => {
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
    else if (path === "days") body = { days: ["2026-10-25"] };
    else if (path === "articles")
      body = {
        rows:
          url.searchParams.get("search") === "missing"
            ? []
            : [
                { ...a, image_url: imageUrl },
                ...(imageUrl
                  ? [
                      {
                        ...a,
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
        article: a,
        source_revision: "fixture-source",
        actors: [
          {
            id: "actor1",
            display_name: "Port authority",
            type_hint: "organization",
          },
        ],
        claims: [
          {
            id: "c1",
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
    else if (path === "topology") {
      seeds.push(route.request().postDataJSON().seed);
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
          headers: { "access-control-allow-origin": "http://127.0.0.1:3100" },
        });
        return;
      }
      body = { job_id: "job1" };
    } else if (route.request().method() === "POST") {
      mutations.push(path);
      body = { job_id: "job1" };
    }
    await route.fulfill({
      json: body,
      headers: { "access-control-allow-origin": "http://127.0.0.1:3100" },
    });
  });
  return {
    offline: () => {
      online = false;
    },
    online: () => {
      online = true;
    },
    mutations,
    admissions,
    failAdmission: (failed: boolean) => {
      admissionFails = failed;
    },
    seeds,
    requests,
  };
}
test("opening and revisiting briefs admits each occurrence without polling job endpoints", async ({
  page,
}) => {
  const f = await fixtures(page);
  await page.goto("/");
  await page.getByRole("button", { name: "Open Focus Brief ↗" }).click();
  await expect.poll(() => f.admissions).toEqual([{ run: "r1", id: "a1" }]);
  await expect(
    page.getByRole("button", { name: "Ensure / resume brief" }),
  ).toBeEnabled();
  const reads = () =>
    f.requests.filter((path) => path === "articles/detail").length;
  const initial = reads();
  await page.getByRole("button", { name: "Refresh retained brief" }).click();
  await expect.poll(reads).toBeGreaterThan(initial);
  const refreshed = reads();
  await expect.poll(reads, { timeout: 8000 }).toBeGreaterThan(refreshed);
  expect(f.admissions).toEqual([{ run: "r1", id: "a1" }]);
  await page
    .getByRole("button", { name: "Earlier shipping agreement" })
    .click();
  await expect
    .poll(() => f.admissions)
    .toEqual([
      { run: "r1", id: "a1" },
      { run: "r1", id: "a2" },
    ]);
  await page.getByRole("button", { name: "← Bulletin" }).click();
  await page.getByRole("button", { name: "Open Focus Brief ↗" }).click();
  await expect
    .poll(() => f.admissions)
    .toEqual([
      { run: "r1", id: "a1" },
      { run: "r1", id: "a2" },
      { run: "r1", id: "a1" },
    ]);
  expect(
    f.requests.filter((path) => /^(jobs|logs|brief\/stages)/.test(path)),
  ).toEqual([]);
});

test("automatic admission errors are visible on narrow Reader and can be retried", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  const f = await fixtures(page);
  f.failAdmission(true);
  await page.goto("/");
  await page.getByRole("button", { name: "Open Focus Brief ↗" }).click();
  await expect(
    page
      .getByRole("alert")
      .filter({ hasText: "Could not start brief extraction" }),
  ).toBeVisible();
  expect(f.admissions).toEqual([{ run: "r1", id: "a1" }]);
  f.failAdmission(false);
  await page.getByRole("button", { name: "Context", exact: true }).click();
  await page.getByRole("button", { name: "Ensure / resume brief" }).click();
  await expect.poll(() => f.admissions.length).toBe(2);
  await expect(
    page
      .getByRole("alert")
      .filter({ hasText: "Could not start brief extraction" }),
  ).toHaveCount(0);
});

for (const width of [1440, 768, 390, 320]) {
  test(`feed image stays left of article text at ${width}px`, async ({
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
    await page.goto("/");
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
    expect(secondaryImageBox.x + secondaryImageBox.width).toBeLessThan(
      secondaryTextBox.x,
    );
    expect(imageBox.x + imageBox.width).toBeLessThan(titleBox.x);
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
    await expect(
      card.getByRole("button", { name: "Open Focus Brief ↗" }),
    ).toBeVisible();
    if (width === 1440 || width === 320)
      await page.screenshot({
        path: `.planning/2026-10-26-intel-images/web-feed-${width}.png`,
        fullPage: true,
      });
    await card.getByRole("button", { name: "Open Focus Brief ↗" }).click();
    await expect(
      page.getByRole("button", { name: "← Bulletin" }),
    ).toBeVisible();
  });
}

test("feed images collapse for missing, unsafe and failed sources", async ({
  page,
}) => {
  await page.route("https://images.example.test/broken.png", (route) =>
    route.abort(),
  );
  for (const imageUrl of [
    "",
    "javascript:alert(1)",
    "https://images.example.test/broken.png",
  ]) {
    await fixtures(page, true, imageUrl);
    await page.goto("/");
    const card = page.getByTestId("article-card-a1");
    await expect(card).toBeVisible();
    await expect(card.getByRole("img")).toHaveCount(0);
    await expect(card.getByText(a.title, { exact: true })).toBeVisible();
    await expect(
      card.getByRole("button", { name: "Open Focus Brief ↗" }),
    ).toBeVisible();
  }
});

test("gates the workspace when the API is unavailable", async ({ page }) => {
  await page.route("http://127.0.0.1:8080/**", (r) => r.abort());
  await page.goto("/");
  await expect(
    page.getByText("Connect to your Intel workspace."),
  ).toBeVisible();
  await expect(page.getByText("The world, in focus.")).toHaveCount(0);
  await page
    .getByRole("textbox", { name: "API endpoint" })
    .fill("http://evil.test:8080");
  await page.getByRole("button", { name: "Connect / retry" }).click();
  await expect(
    page.getByRole("alert").filter({ hasText: "loopback" }),
  ).toBeVisible();
});
test("browse, search, brief, related, topology and reconnect", async ({
  page,
}) => {
  const f = await fixtures(page);
  await page.goto("/");
  await expect(page.getByText("The world, in focus.")).toBeVisible();
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
  await page.getByRole("textbox", { name: "Search articles" }).fill("");
  await page.getByRole("button", { name: "Open Focus Brief ↗" }).click();
  await expect.poll(() => f.admissions).toEqual([{ run: "r1", id: "a1" }]);
  await expect(
    page.getByText("Officials meet to discuss regional shipping access."),
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
    path: ".planning/2026-10-25-argos-intel/web-focus-wide.png",
    fullPage: true,
  });
  await page.getByRole("button", { name: "Ensure / resume brief" }).click();
  await expect.poll(() => f.admissions.length).toBe(2);
  await page
    .getByRole("button", { name: "Reload article / new generation" })
    .click();
  await expect.poll(() => f.mutations).toContain("brief/reload");
  await page
    .getByRole("button", { name: "Port authority · organization" })
    .click();
  await expect(page.getByText("Recorded actor")).toBeVisible();
  await expect
    .poll(() => f.seeds)
    .toContainEqual({
      RetainedActor: {
        run_id: "r1",
        article_id: "a1",
        revision: "fixture-source",
        actor_ref: "actor1",
        name: "Port authority",
      },
    });
  await page.getByRole("button", { name: "Bulletin", exact: true }).click();
  await page.getByRole("button", { name: "Explore article topology" }).click();
  await expect
    .poll(() => f.seeds)
    .toContainEqual({
      Article: {
        run_id: "r1",
        article_id: "a1",
        revision: "fixture-source",
        originating_memory: null,
      },
    });
  await expect(page.getByText("Recorded actor")).toBeVisible();
  f.offline();
  await expect(page.getByText("Connect to your Intel workspace.")).toBeVisible({
    timeout: 75000,
  });
  await expect(
    page.getByRole("button", { name: "Jobs", exact: true }),
  ).toHaveCount(0);
  f.online();
  await page.getByRole("button", { name: "Connect / retry" }).click();
  await expect(page.getByText("The world, in focus.")).toBeVisible();
  expect(
    f.requests.filter((path) =>
      /^(jobs|logs)(\/|$)|^brief\/stages$/.test(path),
    ),
  ).toEqual([]);
  await page.screenshot({
    path: ".planning/2026-10-25-argos-intel/web-wide.png",
    fullPage: true,
  });
});
test("narrow Focus Brief tabs and static hosting policy", async ({
  page,
  request,
}) => {
  await page.setViewportSize({ width: 320, height: 844 });
  await fixtures(page);
  await page.goto("/");
  await page.getByRole("button", { name: "Open Focus Brief ↗" }).click();
  await expect(
    page.getByRole("button", { name: "Reader", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Extracted", exact: true }).click();
  await expect(
    page.getByText("Shipping talks resumed on Monday."),
  ).toBeVisible();
  await page.getByRole("button", { name: "Context", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Ensure / resume brief" }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: ".planning/2026-10-25-argos-intel/web-narrow.png",
    fullPage: true,
  });
  const response = await request.get("/");
  expect(response.headers()["content-security-policy"]).toContain(
    "frame-ancestors 'none'",
  );
  expect(response.headers()["x-content-type-options"]).toBe("nosniff");
});

test("metadata excerpt is not presented as a retained full body", async ({
  page,
}) => {
  await fixtures(page, false);
  await page.goto("/");
  await page.getByRole("button", { name: "Open Focus Brief ↗" }).click();
  await expect(
    page.getByText("Officials meet to discuss regional shipping access."),
  ).toBeVisible();
  await expect(page.getByText(/Full article body/)).toHaveCount(0);
});
