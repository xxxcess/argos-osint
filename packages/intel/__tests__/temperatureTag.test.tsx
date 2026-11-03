import React from "react";
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { ArticleCard } from "../src/views/bulletin/ArticleCard";
import { BriefHeader } from "../src/views/brief/BriefHeader";
import { temperatureLabel } from "../src/views/temperatureTag";

const article = {
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
};

describe("article temperature tags", () => {
  it("splits 0 cold through 1 hot into four equal tags", () => {
    expect(temperatureLabel(0)).toBe("Cold");
    expect(temperatureLabel(0.24)).toBe("Cold");
    expect(temperatureLabel(0.25)).toBe("Cool");
    expect(temperatureLabel(0.49)).toBe("Cool");
    expect(temperatureLabel(0.5)).toBe("Warm");
    expect(temperatureLabel(0.74)).toBe("Warm");
    expect(temperatureLabel(0.75)).toBe("Hot");
    expect(temperatureLabel(1)).toBe("Hot");
    expect(temperatureLabel(undefined)).toBeNull();
  });

  it("shows the bulletin tag from the article temperature", () => {
    render(
      <ArticleCard article={{ ...article, temperature: 1 }} onPress={() => {}} />,
    );
    expect(screen.getByText("Hot")).toBeVisible();
    expect(screen.getByLabelText("Temperature Hot")).toBeVisible();
  });

  it("shows the same tag on the brief header", () => {
    render(
      <BriefHeader
        article={{ ...article, temperature: 0.4 }}
        jobState="idle"
        onBack={() => {}}
      />,
    );
    expect(screen.getByText("Cool")).toBeVisible();
  });
});
