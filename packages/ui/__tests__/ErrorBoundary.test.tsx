import React from "react";
import { afterEach, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ErrorBoundary } from "@argos/ui";

afterEach(() => vi.restoreAllMocks());

it("recovers a failed child on Retry without disturbing neighboring content", async () => {
  // React reports the deliberate render failure to the host console.
  const reported = vi.spyOn(console, "error").mockImplementation(() => {});
  let failed = true;
  const retry = vi.fn(() => {
    failed = false;
  });
  function Child() {
    if (failed) throw new Error("Evidence unavailable");
    return <p>Recovered evidence</p>;
  }
  render(
    <>
      <p>Navigation retained</p>
      <ErrorBoundary label="Evidence" onRetry={retry}>
        <Child />
      </ErrorBoundary>
    </>,
  );
  expect(screen.getByText("Evidence unavailable")).toBeVisible();
  expect(screen.getByText("Navigation retained")).toBeVisible();
  await userEvent.setup().click(screen.getByRole("button", { name: "Retry" }));
  expect(retry).toHaveBeenCalledTimes(1);
  expect(screen.getByText("Recovered evidence")).toBeVisible();
  expect(screen.queryByText("Evidence unavailable")).toBeNull();
  expect(reported).toHaveBeenCalled();
});

it("does not offer a nonfunctional Retry when no recovery action exists", () => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  function Child(): never {
    throw new Error("Canvas unavailable");
  }
  render(
    <ErrorBoundary>
      <Child />
    </ErrorBoundary>,
  );
  expect(screen.getByText("Canvas unavailable")).toBeVisible();
  expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
});
