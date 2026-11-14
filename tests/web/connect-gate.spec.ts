import { test, expect } from "@playwright/test";

test.use({ reducedMotion: "reduce" });

test("deep links, prefetch/RSC, REST and exports are denied before connection", async ({
  page,
  request,
}) => {
  await page.goto("/brief?run=fixture&id=article");
  await expect(
    page.getByRole("heading", { name: "Connect to Intel Web" }),
  ).toBeVisible();
  expect(new URL(page.url()).pathname).toBe("/connect");
  expect(new URL(page.url()).searchParams.get("return")).toBe(
    "/brief?run=fixture&id=article",
  );
  await expect(page.locator("#intel-web-workspace")).toHaveCount(0);
  for (const path of [
    "/api/v1/articles",
    "/api/v1/jobs/detail?id=guessed",
    "/api/v1/export",
  ]) {
    const response = await request.get(path);
    expect(response.status()).toBe(401);
  }
  const rsc = await request.get("/topology?seed=fixture&_rsc=prefetch", {
    headers: { rsc: "1", "next-router-prefetch": "1" },
    maxRedirects: 0,
  });
  expect(rsc.status()).toBe(307);
  expect(await rsc.text()).not.toContain("intel-web-workspace");
});

test("unknown registration cannot self-register; valid connection stays HttpOnly", async ({
  page,
}) => {
  await page.goto("/connect");
  const input = page.getByRole("textbox", { name: "Connect ID", exact: true });
  const connect = page.getByRole("button", {
    name: "Connect registered workstation",
  });
  await input.fill("fixture-workstation-unknown");
  await connect.click();
  await expect(
    page.getByRole("alert").filter({ hasText: "Connection required" }),
  ).toContainText("Connection required or registration no longer available");
  await expect(page.locator("#intel-web-workspace")).toHaveCount(0);
  await input.fill("fixture-workstation-research");
  // Read the real upstream response before the form's successful full
  // navigation disposes Chrome's network body. Keep the normal server guard.
  let acceptedBody: Record<string, unknown> | undefined;
  await page.route("**/api/connect", async (route) => {
    if (route.request().method() !== "POST") {
      await route.continue();
      return;
    }
    const response = await route.fetch();
    acceptedBody = await response.json();
    await route.fulfill({ response });
  });
  await connect.click();
  await expect(
    page.getByText("Workstation: fixture-workstation-research", {
      exact: true,
    }),
  ).toBeVisible();
  expect(acceptedBody).toBeDefined();
  expect(acceptedBody?.session_token).toBeUndefined();
  const cookie = (await page.context().cookies()).find(
    (item) => item.name === "fleetworks_connect_v2",
  )!;
  expect(cookie.httpOnly).toBe(true);
  expect(cookie.sameSite).toBe("Strict");
  expect(await page.evaluate(() => document.cookie)).not.toContain(
    "fleetworks_connect_v2",
  );
  const html = await (await page.request.get("/")).text();
  expect(html).not.toContain(cookie.value);
});

test("switching in another tab invalidates the old view and stale mutations receive 409", async ({
  page,
  context,
}) => {
  await page.goto("/connect");
  await page
    .getByRole("textbox", { name: "Connect ID", exact: true })
    .fill("fixture-workstation-research");
  await page
    .getByRole("button", { name: "Connect registered workstation" })
    .click();
  await expect(
    page.getByText("Workstation: fixture-workstation-research", {
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Open Intel", exact: true }),
  ).toBeVisible();
  const previous = await (await page.request.get("/api/connect")).json();
  const second = await context.newPage();
  await second.goto("/connect");
  await second
    .getByRole("textbox", { name: "Connect ID", exact: true })
    .fill("fixture-workstation-review");
  await second
    .getByRole("button", { name: "Connect registered workstation" })
    .click();
  await expect(
    second.getByText("Workstation: fixture-workstation-review", {
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Reconnect this tab" }),
  ).toBeVisible();
  await expect(page.locator("#intel-web-workspace")).toHaveCount(0);
  const origin = new URL(page.url()).origin;
  const denied = await page.request.post("/api/v1/brief/ensure", {
    headers: {
      origin,
      "x-fleetworks-expected-context-revision": String(
        previous.context.context_revision,
      ),
    },
    data: { run: "fixture", id: "article" },
  });
  expect(denied.status()).toBe(409);
});

test("disconnect revokes the session and removes protected UI and live state", async ({
  page,
}) => {
  await page.goto("/connect");
  await page
    .getByRole("textbox", { name: "Connect ID", exact: true })
    .fill("fixture-workstation-research");
  await page
    .getByRole("button", { name: "Connect registered workstation" })
    .click();
  await expect(
    page.getByRole("button", { name: "Disconnect Intel Web" }),
  ).toBeVisible();
  // The connection bar is server-rendered before its event handlers hydrate.
  // API_OK requires the mounted client query to finish, so the click below
  // exercises the actual logout handler rather than an inert SSR button.
  await expect(page.getByLabel("API connected", { exact: true })).toHaveText(
    "● API_OK",
  );
  const priorCookie = (await page.context().cookies()).find(
    (cookie) => cookie.name === "fleetworks_connect_v2",
  );
  expect(priorCookie).toBeDefined();
  const [logout] = await Promise.all([
    page.waitForResponse(
      (response) =>
        new URL(response.url()).pathname === "/api/connect/logout" &&
        response.request().method() === "POST",
    ),
    page.getByRole("button", { name: "Disconnect Intel Web" }).click(),
  ]);
  expect(logout.status()).toBe(200);
  await expect(
    page.getByRole("heading", { name: "Reconnect this tab" }),
  ).toBeVisible();
  await expect(page.locator("#intel-web-workspace")).toHaveCount(0);
  expect((await page.request.get("/api/v1/articles")).status()).toBe(401);
  // Cookie removal alone cannot prove server revocation. Replaying the original
  // opaque cookie must also fail at the same authoritative data boundary.
  expect(
    (
      await page.request.get("/api/v1/articles", {
        headers: { cookie: `${priorCookie!.name}=${priorCookie!.value}` },
      })
    ).status(),
  ).toBe(401);
});
