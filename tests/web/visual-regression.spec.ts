// @tag:testing — deterministic export screenshots with observable UI assertions.
import { test, expect } from "@playwright/test";

const article = {
  id: "visual-article",
  run_id: "visual-run",
  title: "Harbor negotiations enter a new phase",
  description: "Officials discuss regional shipping access.",
  source_name: "Fixture Wire",
  source_domain: "example.test",
  published_at: "2026-10-25T12:00:00Z",
  category: "geopolitical",
  image_url: "",
  url: "https://example.test/news",
  author: "Fixture reporter",
};
for (const width of [1440, 768, 390, 320]) {
  test(`Bulletin visual contract at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 1000 });
    await page.route("http://127.0.0.1:8080/api/v1/**", async (route) => {
      const url = new URL(route.request().url());
      const path = url.pathname.replace("/api/v1/", "");
      let body: unknown;
      if (path === "health/ready")
        body = {
          service: "argos-intel",
          api_version: 1,
          ready: true,
          storage: true,
          instance_id: "visual-instance",
        };
      else if (path === "days") body = { days: ["2026-10-25"] };
      else if (path === "articles")
        body = {
          rows: [article],
          next_cursor: null,
          revision: "visual-revision",
        };
      else throw new Error(`Unexpected visual fixture request: ${path}`);
      await route.fulfill({ json: body });
    });
    await page.goto("/");
    await expect(page.getByText(article.title)).toBeVisible();
    await expect(page.getByText("● API connected")).toBeVisible();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await expect(page).toHaveScreenshot(`bulletin-${width}.png`, {
      fullPage: true,
      animations: "disabled",
      maxDiffPixelRatio: 0.001,
      threshold: 0.2,
    });
    await page.getByRole("button", { name: "Topology", exact: true }).focus();
    await expect(
      page.getByRole("button", { name: "Topology", exact: true }),
    ).toBeFocused();
    await expect(page).toHaveScreenshot(`bulletin-focus-${width}.png`, {
      fullPage: true,
      animations: "disabled",
      maxDiffPixelRatio: 0.001,
      threshold: 0.2,
    });
  });
}
