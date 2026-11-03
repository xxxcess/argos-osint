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

  it("gives every article image the same square and scales it to fit", () => {
    render(
      <ArticleCard
        article={{
          id: "article-with-image",
          run_id: "run-1",
          title: "Retained harbor report",
          description: "Stored source description.",
          category: "geopolitical",
          source_name: "Fixture Wire",
          source_domain: "example.test",
          published_at: "2026-10-25T12:00:00Z",
          image_url: "https://images.example.test/story.jpg",
          url: "https://example.test/report",
          author: "Fixture reporter",
        }}
        onPress={() => {}}
      />,
    );
    const image = screen.getByRole("img", {
      name: "Article image: Retained harbor report",
    });
    const frame = image.parentElement;
    const painted = frame?.querySelector("div");
    const rules = [...document.styleSheets].flatMap((sheet) => {
      try {
        return [...sheet.cssRules].map((rule) => rule.cssText);
      } catch {
        return [];
      }
    });
    const sizeClass = painted?.className
      .split(" ")
      .find((name) => name.startsWith("r-backgroundSize-"));
    const widthClass = frame?.className
      .split(" ")
      .find((name) => name.startsWith("r-width-"));
    const heightClass = frame?.className
      .split(" ")
      .find((name) => name.startsWith("r-height-"));
    const rule = (className: string | undefined) =>
      rules.find((entry) => className && entry.includes(`.${className}`));
    expect(rule(sizeClass)).toContain("background-size: contain");
    expect(rule(widthClass)).toContain("width: 120px");
    expect(rule(heightClass)).toContain("height: 120px");
  });
});
