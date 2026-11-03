import React from "react";
import { expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { BackendProvider } from "@argos/api-client";
import { IntelApp } from "../src";
import { http, HttpResponse } from "msw";
import { mswServer, testBase } from "../../../vitest.setup";
vi.mock("../src/landing/OrbitScene", () => ({
  OrbitScene: ({
    ready,
    onComplete,
  }: {
    ready: boolean;
    onComplete: () => void;
  }) => (
    <button onClick={onComplete}>
      {ready ? "Green backend, orbit in progress" : "Connecting orbit"}
    </button>
  ),
}));
it("holds the ready workspace until the landing sequence completes", async () => {
  mswServer.use(
    http.get(`${testBase}/api/v1/health/ready`, () =>
      HttpResponse.json({
        ready: true,
        storage: true,
        service: "argos-intel",
        api_version: 1,
        instance_id: "fixture",
      }),
    ),
  );
  const settings = {
    subscribe: () => () => {},
    getSnapshot: () => testBase,
    getServerSnapshot: () => testBase,
    setBase: () => {},
    createRequestId: () => "fixture",
  };
  render(
    <BackendProvider settings={settings}>
      <IntelApp />
    </BackendProvider>,
  );
  const finish = await screen.findByRole("button", {
    name: "Green backend, orbit in progress",
  });
  expect(screen.queryByRole("button", { name: "Open Intel" })).toBeNull();
  await userEvent.click(finish);
  expect(
    await screen.findByRole("button", { name: "Open Intel" }),
  ).toBeVisible();
});
