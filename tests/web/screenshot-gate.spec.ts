import { test, expect, type Page } from "@playwright/test";

const CAPTURE = process.env.ARGOS_WEB_CAPTURE_DIR || ".agent-scratch/tester-gate/screenshots";
const API = "http://127.0.0.1:8080";

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

function mockApi(page: Page) {
  page.route(`${API}/api/v1/**`, async (route) => {
    const url = new URL(route.request().url());
    const path = url.pathname.replace("/api/v1/", "");
    const cursor = url.searchParams.get("cursor");
    if (path === "health/ready") {
      await route.fulfill({ json: { service: "argos-intel", api_version: 1, ready: true, storage: true, instance_id: "fixture-instance" }, headers: { "access-control-allow-origin": "http://127.0.0.1:3100" } });
      return;
    }
    if (path === "days") {
      await route.fulfill({ json: { days: ["2026-10-25"] }, headers: { "access-control-allow-origin": "http://127.0.0.1:3100" } });
      return;
    }
    if (path === "articles") {
      const next = cursor ? null : "retry-cursor";
      await route.fulfill({
        json: {
          rows: cursor ? [] : [{ ...article, image_url: "" }, { ...article, id: "a2", title: "Earlier shipping agreement", image_url: "" }],
          next_cursor: next,
          revision: "r1",
        },
        headers: { "access-control-allow-origin": "http://127.0.0.1:3100" },
      });
      return;
    }
    if (path === "articles/detail") {
      await route.fulfill({ json: { article, source_revision: "fixture-source", actors: [{ id: "actor1", display_name: "Port authority", type_hint: "organization" }], claims: [{ id: "c1", text: "Shipping talks resumed on Monday.", confidence: 0.8, admiralty: "B2", classification: "fact" }], body: { body_markdown: "The full retained article describes the negotiations and their regional context." }, lifecycle: "active", status: "Published", revision: "r2", publication: [1, { claims: [] }, { inferences: [{ text: "Access may improve." }] }], relations: [] }, headers: { "access-control-allow-origin": "http://127.0.0.1:3100" } });
      return;
    }
    if (path === "articles/related") {
      await route.fulfill({ json: { rows: [{ article: { ...article, id: "a2", title: "Earlier shipping agreement" }, basis: ["shared actor"], tentative: true }], next_cursor: null }, headers: { "access-control-allow-origin": "http://127.0.0.1:3100" } });
      return;
    }
    if (path === "topology") {
      await route.fulfill({ json: { root_label: "Port authority", nodes: [{ id: "n1", label: "Port authority", kind: "entity", detail: "Recorded actor", topology_seed: { Actor: "actor1" } }, { id: "n2", label: "Regional shipping", kind: "topic", detail: "Recorded topic", topology_seed: { Topic: "shipping" } }], edges: [{ source: "n1", target: "n2" }], findings: { gaps: [] }, has_continuation: false }, headers: { "access-control-allow-origin": "http://127.0.0.1:3100" } });
      return;
    }
    if (path === "brief/ensure") {
      await route.fulfill({ json: { job_id: "job1" }, headers: { "access-control-allow-origin": "http://127.0.0.1:3100" } });
      return;
    }
    if (route.request().method() === "POST") {
      await route.fulfill({ json: { job_id: "job1" }, headers: { "access-control-allow-origin": "http://127.0.0.1:3100" } });
      return;
    }
    await route.fulfill({ json: {}, headers: { "access-control-allow-origin": "http://127.0.0.1:3100" } });
  });
}

const VIEWPORTS: Array<{ name: string; width: number; height: number }> = [
  { name: "1440x1000", width: 1440, height: 1000 },
  { name: "768x1000", width: 768, height: 1000 },
  { name: "390x844", width: 390, height: 844 },
  { name: "320x800", width: 320, height: 800 },
];

for (const vp of VIEWPORTS) {
  test(`bulletin article card + quiet action @ ${vp.name}`, async ({ page }) => {
    await page.setViewportSize({ width: vp.width, height: vp.height });
    await mockApi(page);
    await page.goto("/");
    await expect(page.getByRole("button", { name: "Open Focus Brief \u2192" })).toBeVisible();
    await page.screenshot({ path: `${CAPTURE}/${vp.name}-bulletin-quiet-action.png`, fullPage: true });
  });

  test(`focus brief: primary ensure + related leads + default button vs panel @ ${vp.name}`, async ({ page }) => {
    await page.setViewportSize({ width: vp.width, height: vp.height });
    await mockApi(page);
    await page.goto("/");
    await page.getByRole("button", { name: "Open Focus Brief \u2192" }).click();
    await expect(page.getByRole("button", { name: "Ensure / resume brief" })).toBeVisible();
    await page.screenshot({ path: `${CAPTURE}/${vp.name}-focus-brief-primary-related.png`, fullPage: true });
  });

  test(`topology node-list rows @ ${vp.name}`, async ({ page }) => {
    await page.setViewportSize({ width: vp.width, height: vp.height });
    await mockApi(page);
    await page.goto("/");
    await page.getByRole("button", { name: "Topology", exact: true }).click();
    await expect(page.getByRole("heading", { name: "No topology selected" })).toBeVisible();
    await page.screenshot({ path: `${CAPTURE}/${vp.name}-topology-node-list.png`, fullPage: true });
  });

  test(`narrow tabs default-button vs panel affordance @ ${vp.name}`, async ({ page }) => {
    await page.setViewportSize({ width: vp.width, height: vp.height });
    await mockApi(page);
    await page.goto("/");
    await page.getByRole("button", { name: "Open Focus Brief \u2192" }).click();
    await expect(page.getByRole("button", { name: "Extracted", exact: true })).toBeVisible();
    await page.screenshot({ path: `${CAPTURE}/${vp.name}-narrow-tabs-default-button.png`, fullPage: true });
  });
}
