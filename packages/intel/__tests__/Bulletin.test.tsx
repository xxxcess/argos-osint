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
const searches: string[] = [];
const actorSearches: URLSearchParams[] = [];
const articleHit = {
  run_id: "run-one",
  article_id: "article-retained-id",
  title: "Harbor access report",
  description: "Retained source description.",
  excerpt: "Harbor access…",
  matched_field: "title",
  categories: ["governance"],
  taxonomy_version: "v1",
  admission_state: "admitted",
  publication_version: "v1",
  source_name: "Fixture Wire",
  source_domain: "example.test",
  published_at: "2026-10-29T08:00:00Z",
  url: "https://example.test/report",
};
const actorHit = (actor_id: string, label: string, extra = {}) => ({
  actor_id,
  label,
  projection_revision: 1,
  index_generation: "g1",
  score: 1,
  ...extra,
});
const actorPage = (rows: unknown[], state = "ready") => ({
  rows,
  index: { state },
  truncated: false,
  continuation: null,
});
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
  searches.length = 0;
  actorSearches.length = 0;
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
    http.get(`${testBase}/api/v1/articles/search`, ({ request }) => {
      searches.push(new URL(request.url).searchParams.get("query") || "");
      return HttpResponse.json({ hits: [], next_cursor: null, revision: "s" });
    }),
    http.get(`${testBase}/api/v1/actors/search`, ({ request }) => {
      actorSearches.push(new URL(request.url).searchParams);
      return HttpResponse.json(actorPage([]));
    }),
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

  it("selects categories, searches only on submit and clears back to the feed", async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.click(await screen.findByRole("button", { name: "Open Intel" }));
    await screen.findByRole("button", {
      name: `Open Focus Brief: ${row.title}`,
    });
    await user.click(
      screen.getByRole("button", { name: /^Economy & Supply Chains$/ }),
    );
    await waitFor(() => expect(requests.at(-1)?.category).toBe("economic"));
    const search = screen.getByRole("textbox", {
      name: "Search article titles and descriptions",
    });
    await user.type(search, "missing");
    expect(searches).toEqual([]);
    await user.keyboard("{Enter}");
    await screen.findByRole("heading", { name: "NO MATCHING ARTICLES" });
    expect(searches).toEqual(["missing"]);
    expect(requests.filter((request) => request.search)).toEqual([]);
    await user.click(screen.getByRole("button", { name: "Clear search" }));
    await screen.findByRole("button", {
      name: `Open Focus Brief: ${row.title}`,
    });
    expect(search).toHaveValue("");
    // Clearing the search keeps the chosen category; only Reset filters resets it.
    expect(
      screen.getByRole("button", { name: /^Economy & Supply Chains$/ }),
    ).toHaveAttribute("aria-pressed", "true");
  });

  it("shows hits with their matched field and says when the candidate window was filled", async () => {
    mswServer.use(
      http.get(`${testBase}/api/v1/articles/search`, () =>
        HttpResponse.json({
          hits: [
            {
              run_id: "run-one",
              article_id: "article-retained-id",
              title: "Harbor access report",
              description: "Retained source description.",
              excerpt: "Harbor access…",
              matched_field: "title",
              categories: ["governance"],
              taxonomy_version: "v1",
              admission_state: "admitted",
              publication_version: "v1",
              source_name: "Fixture Wire",
              source_domain: "example.test",
              published_at: "2026-10-29T08:00:00Z",
              url: "https://example.test/report",
            },
          ],
          next_cursor: null,
          revision: "s",
          truncated: true,
        }),
      ),
    );
    const user = userEvent.setup();
    render(<App />);
    await user.click(await screen.findByRole("button", { name: "Open Intel" }));
    const search = await screen.findByRole("textbox", {
      name: "Search article titles and descriptions",
    });
    await user.type(search, "harbor{Enter}");
    await screen.findByText("Matched title");
    expect(
      screen.getByText(
        "Showing the best matches only. Narrow the search to see others.",
      ),
    ).toBeVisible();
  });

  it("lists actors apart from articles and opens each by its canonical id, then Back restores the search", async () => {
    const details: (string | null)[] = [];
    mswServer.use(
      http.get(`${testBase}/api/v1/articles/search`, () =>
        HttpResponse.json({
          hits: [articleHit],
          next_cursor: null,
          revision: "s",
        }),
      ),
      http.get(`${testBase}/api/v1/actors/search`, ({ request }) => {
        actorSearches.push(new URL(request.url).searchParams);
        return HttpResponse.json(
          actorPage([
            actorHit("actor:one", "Orion Group", {
              actor_type: "organization",
              countries: ["NO"],
            }),
            actorHit("actor:two", "orion group", {
              matched_alias: "Orion",
              countries: ["SE"],
            }),
            actorHit("actor:one", "Orion Group (repeat)"),
            actorHit("", "No identity"),
          ]),
        );
      }),
      http.get(`${testBase}/api/v1/actors/detail`, ({ request }) => {
        details.push(new URL(request.url).searchParams.get("id"));
        return HttpResponse.json({ error: "missing" }, { status: 404 });
      }),
    );
    const user = userEvent.setup();
    render(<App />);
    await user.click(await screen.findByRole("button", { name: "Open Intel" }));
    const search = await screen.findByRole("textbox", {
      name: "Search article titles and descriptions",
    });
    await user.type(search, "orion");
    expect(actorSearches).toEqual([]);
    await user.keyboard("{Enter}");
    await screen.findByRole("heading", { name: "Actors" });
    await waitFor(() => expect(actorSearches).toHaveLength(1));
    expect(Object.fromEntries(actorSearches[0])).toEqual({
      mode: "discovery",
      query: "orion",
      limit: "10",
    });
    expect(screen.getByRole("heading", { name: "Articles" })).toBeVisible();
    await screen.findByText("Matched title");
    // Same-name actors stay separate and show the id that tells them apart.
    const links = await screen.findAllByRole("link", {
      name: /^Open actor dossier/,
    });
    expect(links.map((link) => link.getAttribute("aria-label"))).toEqual([
      "Open actor dossier: Orion Group (actor:one)",
      "Open actor dossier: orion group (actor:two)",
    ]);
    expect(screen.getByText("Matched alias “Orion”")).toBeVisible();
    await user.click(links[1]);
    await screen.findByText(
      "This identity is missing or is not an active actor.",
      undefined,
      { timeout: 5000 },
    );
    // The dossier read uses the canonical id (it may retry once), never the label or alias.
    expect(new Set(details)).toEqual(new Set(["actor:two"]));
    await user.click(
      screen.getByRole("button", { name: "Return to Bulletin" }),
    );
    await screen.findByRole("heading", { name: "Results for “orion”" });
    expect(
      await screen.findAllByRole("link", { name: /^Open actor dossier/ }),
    ).toHaveLength(2);
    expect(
      screen.getByRole("textbox", {
        name: "Search article titles and descriptions",
      }),
    ).toHaveValue("orion");
  });

  it.each([
    [
      "an unavailable route",
      () => HttpResponse.json({ error: "down" }, { status: 503 }),
      "Actor search is unavailable on this backend.",
    ],
    [
      "an index that is not built",
      () => HttpResponse.json(actorPage([], "not_built")),
      "Actor index is not built. Actor results are not available.",
    ],
  ])(
    "keeps article results when actor search has %s",
    async (_name, respond, notice) => {
      mswServer.use(
        http.get(`${testBase}/api/v1/articles/search`, () =>
          HttpResponse.json({
            hits: [articleHit],
            next_cursor: null,
            revision: "s",
          }),
        ),
        http.get(`${testBase}/api/v1/actors/search`, respond),
      );
      const user = userEvent.setup();
      render(<App />);
      await user.click(
        await screen.findByRole("button", { name: "Open Intel" }),
      );
      await user.type(
        await screen.findByRole("textbox", {
          name: "Search article titles and descriptions",
        }),
        "harbor{Enter}",
      );
      await screen.findByText(notice);
      expect(await screen.findByText("Matched title")).toBeVisible();
      expect(
        screen.queryByRole("heading", { name: "NO MATCHING ACTORS" }),
      ).not.toBeInTheDocument();
    },
  );

  it("narrows a search by day, category and source domain without making them text fields", async () => {
    const params: Record<string, string>[] = [];
    mswServer.use(
      http.get(`${testBase}/api/v1/articles/search`, ({ request }) => {
        params.push(Object.fromEntries(new URL(request.url).searchParams));
        return HttpResponse.json({
          hits: [],
          next_cursor: null,
          revision: "s",
          truncated: false,
        });
      }),
    );
    const user = userEvent.setup();
    render(<App />);
    await user.click(await screen.findByRole("button", { name: "Open Intel" }));
    const search = await screen.findByRole("textbox", {
      name: "Search article titles and descriptions",
    });
    await user.type(search, "harbor{Enter}");
    await screen.findByRole("heading", { name: "NO MATCHING ARTICLES" });
    expect(params.at(-1)).toEqual({ query: "harbor", limit: "20" });
    await user.click(screen.getByRole("button", { name: "Only 2026-10-29" }));
    await waitFor(() => expect(params.at(-1)?.day).toBe("2026-10-29"));
    await user.click(
      screen.getByRole("button", { name: "Only Politics & Governance" }),
    );
    await waitFor(() => expect(params.at(-1)?.category).toBe("governance"));
    const source = screen.getByRole("textbox", { name: "Source domain" });
    await user.type(source, "not a domain{Enter}");
    expect(screen.getByRole("alert")).toHaveTextContent("Use a domain");
    await user.clear(source);
    await user.type(source, "Example.Test{Enter}");
    await waitFor(() => expect(params.at(-1)?.source).toBe("example.test"));
    expect(params.at(-1)).toEqual({
      query: "harbor",
      limit: "20",
      category: "governance",
      day: "2026-10-29",
      source: "example.test",
    });
    await user.click(screen.getByRole("button", { name: "Only 2026-10-29" }));
    await waitFor(() => expect(params.at(-1)?.day).toBeUndefined());
  });

  it("moves category focus with arrows, selects with Enter and opens the whole article card", async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.click(await screen.findByRole("button", { name: "Open Intel" }));
    await screen.findByRole("button", {
      name: `Open Focus Brief: ${row.title}`,
    });
    await user.click(
      screen.getByRole("button", { name: /^Politics & Governance$/ }),
    );
    await user.keyboard("{ArrowDown}");
    expect(
      screen.getByRole("button", { name: /^Security & Conflict$/ }),
    ).toHaveFocus();
    expect(
      screen.getByRole("button", { name: /^Politics & Governance$/ }),
    ).toHaveAttribute("aria-pressed", "true");
    await user.keyboard("{Enter}");
    expect(
      screen.getByRole("button", { name: /^Security & Conflict$/ }),
    ).toHaveAttribute("aria-pressed", "true");
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
