import React from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { BackendProvider, useAction, type Article } from "@argos/api-client";
import { Button } from "@argos/ui";
import { IntelApp } from "../src";
import { mswServer, testBase } from "../../../vitest.setup";

// @tag:testing — server-ordered All pages through real IntelApp/Query hooks.
// Equal publication times deliberately have non-lexical ID order: the server
// knows seen_at; presentation must neither infer that tie-breaker nor re-sort.
const a: Article = {
  id: "z",
  run_id: "run-1",
  title: "Server first",
  description: "First retained occurrence",
  category: "geopolitical",
  source_name: "Fixture Wire",
  source_domain: "example.test",
  url: "https://example.test/a",
  published_at: "2026-10-29T10:00:00Z",
  image_url: "",
  author: "Fixture",
};
const b: Article = {
  ...a,
  id: "a",
  title: "Server second",
  category: "military",
};
const c: Article = {
  ...b,
  run_id: "run-2",
  title: "Another run of the same article",
  category: "economic",
  published_at: "2026-10-28T10:00:00Z",
};
const d: Article = {
  ...a,
  id: "older",
  title: "Server older",
  category: "technology",
  published_at: "2026-10-27T10:00:00Z",
};
const cursor1 = JSON.stringify([
  b.published_at,
  "2026-10-29T11:00:00Z",
  b.id,
  b.run_id,
]);
const cursor2 = JSON.stringify([
  c.published_at,
  "2026-10-29T11:00:00Z",
  c.id,
  c.run_id,
]);
type Read = {
  category: string | null;
  day: string | null;
  search: string | null;
  cursor: string | null;
};
const reads: Read[] = [];
const settings = {
  subscribe: () => () => {},
  getSnapshot: () => testBase,
  getServerSnapshot: () => testBase,
  setBase: () => {},
  createRequestId: () => "all-request",
};
function Invalidate() {
  const ensure = useAction<{ job_id: string }>("brief/ensure");
  return (
    <Button onPress={() => ensure.mutate({ run: a.run_id, id: a.id })}>
      Trigger real mutation invalidation
    </Button>
  );
}
function App({
  invalidationControl = false,
}: {
  invalidationControl?: boolean;
}) {
  return (
    <BackendProvider settings={settings}>
      <IntelApp />
      {invalidationControl ? <Invalidate /> : null}
    </BackendProvider>
  );
}
function record(request: { url: string }) {
  const params = new URL(request.url).searchParams;
  const read = {
    category: params.get("category"),
    day: params.get("day"),
    search: params.get("search"),
    cursor: params.get("cursor"),
  };
  reads.push(read);
  return read;
}
function page(
  rows: Article[],
  next_cursor: string | null = null,
  revision = "fixture",
) {
  return HttpResponse.json({ rows, next_cursor, revision });
}
function cards() {
  return screen
    .getAllByRole("button", { name: /^Open Focus Brief:/ })
    .map((element) => element.getAttribute("aria-label"));
}
async function openAll(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByRole("button", { name: "Open Intel" }));
  expect(screen.queryByRole("button", { name: "All" })).toBeNull();
}
async function loadMore(user: ReturnType<typeof userEvent.setup>) {
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Load more articles" }),
    ).toBeEnabled(),
  );
  await user.click(screen.getByRole("button", { name: "Load more articles" }));
}
beforeEach(() => {
  reads.length = 0;
  mswServer.use(
    http.get(`${testBase}/api/v1/health/ready`, () =>
      HttpResponse.json({
        ready: true,
        storage: true,
        service: "argos-intel",
        api_version: 1,
        instance_id: "server-all-fixture",
      }),
    ),
    http.get(`${testBase}/api/v1/days`, () =>
      HttpResponse.json({ days: ["2026-10-29", "2026-10-28"] }),
    ),
    http.get(`${testBase}/api/v1/articles`, ({ request }) => {
      const read = record(request);
      if (read.category !== "geopolitical")
        return page([{ ...a, category: read.category || "geopolitical" }]);
      if (read.cursor === cursor2) return page([d]);
      if (read.cursor === cursor1) return page([b, c], cursor2);
      return page([a, b], cursor1);
    }),
  );
});
afterEach(() => vi.restoreAllMocks());

describe("server-backed global All", () => {
  it("reports an initial All failure as unavailable rather than an empty or category-only feed", async () => {
    let fail = true;
    mswServer.use(
      http.get(`${testBase}/api/v1/articles`, ({ request }) => {
        const read = record(request);
        if (read.category !== "geopolitical") return page([a]);
        return fail
          ? HttpResponse.json(
              { error: "Global feed unavailable" },
              { status: 503 },
            )
          : page([a, b]);
      }),
    );
    const user = userEvent.setup();
    render(<App />);
    await openAll(user);
    const retry = await screen.findByRole(
      "button",
      { name: "Retry bulletin" },
      { timeout: 5000 },
    );
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Global feed unavailable",
    );
    expect(
      screen.queryByRole("heading", { name: "NO MATCHING INTELLIGENCE" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: `Open Focus Brief: ${a.title}` }),
    ).not.toBeInTheDocument();
    expect(screen.queryByText("0 loaded")).not.toBeInTheDocument();
    fail = false;
    await user.click(retry);
    await screen.findByText("2 loaded");
    expect(cards()).toEqual(
      [a, b].map((row) => `Open Focus Brief: ${row.title}`),
    );
    expect(reads.filter((read) => read.category !== "geopolitical")).toHaveLength(0);
  });
  it("uses one server stream, preserves uneven page order, deduplicates occurrences and exhausts opaque cursors", async () => {
    const user = userEvent.setup();
    render(<App />);
    await openAll(user);
    await screen.findByText("2 loaded");
    expect(cards()).toEqual(
      [a, b].map((row) => `Open Focus Brief: ${row.title}`),
    );
    expect(reads.filter((read) => read.category === "geopolitical")).toEqual([
      { category: "geopolitical", day: "2026-10-29", search: "", cursor: null },
    ]);
    expect(reads.filter((read) => read.category !== "geopolitical")).toHaveLength(0);
    await loadMore(user);
    await screen.findByText("3 loaded");
    expect(cards()).toEqual(
      [a, b, c].map((row) => `Open Focus Brief: ${row.title}`),
    );
    await loadMore(user);
    await screen.findByText("4 loaded");
    expect(cards()).toEqual(
      [a, b, c, d].map((row) => `Open Focus Brief: ${row.title}`),
    );
    expect(
      reads
        .filter((read) => read.category === "geopolitical")
        .map((read) => read.cursor),
    ).toEqual([null, cursor1, cursor2]);
    expect(
      screen.queryByRole("button", { name: "Load more articles" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByText(/Loaded counts/)).not.toBeInTheDocument();
    expect(
      screen.queryByText(/bounded previews|not a global chronology/),
    ).not.toBeInTheDocument();
  });

  it("retains global rows after a failed next page and retries the same cursor without losing data", async () => {
    let fail = true;
    mswServer.use(
      http.get(`${testBase}/api/v1/articles`, ({ request }) => {
        const read = record(request);
        if (read.category !== "geopolitical") return page([a]);
        if (!read.cursor) return page([a, b], cursor1);
        return fail
          ? HttpResponse.json(
              { error: "Global next page unavailable" },
              { status: 503 },
            )
          : page([b, c]);
      }),
    );
    const user = userEvent.setup();
    render(<App />);
    await openAll(user);
    await screen.findByText("2 loaded");
    await loadMore(user);
    const retry = await screen.findByRole(
      "button",
      { name: "Retry loading articles" },
      { timeout: 5000 },
    );
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Global next page unavailable",
    );
    expect(cards()).toEqual(
      [a, b].map((row) => `Open Focus Brief: ${row.title}`),
    );
    expect(
      screen.queryByRole("heading", { name: "NO MATCHING INTELLIGENCE" }),
    ).not.toBeInTheDocument();
    const before = reads.length;
    fail = false;
    await user.click(retry);
    await screen.findByText("3 loaded");
    expect(reads.slice(before)).toEqual([
      { category: "geopolitical", day: "2026-10-29", search: "", cursor: cursor1 },
    ]);
    expect(cards()).toEqual(
      [a, b, c].map((row) => `Open Focus Brief: ${row.title}`),
    );
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("refetches loaded All pages through fresh server cursors after real mutation invalidation", async () => {
    let generation = 1;
    const freshCursor = JSON.stringify([
      b.published_at,
      "2026-10-29T12:00:00Z",
      b.id,
      b.run_id,
    ]);
    const refreshed = { ...a, title: "Refreshed server first" };
    mswServer.use(
      http.get(`${testBase}/api/v1/articles`, ({ request }) => {
        const read = record(request);
        if (read.category !== "geopolitical") return page([a]);
        return read.cursor
          ? page(generation === 1 ? [c] : [d])
          : page(
              generation === 1 ? [a, b] : [refreshed, b],
              generation === 1 ? cursor1 : freshCursor,
              `generation-${generation}`,
            );
      }),
      http.post(`${testBase}/api/v1/brief/ensure`, () => {
        generation = 2;
        return HttpResponse.json({ job_id: "existing-job" });
      }),
    );
    const user = userEvent.setup();
    render(<App invalidationControl />);
    await openAll(user);
    await screen.findByText("2 loaded");
    await loadMore(user);
    await screen.findByText("3 loaded");
    const before = reads.length;
    await user.click(
      screen.getByRole("button", {
        name: "Trigger real mutation invalidation",
      }),
    );
    await screen.findByRole("button", { name: `Open Focus Brief: ${d.title}` });
    expect(cards()).toEqual(
      [refreshed, b, d].map((row) => `Open Focus Brief: ${row.title}`),
    );
    expect(
      reads
        .slice(before)
        .filter((read) => read.category === "geopolitical")
        .map((read) => read.cursor),
    ).toEqual([null, freshCursor]);
    expect(
      screen.queryByRole("button", { name: `Open Focus Brief: ${c.title}` }),
    ).not.toBeInTheDocument();
  });

  it("isolates All cursors by search, day and category instead of reusing another scope's pages", async () => {
    const scoped = { ...d, title: "Scoped October 28 result" };
    mswServer.use(
      http.get(`${testBase}/api/v1/articles`, ({ request }) => {
        const read = record(request);
        if (read.search === "ports")
          return page(
            read.day === "2026-10-28"
              ? [
                  {
                    ...scoped,
                    category:
                      read.category === "geopolitical"
                        ? scoped.category
                        : read.category || "geopolitical",
                  },
                ]
              : [],
          );
        if (read.category !== "geopolitical") return page([a]);
        return read.cursor ? page([c]) : page([a, b], cursor1);
      }),
    );
    const user = userEvent.setup();
    render(<App />);
    await openAll(user);
    await screen.findByText("2 loaded");
    await loadMore(user);
    await screen.findByText("3 loaded");
    await user.type(
      screen.getByRole("textbox", { name: "Search articles" }),
      "ports",
    );
    await screen.findByRole("heading", { name: "NO MATCHING INTELLIGENCE" });
    expect(reads.at(-1)).toEqual({
      category: "geopolitical",
      day: "2026-10-29",
      search: "ports",
      cursor: null,
    });
    await user.click(screen.getByRole("button", { name: /Retained day/ }));
    await user.click(screen.getByRole("button", { name: "Day 2026-10-28" }));
    await screen.findByRole("button", {
      name: `Open Focus Brief: ${scoped.title}`,
    });
    expect(reads.at(-1)).toEqual({
      category: "geopolitical",
      day: "2026-10-28",
      search: "ports",
      cursor: null,
    });
    await user.click(screen.getByRole("button", { name: /^Economic$/ }));
    await waitFor(() =>
      expect(reads.at(-1)).toEqual({
        category: "economic",
        day: "2026-10-28",
        search: "ports",
        cursor: null,
      }),
    );
    expect(
      screen.queryByRole("button", { name: "Load more articles" }),
    ).not.toBeInTheDocument();
  });

  it("cancels obsolete All requests on search change and category navigation without showing failure or empty claims", async () => {
    const signals: AbortSignal[] = [];
    const originalFetch = globalThis.fetch;
    vi.spyOn(globalThis, "fetch").mockImplementation((input, init) => {
      const url = new URL(
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.href
            : input.url,
      );
      if (
        url.pathname.endsWith("/articles") &&
        url.searchParams.get("category") === "geopolitical"
      ) {
        const signal = init?.signal;
        if (!signal)
          throw new Error("All requests require the query cancellation signal");
        signals.push(signal);
        return new Promise<Response>((_resolve, reject) => {
          const aborted = () =>
            reject(new DOMException("All request cancelled", "AbortError"));
          if (signal.aborted) aborted();
          else signal.addEventListener("abort", aborted, { once: true });
        });
      }
      return originalFetch(input, init);
    });
    const user = userEvent.setup();
    render(<App />);
    await openAll(user);
    await screen.findByRole("progressbar", { name: "Loading bulletin…" });
    expect(
      screen.queryByRole("heading", { name: "NO MATCHING INTELLIGENCE" }),
    ).not.toBeInTheDocument();
    await waitFor(() => expect(signals).toHaveLength(1));
    await user.type(
      screen.getByRole("textbox", { name: "Search articles" }),
      "ports",
    );
    await waitFor(() => expect(signals).toHaveLength(2));
    expect(signals[0].aborted).toBe(true);
    expect(signals[1].aborted).toBe(false);
    await user.click(screen.getByRole("button", { name: /^Economic$/ }));
    await screen.findByRole("button", { name: `Open Focus Brief: ${a.title}` });
    await waitFor(() =>
      expect(signals.every((signal) => signal.aborted)).toBe(true),
    );
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
});
