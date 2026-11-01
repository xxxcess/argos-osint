import { installLiveFixture } from "./live-fixture";
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
    await installLiveFixture(page);
    await page.setViewportSize({ width, height: 1000 });
    // Freeze Date, not timers: the real status-bar interval must still update.
    await page.clock.setFixedTime(new Date("2026-10-31T10:00:54Z"));
    await page.route(
      `http://127.0.0.1:${process.env.ARGOS_WEB_TEST_PORT || 3100}/api/v1/**`,
      async (route) => {
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
      },
    );
    await page.goto("/");
    await page.getByRole("button", { name: "Open Intel", exact: true }).click();
    await expect(page.getByText(article.title)).toBeVisible();
    await expect(page.getByLabel("API connected", { exact: true })).toHaveText(
      "● API_OK",
    );
    await expect(
      page.getByRole("button", { name: "Desktop", exact: true }),
    ).toBeEnabled();
    await expect(
      page.getByRole("button", { name: "Bulletin", exact: true }),
    ).toHaveAttribute("aria-pressed", "true");
    if (width >= 1100) {
      const clock = await page.evaluate(() =>
        new Date().toLocaleTimeString("en-US"),
      );
      await expect(page.getByText(clock, { exact: true })).toBeVisible();
    }
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
    await page
      .getByRole("button", { name: "Geopolitical", exact: true })
      .focus();
    await expect(
      page.getByRole("button", { name: "Geopolitical", exact: true }),
    ).toBeFocused();
    const interactionColors = await page.evaluate(() => ({
      focus: getComputedStyle(document.activeElement!).outlineColor,
      selection: getComputedStyle(document.body, "::selection").backgroundColor,
    }));
    expect(interactionColors.focus).toBe("rgb(127, 224, 180)");
    expect(interactionColors.selection).toBe("rgb(46, 56, 80)");
    expect(interactionColors.selection).not.toBe(interactionColors.focus);
    await expect(page).toHaveScreenshot(`bulletin-focus-${width}.png`, {
      fullPage: true,
      animations: "disabled",
      maxDiffPixelRatio: 0.001,
      threshold: 0.2,
    });
  });
}
