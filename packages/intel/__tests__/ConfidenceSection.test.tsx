import React from "react";
import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import type { Brief } from "@argos/api-client";
import { ConfidenceSection } from "../src/views/brief/ConfidenceSection";

it("distinguishes pending, unavailable and an assessed zero score", () => {
  const { rerender } = render(<ConfidenceSection pending />);
  expect(screen.getByText("Assessment pending")).toBeVisible();
  expect(screen.getByText("Brief rating N/A")).toBeVisible();
  rerender(<ConfidenceSection pending={false} />);
  expect(screen.getByText("Assessment unavailable")).toBeVisible();
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
  expect(screen.getByText("Brief rating 0/100")).toBeVisible();
  expect(screen.getByText("Assessed")).toBeVisible();
  expect(screen.getByLabelText("Admiralty grade B2")).toBeVisible();
});

it("labels publisher-only ratings as provisional and explains an unavailable rating", () => {
  const { rerender } = render(
    <ConfidenceSection
      pending={false}
      brief={
        {
          confidence_grade: "B6",
          confidence_basis: "publisher_prior",
          confidence: {
            score: 60,
            provisional: true,
            claims: 0,
            inferences: 0,
          },
        } as Brief
      }
    />,
  );
  expect(screen.getByText("Brief rating 60/100")).toBeVisible();
  expect(screen.getByText("Provisional · publisher prior only")).toBeVisible();
  rerender(
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
  expect(screen.getByText("Assessment unavailable")).toBeVisible();
  expect(
    screen.getByText(
      "No scored corroboration or publisher reliability assessment is available.",
    ),
  ).toBeVisible();
});
