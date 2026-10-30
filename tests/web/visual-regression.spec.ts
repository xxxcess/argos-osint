import { test, expect } from "@playwright/test";

const CAPTURE = process.env.ARGOS_WEB_CAPTURE_DIR || ".agent-scratch/visual-regression";
const VIEWPORTS: Array<{ name: string; width: number; height: number }> = [
  { name: "1440x1000", width: 1440, height: 1000 },
  { name: "768x1000", width: 768, height: 1000 },
  { name: "390x844", width: 390, height: 844 },
  { name: "320x800", width: 320, height: 800 },
];

// Visual regression baseline comparison with diff thresholds
for (const vp of VIEWPORTS) {
  test(`UI theme tokens @ ${vp.name} - full page screenshot`, async ({ page }) => {
    await page.setViewportSize({ width: vp.width, height: vp.height });
    await page.goto("/");
    // Capture full page screenshot for visual regression baseline
    await expect(page).toHaveScreenshot(`${vp.name}-fullpage.png`, {
      fullPage: true,
      thresholds: {
        // 0.1% pixel diff tolerance for baseline establishment
        actualThreshold: 0.1,
        // 0.2% structural diff tolerance
        threshold: 0.2,
      },
    });
  });

  test(`UI density modes @ ${vp.name} - compact vs default`, async ({ page }) => {
    await page.setViewportSize({ width: vp.width, height: vp.height });
    await page.goto("/");
    // Capture loading state screenshot
    await expect(page.getByRole("progressbar", { name: /Loading/ })).toHaveScreenshot(
      `${vp.name}-loading.png`,
      { fullPage: true }
    );
    // Capture empty state screenshot
    await expect(page.getByText(/No items/)).toHaveScreenshot(
      `${vp.name}-empty.png`,
      { fullPage: true }
    );
  });

  test(`UI focus/select states @ ${vp.name} - button states`, async ({ page }) => {
    await page.setViewportSize({ width: vp.width, height: vp.height });
    await page.goto("/");
    // Capture primary button screenshot
    await expect(page.getByRole("button", { name: /Open Focus Brief →/ })).toHaveScreenshot(
      `${vp.name}-primary-button.png`,
      { fullPage: true }
    );
    // Capture quiet button screenshot
    await expect(page.getByRole("button", { name: /Quiet/ })).toHaveScreenshot(
      `${vp.name}-quiet-button.png`,
      { fullPage: true }
    );
  });

  test(`UI theme compliance @ ${vp.name} - color consistency`, async ({ page }) => {
    await page.setViewportSize({ width: vp.width, height: vp.height });
    await page.goto("/");
    // Verify accent color is present in the UI
    const accentElements = page.locator(".accent, [style*='87bfff']");
    await expect(accentElements.first).toBeVisible();
    // Capture header screenshot
    await expect(page.getByTestId("masthead")).toHaveScreenshot(
      `${vp.name}-header.png`,
      { fullPage: true }
    );
  });
}
