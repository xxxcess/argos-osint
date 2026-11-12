import React from "react";
import { it, expect } from "vitest";
import { render, screen, within } from "@testing-library/react";
import type { BriefStage } from "@argos/api-client";
import { BriefProgressStepper } from "../src/views/brief/BriefProgressStepper";
import { intelColors } from "../src/shell/intel-theme";
import { paint } from "../../../vitest.test-support";
const stage = (label: string, state: string, active = false): BriefStage => ({
  operation: label,
  label,
  state,
  active,
  row: state,
  detail: "",
});
it("shows completed, active and queued steps without inventing completion for parallel work", () => {
  render(
    <BriefProgressStepper
      stages={[
        stage("Article", "completed"),
        stage("Claims", "queued"),
        stage("Inferences", "running", true),
        stage("Links", "queued"),
      ]}
      state="running"
    />,
  );
  const list = screen.getByRole("list", { name: "Brief progress steps" });
  expect(
    within(list).getByRole("listitem", { name: "Article: Complete" }),
  ).toBeVisible();
  expect(
    within(list).getByRole("listitem", { name: "Claims: queued" }),
  ).toBeVisible();
  expect(
    within(list).getByRole("listitem", { name: "Inferences: In progress" }),
  ).toBeVisible();
  expect(screen.getByText("Brief in progress")).toBeVisible();
  expect(paint(screen.getByText("Article"), "color")).toBe(
    intelColors.success.toLowerCase(),
  );
  expect(paint(screen.getByText("Inferences"), "color")).toBe(
    intelColors.cool.toLowerCase(),
  );
});
it("retains failure details and indicates interrupted updates independently of completed stages", () => {
  render(
    <BriefProgressStepper
      stages={[
        stage("Article", "completed"),
        { ...stage("Claims", "failed"), detail: "Extraction could not finish" },
      ]}
      state="failed"
      connected={false}
    />,
  );
  expect(
    screen.getByRole("listitem", { name: "Article: Complete" }),
  ).toBeVisible();
  expect(
    screen.getByRole("listitem", { name: "Claims: Failed" }),
  ).toBeVisible();
  expect(
    screen.getByText("Claims · Extraction could not finish"),
  ).toBeVisible();
  expect(
    screen.getByText("Live updates interrupted. Reconnecting…"),
  ).toBeVisible();
});
it("keeps a completed track visible without a reconnect warning", () => {
  render(
    <BriefProgressStepper
      stages={[stage("Article", "completed"), stage("Claims", "completed")]}
      state="succeeded"
      connected={false}
    />,
  );
  expect(screen.getByText("Brief complete")).toBeVisible();
  expect(screen.queryByText(/Reconnecting/)).toBeNull();
});
