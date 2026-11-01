import React from "react";
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ArticleCard } from "../src/views/ArticleCard";

// @tag:testing — real card metadata; no synthetic REF, trend or TLP fields.
describe("Bulletin article metadata", () => {
  it("keeps category, date and source visible and opens the whole card without a redundant details link", async () => {
    const onPress = vi.fn();
    const user = userEvent.setup();
    render(
      <ArticleCard
        article={{
          id: "article-raw-id",
          run_id: "run-1",
          title: "Retained harbor report",
          description: "Stored source description.",
          category: "geopolitical",
          source_name: "Fixture Wire",
          source_domain: "example.test",
          published_at: "2026-10-25T12:00:00Z",
          image_url: "",
          url: "https://example.test/report",
          author: "Fixture reporter",
        }}
        onPress={onPress}
      />,
    );
    expect(screen.queryByText("article-raw-id")).not.toBeInTheDocument();
    expect(screen.getByText("GEOPOLITICAL")).toBeVisible();
    expect(screen.getByText("2026-10-25")).toBeVisible();
    expect(screen.queryByText(/VIEW_DETAILS/)).not.toBeInTheDocument();
    expect(screen.getByText("Fixture Wire · Fixture reporter")).toBeVisible();
    expect(
      screen.queryByText(/REF-000|COOLING DOWN|HEATING UP|TLP:/),
    ).not.toBeInTheDocument();
    await user.click(
      screen.getByRole("button", {
        name: "Open Focus Brief: Retained harbor report",
      }),
    );
    expect(onPress).toHaveBeenCalledOnce();
  });
});
