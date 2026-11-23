import { installLiveFixture } from "./live-fixture";
import { test, expect, type CDPSession, type Page } from "@playwright/test";

test.use({ reducedMotion: "reduce" });

const CAPTURE =
  process.env.ARGOS_WEB_CAPTURE_DIR || ".agent-scratch/tester-gate/screenshots";
const API = `http://127.0.0.1:${process.env.ARGOS_WEB_TEST_PORT || 3100}`;

const article = {
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

async function mockApi(page: Page) {
  await installLiveFixture(page);
  await page.route(`${API}/api/v1/**`, async (route) => {
    const url = new URL(route.request().url());
    const path = url.pathname.replace("/api/v1/", "");
    const cursor = url.searchParams.get("cursor");
    if (path === "health/ready") {
      await route.fulfill({
        json: {
          service: "argos-intel",
          api_version: 1,
          ready: true,
          storage: true,
          instance_id: "fixture-instance",
        },
        headers: {
          "access-control-allow-origin": `http://127.0.0.1:${process.env.ARGOS_WEB_TEST_PORT || 3100}`,
        },
      });
      return;
    }
    if (path === "days") {
      await route.fulfill({
        json: { days: ["2026-10-25"] },
        headers: {
          "access-control-allow-origin": `http://127.0.0.1:${process.env.ARGOS_WEB_TEST_PORT || 3100}`,
        },
      });
      return;
    }
    if (path === "articles") {
      const next = cursor ? null : "retry-cursor";
      await route.fulfill({
        json: {
          rows: cursor
            ? []
            : [
                { ...article, image_url: "" },
                {
                  ...article,
                  id: "a2",
                  title: "Earlier shipping agreement",
                  image_url: "",
                },
              ],
          next_cursor: next,
          revision: "r1",
        },
        headers: {
          "access-control-allow-origin": `http://127.0.0.1:${process.env.ARGOS_WEB_TEST_PORT || 3100}`,
        },
      });
      return;
    }
    if (path === "articles/detail") {
      await route.fulfill({
        json: {
          article,
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
          body: {
            body_markdown:
              "The full retained article describes the negotiations and their regional context.",
          },
          lifecycle: "active",
          status: "Published",
          revision: "r2",
          publication: [
            1,
            { claims: [] },
            { inferences: [{ text: "Access may improve." }] },
          ],
          relations: [],
        },
        headers: {
          "access-control-allow-origin": `http://127.0.0.1:${process.env.ARGOS_WEB_TEST_PORT || 3100}`,
        },
      });
      return;
    }
    if (path === "articles/related") {
      await route.fulfill({
        json: {
          rows: [
            {
              article: {
                ...article,
                id: "a2",
                title: "Earlier shipping agreement",
              },
              basis: ["shared actor"],
              tentative: true,
            },
          ],
          next_cursor: null,
        },
        headers: {
          "access-control-allow-origin": `http://127.0.0.1:${process.env.ARGOS_WEB_TEST_PORT || 3100}`,
        },
      });
      return;
    }
    if (path === "topology/resolve") {
      await route.fulfill({ json: { seed: "brain-memory-fixture" } });
      return;
    }
    if (path === "topology") {
      await route.fulfill({
        json: {
          root_label: "Port authority",
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
              label: "Regional shipping",
              kind: "topic",
              detail: "Recorded topic",
              topology_seed: { Topic: "shipping" },
            },
          ],
          edges: [{ id: "edge1", from: "n1", to: "n2", label: "related" }],
          findings: { gaps: [] },
          has_continuation: false,
        },
        headers: {
          "access-control-allow-origin": `http://127.0.0.1:${process.env.ARGOS_WEB_TEST_PORT || 3100}`,
        },
      });
      return;
    }
    if (path === "brief/ensure") {
      await route.fulfill({
        json: { job_id: "job1" },
        headers: {
          "access-control-allow-origin": `http://127.0.0.1:${process.env.ARGOS_WEB_TEST_PORT || 3100}`,
        },
      });
      return;
    }
    if (route.request().method() === "POST") {
      await route.fulfill({
        json: { job_id: "job1" },
        headers: {
          "access-control-allow-origin": `http://127.0.0.1:${process.env.ARGOS_WEB_TEST_PORT || 3100}`,
        },
      });
      return;
    }
    await route.fulfill({
      json: {},
      headers: {
        "access-control-allow-origin": `http://127.0.0.1:${process.env.ARGOS_WEB_TEST_PORT || 3100}`,
      },
    });
  });
}

const VIEWPORTS: Array<{ name: string; width: number; height: number }> = [
  { name: "1440x1000", width: 1440, height: 1000 },
  { name: "768x1000", width: 768, height: 1000 },
  { name: "390x844", width: 390, height: 844 },
  { name: "320x800", width: 320, height: 800 },
];

async function emulateTransparency(
  session: CDPSession,
  value: "no-preference" | "reduce",
) {
  await session.send("Emulation.setEmulatedMedia", {
    features: [
      { name: "prefers-reduced-motion", value: "reduce" },
      { name: "prefers-reduced-transparency", value },
    ],
  });
}

for (const vp of VIEWPORTS) {
  test(`bulletin pressable article card @ ${vp.name}`, async ({ page }) => {
    await page.setViewportSize({ width: vp.width, height: vp.height });
    await mockApi(page);
    const session = await page.context().newCDPSession(page);
    await emulateTransparency(session, "no-preference");
    await page.goto("/");
    const launcher = page.getByRole("button", {
      name: "Open Intel",
      exact: true,
    });
    await expect(launcher).toBeVisible();
    await expect(
      page.getByLabel("API connected", { exact: true }),
    ).toBeVisible();
    await page.screenshot({
      path: `${CAPTURE}/${vp.name}-home-ready.png`,
    });
    await launcher.click();
    await page.mouse.move(0, 0);
    const card = page
      .getByRole("button", { name: /^Open Focus Brief:/ })
      .first();
    const header = page.locator("#intel-bulletin-header");
    const taskbar = page.locator("#intel-status-bar");
    const filter = page.getByRole("button", {
      name: "Refresh feed",
      exact: true,
    });
    const search = page.getByRole("textbox", {
      name: "Search articles and actors",
    });
    await expect(card).toBeVisible();
    const surfaces = [header, taskbar, card, filter, search];
    const measureSurfaces = () =>
      Promise.all(surfaces.map((surface) => surface.boundingBox()));
    const taskbarPaint = () =>
      taskbar.evaluate((element) => {
        const paint = getComputedStyle(element, "::before");
        return [paint.backgroundColor, paint.backdropFilter];
      });
    await expect(header).toHaveCSS(
      "background-color",
      "rgba(11, 15, 20, 0.82)",
    );
    await expect(header).toHaveCSS("backdrop-filter", "blur(10px)");
    await expect
      .poll(taskbarPaint)
      .toEqual(["rgba(11, 15, 20, 0.82)", "blur(10px)"]);
    await expect(card).toHaveCSS("background-color", "rgba(11, 15, 20, 0.8)");
    await expect(card).toHaveCSS("border-top-color", "rgb(43, 52, 63)");
    await expect(card).toHaveCSS("box-shadow", /inset/);
    await expect(
      page.getByRole("button", { name: "Bulletin", exact: true }),
    ).toHaveCSS("background-color", "rgb(36, 29, 18)");
    for (const control of [filter, search]) {
      await expect(control).toHaveCSS(
        "background-color",
        "rgba(18, 24, 32, 0.82)",
      );
      await expect(control).toHaveCSS("border-top-color", "rgb(100, 117, 135)");
      await expect(control).toHaveCSS("backdrop-filter", "none");
    }
    await page.screenshot({
      path: `${CAPTURE}/${vp.name}-bulletin-quiet-action.png`,
      fullPage: true,
    });
    await page.screenshot({
      path: `${CAPTURE}/${vp.name}-bulletin-glass-canvas.png`,
    });
    await card.hover();
    await expect(card).toHaveCSS("background-color", "rgba(25, 33, 43, 0.88)");
    await expect(card).toHaveCSS("backdrop-filter", "blur(12px)");

    await page.getByRole("button", { name: /^Retained day / }).click();
    const menu = page.getByRole("menu");
    await expect(menu).toHaveCSS("backdrop-filter", "blur(16px)");
    await expect(menu).toHaveCSS("background-color", "rgba(25, 33, 43, 0.88)");
    await expect(menu).toHaveCSS("box-shadow", /inset/);
    const day = menu.getByRole("button", { name: "Day 2026-10-25" });
    await expect(day).toHaveCSS("background-color", "rgb(36, 29, 18)");
    const bounds = (await day.boundingBox())!;
    expect(bounds.x).toBeGreaterThanOrEqual(0);
    expect(bounds.x + bounds.width).toBeLessThanOrEqual(vp.width);
    expect(
      await day
        .getByText("2026-10-25")
        .evaluate((element) => element.scrollWidth <= element.clientWidth),
    ).toBe(true);
    await page.screenshot({
      path: `${CAPTURE}/${vp.name}-bulletin-glass-day-menu.png`,
    });
    const surfaceBounds = await measureSurfaces();

    await emulateTransparency(session, "reduce");
    await expect(menu).toHaveCSS("backdrop-filter", "none");
    await expect(menu).toHaveCSS("background-color", "rgb(25, 33, 43)");
    await expect(menu).toHaveCSS("border-top-color", "rgb(100, 117, 135)");
    expect(await day.boundingBox()).toEqual(bounds);
    expect(await measureSurfaces()).toEqual(surfaceBounds);
    await page.screenshot({
      path: `${CAPTURE}/${vp.name}-bulletin-opaque-day-menu.png`,
    });
    await day.click();
    await expect(menu).toHaveCount(0);
    await page.mouse.move(0, 0);
    await expect(header).toHaveCSS("background-color", "rgb(11, 15, 20)");
    await expect(header).toHaveCSS("backdrop-filter", "none");
    await expect.poll(taskbarPaint).toEqual(["rgb(11, 15, 20)", "none"]);
    await expect(card).toHaveCSS("background-color", "rgb(11, 15, 20)");
    await expect(card).toHaveCSS("border-top-color", "rgb(100, 117, 135)");
    for (const control of [filter, search]) {
      await expect(control).toHaveCSS("background-color", "rgb(18, 24, 32)");
    }
    await page.screenshot({
      path: `${CAPTURE}/${vp.name}-bulletin-opaque-canvas.png`,
    });

    await page
      .getByRole("button", { name: "Background jobs", exact: true })
      .click();
    await expect(menu).toHaveCSS("backdrop-filter", "none");
    await expect(menu).toHaveCSS("background-color", "rgb(25, 33, 43)");
    const jobMenuBounds = (await menu.boundingBox())!;
    expect(jobMenuBounds.x).toBeGreaterThanOrEqual(0);
    expect(jobMenuBounds.x + jobMenuBounds.width).toBeLessThanOrEqual(vp.width);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await session.detach();
  });

  test(`focus brief: primary ensure + related leads + default button vs panel @ ${vp.name}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: vp.width, height: vp.height });
    await mockApi(page);
    const session = await page.context().newCDPSession(page);
    await emulateTransparency(session, "no-preference");
    await page.goto("/");
    await page.getByRole("button", { name: "Open Intel", exact: true }).click();
    await page
      .getByRole("button", { name: /^Open Focus Brief:/ })
      .first()
      .click();
    const panel = page.locator("#intel-brief");
    await expect(panel).toHaveCSS("background-color", "rgba(11, 15, 20, 0.8)");
    await expect(panel).toHaveCSS("backdrop-filter", "blur(12px)");
    await expect(panel).toHaveCSS("border-top-color", "rgb(43, 52, 63)");
    const ensure = page.getByRole("button", { name: "Ensure / resume brief" });
    const reload = page.getByRole("button", {
      name: "Reload article / new generation",
    });
    for (const button of [ensure, reload]) {
      await expect(button).toBeVisible();
      await expect(button).toHaveCSS("box-shadow", /inset/);
      await expect(button.locator('[aria-hidden="true"]')).toHaveCSS(
        "background-image",
        /^url\(/,
      );
    }
    await page.screenshot({
      path: `${CAPTURE}/${vp.name}-focus-brief-primary-related.png`,
      fullPage: true,
    });
    await ensure.evaluate((button) =>
      button.scrollIntoView({ block: "center" }),
    );
    await expect(ensure).toBeInViewport();
    await expect(reload).toBeInViewport();
    await page.screenshot({
      path: `${CAPTURE}/${vp.name}-focus-brief-committing-actions.png`,
    });
    const panelBounds = await panel.boundingBox();
    await emulateTransparency(session, "reduce");
    await expect(panel).toHaveCSS("background-color", "rgb(11, 15, 20)");
    await expect(panel).toHaveCSS("backdrop-filter", "none");
    await expect(panel).toHaveCSS("border-top-color", "rgb(100, 117, 135)");
    expect(await panel.boundingBox()).toEqual(panelBounds);
    await page.screenshot({
      path: `${CAPTURE}/${vp.name}-focus-brief-opaque-panel.png`,
    });
    await session.detach();
  });

  test(`topology empty-selection state @ ${vp.name}`, async ({ page }) => {
    await page.setViewportSize({ width: vp.width, height: vp.height });
    await mockApi(page);
    await page.goto("/");
    await page.getByRole("button", { name: "Open Intel", exact: true }).click();
    await page
      .getByRole("button", { name: /^Open Focus Brief:/ })
      .first()
      .click();
    await page.getByRole("button", { name: "Launch 3D topology" }).click();
    await expect(
      page.getByRole("button", { name: "Return to Brief" }),
    ).toBeVisible();
    if (vp.width <= 390) {
      const workspace = page.locator("#intel-topology-workspace");
      const bounds = (await workspace.boundingBox())!;
      const toolbar = await Promise.all(
        ["Neighbors", "Nodes", "Connections"].map(async (name) => {
          const button = workspace.getByRole("button", { name, exact: true });
          await expect(button).toBeInViewport({ ratio: 1 });
          const box = (await button.boundingBox())!;
          expect(box.x).toBeGreaterThanOrEqual(bounds.x);
          expect(box.x + box.width).toBeLessThanOrEqual(
            bounds.x + bounds.width,
          );
          return box;
        }),
      );
      if (vp.width === 320) expect(toolbar[2].y).toBeGreaterThan(toolbar[0].y);

      const graph = page.locator("#intel-topology-viewport");
      const focusedTile = '[data-role="focus"] [data-tile="node"]';
      // The route and its toolbar mount before the lazy graph is rendered.
      await expect
        .poll(
          () =>
            graph.evaluate((element, selector) => {
              const tile = element.querySelector(selector);
              if (!tile) return false;
              const frame = element.getBoundingClientRect();
              const focus = tile.getBoundingClientRect();
              return (
                Math.abs(
                  focus.x + focus.width / 2 - frame.x - frame.width / 2,
                ) <= 8 &&
                Math.abs(
                  focus.y + focus.height / 2 - frame.y - frame.height / 2,
                ) <= 8
              );
            }, focusedTile),
          { timeout: 15_000 },
        )
        .toBe(true);
      await expect(graph.locator(focusedTile)).toBeInViewport({ ratio: 1 });
    }
    await page.screenshot({
      path: `${CAPTURE}/${vp.name}-topology-node-list.png`,
      fullPage: vp.width > 390,
    });
  });

  test(`brief evidence and entity affordance @ ${vp.name}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: vp.width, height: vp.height });
    await mockApi(page);
    await page.goto("/");
    await page.getByRole("button", { name: "Open Intel", exact: true }).click();
    await page
      .getByRole("button", { name: /^Open Focus Brief:/ })
      .first()
      .click();
    await expect(
      page.getByRole("heading", { name: "IDENTIFIED ACTORS", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", {
        name: "Explore actor Port authority",
        exact: true,
      }),
    ).toBeVisible();
    await page.screenshot({
      path: `${CAPTURE}/${vp.name}-narrow-tabs-default-button.png`,
      fullPage: true,
    });
  });
}
