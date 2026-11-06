import React from "react";
import { beforeEach, describe, expect, it } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { BackendProvider, type Article } from "@argos/api-client";
import { IntelApp } from "../src";
import { mswServer, testBase } from "../../../vitest.setup";
import { sheetProp } from "../../../vitest.test-support";

// @tag:testing — real IntelApp + Query, isolated offline HTTP boundary.
const row: Article = {
  id: "article-retained-id",
  run_id: "run-one",
  title: "Harbor access report",
  description: "Retained source description.",
  category: "governance",
  source_name: "Fixture Wire",
  source_domain: "example.test",
  url: "https://example.test/report",
  published_at: "2026-10-29T08:00:00Z",
  image_url: "",
  author: "Fixture author",
};
const requests: {
  category: string | null;
  search: string | null;
  cursor: string | null;
}[] = [];
function App() {
  return (
    <BackendProvider
      settings={{
        subscribe: () => () => {},
        getSnapshot: () => testBase,
        getServerSnapshot: () => testBase,
        setBase: () => {},
        createRequestId: () => "fixture-request",
      }}
    >
      <IntelApp />
    </BackendProvider>
  );
}
beforeEach(() => {
  requests.length = 0;
  mswServer.use(
    http.get(`${testBase}/api/v1/health/ready`, () =>
      HttpResponse.json({
        ready: true,
        storage: true,
        service: "argos-intel",
        api_version: 1,
        instance_id: "bulletin-fixture",
      }),
    ),
    http.get(`${testBase}/api/v1/days`, () =>
      HttpResponse.json({ days: ["2026-10-29", "2026-10-28"] }),
    ),
    http.get(`${testBase}/api/v1/articles`, ({ request }) => {
      const params = new URL(request.url).searchParams;
      requests.push({
        category: params.get("category"),
        search: params.get("search"),
        cursor: params.get("cursor"),
      });
      return HttpResponse.json({
        rows: params.get("search") === "missing" ? [] : [row],
        next_cursor: null,
        revision: "fixture",
      });
    }),
    http.get(`${testBase}/api/v1/articles/detail`, () =>
      HttpResponse.json({
        article: row,
        actors: [],
        claims: [],
        body: null,
        output: null,
        lifecycle: "active",
        status: "Published",
        revision: "fixture",
        publication: null,
        relations: [],
      }),
    ),
  );
});

describe("Nexus Desk Bulletin", () => {
  it("keeps cached feed rows visible without loaders during refresh", async () => {
    let reads = 0;
    let release!: () => void;
    const refreshing = new Promise<void>((resolve) => {
      release = resolve;
    });
    mswServer.use(
      http.get(`${testBase}/api/v1/articles`, async () => {
        if (++reads > 1) await refreshing;
        return HttpResponse.json({
          rows: [row],
          next_cursor: null,
          revision: "cached",
        });
      }),
    );
    const user = userEvent.setup();
    render(<App />);
    await user.click(await screen.findByRole("button", { name: "Open Intel" }));
    await screen.findByRole("button", {
      name: `Open Focus Brief: ${row.title}`,
    });
    try {
      await user.click(screen.getByRole("button", { name: "Refresh feed" }));
      await waitFor(() => expect(reads).toBe(2));
      expect(
        screen.getByRole("button", { name: `Open Focus Brief: ${row.title}` }),
      ).toBeVisible();
      expect(
        screen.queryByRole("status", { name: "Loading bulletin…" }),
      ).not.toBeInTheDocument();
      expect(
        screen.queryByRole("progressbar", { name: "Refreshing bulletin…" }),
      ).not.toBeInTheDocument();
    } finally {
      release();
    }
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Refresh feed" }),
      ).toBeEnabled(),
    );
  });

  it("selects categories, debounces search and resets a verified empty result", async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.click(await screen.findByRole("button", { name: "Open Intel" }));
    await screen.findByRole("button", {
      name: `Open Focus Brief: ${row.title}`,
    });
    await user.click(screen.getByRole("button", { name: /^Economic$/ }));
    await waitFor(() => expect(requests.at(-1)?.category).toBe("economic"));
    const search = screen.getByRole("textbox", { name: "Search articles" });
    await user.type(search, "missing");
    expect(requests.filter((request) => request.search)).toEqual([]);
    await screen.findByRole("heading", { name: "NO MATCHING INTELLIGENCE" });
    expect(
      requests
        .filter((request) => request.search)
        .map((request) => request.search),
    ).toEqual(["missing"]);
    await user.click(screen.getByRole("button", { name: "Reset filters" }));
    await screen.findByRole("button", {
      name: `Open Focus Brief: ${row.title}`,
    });
    expect(search).toHaveValue("");
    expect(
      screen.getByRole("button", { name: /^Governance$/ }),
    ).toHaveAttribute("aria-pressed", "true");
  });

  it("moves category focus with arrows, selects with Enter and opens the whole article card", async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.click(await screen.findByRole("button", { name: "Open Intel" }));
    await screen.findByRole("button", {
      name: `Open Focus Brief: ${row.title}`,
    });
    await user.click(screen.getByRole("button", { name: /^Governance$/ }));
    await user.keyboard("{ArrowDown}");
    expect(screen.getByRole("button", { name: /^Security$/ })).toHaveFocus();
    expect(
      screen.getByRole("button", { name: /^Governance$/ }),
    ).toHaveAttribute("aria-pressed", "true");
    await user.keyboard("{Enter}");
    expect(screen.getByRole("button", { name: /^Security$/ })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(
      screen.queryByText(/COOLING DOWN|HEATING UP|TLP:/),
    ).not.toBeInTheDocument();
    expect(screen.queryByText(row.id)).not.toBeInTheDocument();
    expect(screen.getByText(/Fixture Wire/)).toBeVisible();
    await waitFor(() => expect(requests.at(-1)?.category).toBe("security"));
    const card = await screen.findByRole("button", {
      name: `Open Focus Brief: ${row.title}`,
    });
    await user.click(card);
    await waitFor(() => expect(card).not.toBeInTheDocument());
    await screen.findByRole("button", { name: "Return to Bulletin" });
    expect(screen.getByRole("heading", { name: row.title })).toBeVisible();
  });

  it("preserves opaque cursor pagination and deduplicates run/article occurrences", async () => {
    mswServer.use(
      http.get(`${testBase}/api/v1/articles`, ({ request }) => {
        const cursor = new URL(request.url).searchParams.get("cursor");
        requests.push({ category: "governance", search: "", cursor });
        return HttpResponse.json({
          rows: cursor ? [row, { ...row, run_id: "run-two" }] : [row],
          next_cursor: cursor ? null : "opaque+/cursor=",
          revision: "page",
        });
      }),
    );
    const user = userEvent.setup();
    render(<App />);
    await user.click(await screen.findByRole("button", { name: "Open Intel" }));
    await screen.findByRole("button", {
      name: `Open Focus Brief: ${row.title}`,
    });
    await user.click(
      screen.getByRole("button", { name: "Load more articles" }),
    );
    await waitFor(() =>
      expect(
        screen.getAllByRole("button", { name: /^Open Focus Brief:/ }),
      ).toHaveLength(2),
    );
    expect(
      screen.getAllByRole("button", { name: `Open Focus Brief: ${row.title}` }),
    ).toHaveLength(2);
    expect(requests.at(-1)?.cursor).toBe("opaque+/cursor=");
  });

  it("shows three pending skeletons instead of claiming an empty result", async () => {
    const user = userEvent.setup();
    let resolve!: (response: Response) => void;
    const pending = new Promise<Response>((done) => {
      resolve = done;
    });
    mswServer.use(http.get(`${testBase}/api/v1/articles`, () => pending));
    render(<App />);
    await user.click(await screen.findByRole("button", { name: "Open Intel" }));
    await screen.findByRole("status", { name: "Loading bulletin…" });
    expect(screen.getAllByTestId("bulletin-skeleton")).toHaveLength(3);
    expect(
      screen.queryByRole("heading", { name: "NO MATCHING INTELLIGENCE" }),
    ).not.toBeInTheDocument();
    resolve(
      HttpResponse.json({ rows: [row], next_cursor: null, revision: "loaded" }),
    );
    await screen.findByRole("button", {
      name: `Open Focus Brief: ${row.title}`,
    });
    expect(
      screen.queryByRole("status", { name: "Loading bulletin…" }),
    ).not.toBeInTheDocument();
  });

  it("replaces the feed category stamp with an overlay date filter", async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.click(await screen.findByRole("button", { name: "Open Intel" }));
    const article = await screen.findByRole("button", {
      name: `Open Focus Brief: ${row.title}`,
    });
    expect(
      screen.queryByRole("heading", { name: /GEOPOLITICAL/ }),
    ).not.toBeInTheDocument();
    expect(screen.queryByText(/Updated /)).not.toBeInTheDocument();
    const day = screen.getByRole("button", { name: "Retained day 2026-10-29" });
    const articleTop = article.getBoundingClientRect().top;
    await user.click(day);
    expect(article.getBoundingClientRect().top).toBe(articleTop);
    expect(
      screen.getByRole("button", { name: "Day 2026-10-28" }),
    ).toBeVisible();
    const feed = document.getElementById("intel-article-feed");
    const overlay = document.getElementById("intel-day-overlay");
    expect(feed).toBeTruthy();
    expect(overlay).toBeTruthy();
    expect(feed?.contains(overlay)).toBe(true);
    expect(sheetProp(overlay!, "position")).toBe("absolute");
    await user.click(screen.getByRole("button", { name: "Day 2026-10-28" }));
    expect(
      screen.getByRole("button", { name: "Retained day 2026-10-28" }),
    ).toBeVisible();
    expect(screen.queryByRole("button", { name: "Day 2026-10-28" })).toBeNull();
  });

  it("retains a concrete retry after a failed Bulletin request", async () => {
    mswServer.use(
      http.get(`${testBase}/api/v1/articles`, () =>
        HttpResponse.json({ error: "Fixture unavailable" }, { status: 503 }),
      ),
    );
    const user = userEvent.setup();
    render(<App />);
    await user.click(await screen.findByRole("button", { name: "Open Intel" }));
    const retry = await screen.findByRole(
      "button",
      { name: "Retry bulletin" },
      { timeout: 5000 },
    );
    expect(screen.getByRole("alert")).toHaveTextContent("Fixture unavailable");
    expect(
      screen.queryByRole("heading", { name: "NO MATCHING INTELLIGENCE" }),
    ).not.toBeInTheDocument();
    mswServer.use(
      http.get(`${testBase}/api/v1/articles`, () =>
        HttpResponse.json({
          rows: [row],
          next_cursor: null,
          revision: "recovered",
        }),
      ),
    );
    await user.click(retry);
    await screen.findByRole("button", {
      name: `Open Focus Brief: ${row.title}`,
    });
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
});
