import React from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { mswServer, testBase } from "../../../vitest.setup";
import { BackendProvider } from "@argos/api-client";
import { IntelApp } from "@argos/intel";
import { ErrorBoundary } from "@argos/ui";

// In-memory persistence adapter (same pattern as useIntelQuery tests)
let currentBase = testBase;
const subscribers = new Set<() => void>();
const settings = {
  subscribe: (onChange: () => void) => {
    subscribers.add(onChange);
    return () => subscribers.delete(onChange);
  },
  getSnapshot: () => currentBase,
  getServerSnapshot: () => testBase,
  setBase: (base: string) => {
    currentBase = base;
    subscribers.forEach((fn) => fn());
  },
  createRequestId: () => "test-request-id",
};

const readyHealth = {
  ready: true,
  storage: true,
  service: "argos-intel",
  api_version: 1,
  instance_id: "inst-test",
};

function providerWrapper() {
  return function Wrapper({ children }: { children?: React.ReactNode }) {
    return <BackendProvider settings={settings}>{children}</BackendProvider>;
  };
}

beforeEach(() => {
  currentBase = testBase;
});

describe("Web Desktop App (real IntelApp)", () => {
  beforeEach(() => {
    // Mock health endpoint to return offline initially
    mswServer.use(
      http.get(`${testBase}/api/v1/health/ready`, () => {
        return HttpResponse.json({
          ...readyHealth,
          ready: false,
          storage: false,
        });
      }),
    );
  });

  it("renders the offline shell without exposing a working app tab", () => {
    render(<IntelApp />, { wrapper: providerWrapper() });
    expect(screen.queryByLabelText("API offline")).toBeNull();
    expect(screen.getByTestId("landing-core")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "server" })).toBeDisabled();
    expect(
      screen.queryByRole("button", { name: "Open Intel" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /^Bulletin$/ }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("heading", { name: "Intelligence bulletin" }),
    ).not.toBeInTheDocument();
  });

  it("shows connection status and retry when API is offline", async () => {
    render(<IntelApp />, { wrapper: providerWrapper() });
    // A settled not-ready response must leave a usable retry, not a pending form.
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Retry connection checks" }),
      ).toBeEnabled(),
    );
    expect(screen.queryByLabelText("API offline")).toBeNull();
    expect(screen.getByTestId("landing-core")).toBeInTheDocument();
    expect(
      screen.getByRole("status", { name: "Waiting for workspace readiness" }),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText(/api endpoint/i)).not.toBeInTheDocument();
    expect(
      screen.queryByRole("status", { name: "Checking local API…" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("list", { name: "Connection checks" }),
    ).toBeInTheDocument();
  });

  it("shows connecting state when health check is in progress", async () => {
    const user = userEvent.setup();
    let resolveHealth!: (value: Response) => void;
    const healthPromise = new Promise<Response>((resolve) => {
      resolveHealth = resolve;
    });

    mswServer.use(
      http.get(`${testBase}/api/v1/health/ready`, () => healthPromise),
    );

    render(<IntelApp />, { wrapper: providerWrapper() });
    await waitFor(() =>
      expect(
        screen.getByRole("status", { name: "Checking local API…" }),
      ).toBeVisible(),
    );
    expect(
      screen.queryByRole("button", { name: "Retry connection checks" }),
    ).toBeNull();
    expect(screen.queryByLabelText("API offline")).toBeNull();
    expect(screen.getByTestId("landing-core")).toBeInTheDocument();
    resolveHealth(HttpResponse.json(readyHealth));
    await waitFor(() =>
      expect(screen.getByLabelText("API connected")).toHaveTextContent(
        "API_OK",
      ),
    );
    expect(screen.getByRole("button", { name: "server" })).toBeEnabled();
    expect(
      screen.queryByRole("button", { name: /^Bulletin$/ }),
    ).not.toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Open Intel" })).toBeVisible(),
    );
    expect(
      screen.queryByRole("heading", { name: "Intelligence bulletin" }),
    ).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Open Intel" }));
    expect(screen.getByRole("button", { name: /^Bulletin$/ })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(
      screen.queryByRole("status", { name: "Checking local API…" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByLabelText(/api endpoint/i)).not.toBeInTheDocument();
  });

  it("keeps backend configuration server-side and offers readiness retry", async () => {
    render(<IntelApp />, { wrapper: providerWrapper() });
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Retry connection checks" }),
      ).toBeEnabled(),
    );
    expect(screen.queryByRole("textbox", { name: /API endpoint/i })).toBeNull();
    expect(
      screen.getByRole("list", { name: "Connection checks" }),
    ).toBeVisible();
  });

  it("retries readiness and retains Bulletin filters while navigating through Home", async () => {
    const user = userEvent.setup();
    render(<IntelApp />, { wrapper: providerWrapper() });
    const retry = await screen.findByRole("button", {
      name: "Retry connection checks",
    });
    mswServer.use(
      http.get(`${testBase}/api/v1/health/ready`, () =>
        HttpResponse.json(readyHealth),
      ),
    );
    await user.click(retry);
    await screen.findByLabelText("API connected");
    await user.click(screen.getByRole("button", { name: "Open Intel" }));
    await user.type(
      screen.getByRole("textbox", { name: "Search articles" }),
      "shipping",
    );
    await user.click(screen.getByRole("button", { name: /^Economic$/ }));
    await user.click(screen.getByRole("button", { name: /^server$/ }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Open Intel" })).toBeVisible(),
    );
    expect(
      screen.queryByRole("button", { name: /^Bulletin$/ }),
    ).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Open Intel" }));
    expect(
      screen.getByRole("heading", { name: "Intelligence bulletin" }),
    ).toBeVisible();
    expect(
      screen.getByRole("textbox", { name: "Search articles" }),
    ).toHaveValue("shipping");
    expect(screen.getByRole("button", { name: /^Economic$/ })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByLabelText("API connected")).toHaveTextContent("API_OK");
  });
});

describe("Web Desktop App - Error Boundary", () => {
  it("catches render errors and displays fallback", () => {
    // Create a component that throws during render to test the error boundary
    function ThrowingComponent(): never {
      throw new Error("Test error boundary");
    }

    const { container } = render(
      <ErrorBoundary label="Test Error Boundary" onRetry={() => {}}>
        <ThrowingComponent />
      </ErrorBoundary>,
    );

    // ErrorBoundary should catch the error and display fallback
    expect(container.textContent).toContain("Test Error Boundary");
    expect(container.textContent).toContain("Test error boundary");
    expect(container.textContent).toContain("Retry");
  });
});
