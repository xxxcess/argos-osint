import React from "react";
import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import type { Brief } from "@argos/api-client";
import { ConfidenceSection } from "../src/views/brief/ConfidenceSection";

it("keeps the grade and evidence lines without a rating or assessment status", () => {
  const { rerender } = render(<ConfidenceSection pending />);
  expect(screen.queryByText(/Brief rating/)).not.toBeInTheDocument();
  expect(screen.queryByText("Assessment pending")).not.toBeInTheDocument();
  expect(screen.getByText("Source F · Information 6")).toBeVisible();
  rerender(<ConfidenceSection pending={false} />);
  expect(screen.queryByText("Assessment unavailable")).not.toBeInTheDocument();
  rerender(
    <ConfidenceSection
      pending={false}
      brief={
        {
          confidence_grade: "B2",
          confidence: {
            score: 0,
            provisional: false,
            claims: 2,
            inferences: 1,
          },
        } as Brief
      }
    />,
  );
  expect(screen.queryByText(/Brief rating/)).not.toBeInTheDocument();
  expect(screen.queryByText("Assessed")).not.toBeInTheDocument();
  expect(screen.getByText("Source B · Information 2")).toBeVisible();
  expect(screen.getByText("2 claims · 1 inferences")).toBeVisible();
  expect(screen.getByLabelText("Admiralty grade B2")).toBeVisible();
});

it("shows the confidence reason in the same evidence style as other sections", () => {
  render(
    <ConfidenceSection
      pending={false}
      brief={
        {
          confidence_reason:
            "No scored corroboration or publisher reliability assessment is available.",
        } as Brief
      }
    />,
  );
  expect(screen.queryByText("Assessment unavailable")).not.toBeInTheDocument();
  expect(screen.queryByText(/Brief rating/)).not.toBeInTheDocument();
  const reason = screen.getByText(
    "No scored corroboration or publisher reliability assessment is available.",
  );
  const source = screen.getByText("Source F · Information 6");
  expect(reason.className).toBe(source.className);
});
