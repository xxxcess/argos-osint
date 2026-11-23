import React from "react";
import { expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { BackendProvider } from "@argos/api-client";
import { IntelApp } from "../src";
import { http, HttpResponse } from "msw";
import { mswServer, testBase } from "../../../vitest.setup";
// This scene has no completion event. Home must mount once readiness succeeds.
vi.mock("../src/landing/OrbitScene", () => ({
  OrbitScene: () => <div aria-label="Connecting orbit" />,
}));
it("admits the ready workspace independently of landing animation", async () => {
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
  expect(
    await screen.findByRole("button", { name: "Open Intel" }),
  ).toBeVisible();
});
