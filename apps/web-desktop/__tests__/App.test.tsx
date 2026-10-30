import React from "react";
import { render, screen, waitFor } from "@testing-library/react";
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

const waitForConnected = (connected: () => boolean) =>
  waitFor(
    () => {
      if (!connected()) throw new Error("not connected yet");
    },
    { timeout: 5000 },
  );

function providerWrapper() {
  return function Wrapper({ children }: { children?: React.ReactNode }) {
    return (
      <BackendProvider settings={settings}>
        {children}
      </BackendProvider>
    );
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
        return HttpResponse.json({ ...readyHealth, ready: false, storage: false });
      })
    );
  });

  it("renders the Intel app with masthead and offline status", () => {
    render(<IntelApp />, { wrapper: providerWrapper() });
    // Find the masthead via the unique "Local intelligence workspace" text
    // Its parent is the brand container which also holds "ARGOS / INTEL"
    const workspaceDesc = screen.getByText("Local intelligence workspace");
    const brandContainer = workspaceDesc.parentElement?.parentElement;
    expect(brandContainer?.textContent).toContain("ARGOS");
    expect(brandContainer?.textContent).toContain("INTEL");
    expect(workspaceDesc).toBeInTheDocument();
    // Shows offline · reconnecting when health is not ready
    expect(screen.getByText(/offline · reconnecting/i)).toBeInTheDocument();
  });

  it("shows connection form when API is offline", async () => {
    render(<IntelApp />, { wrapper: providerWrapper() });
    // Wait for health query to complete and show offline state
    await waitFor(() => expect(screen.getByText(/offline · reconnecting/i)).toBeInTheDocument(), { timeout: 5000 });
    expect(screen.getByText(/connection required/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/api endpoint/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /connect \/ retry/i })).toBeInTheDocument();
    expect(screen.getByText(/loopback only · local development/i)).toBeInTheDocument();
  });

  it("shows connecting state when health check is in progress", async () => {
    let resolveHealth: (value: HttpResponse) => void;
    const healthPromise = new Promise<HttpResponse>((resolve) => {
      resolveHealth = resolve;
    });

    mswServer.use(
      http.get(`${testBase}/api/v1/health/ready`, () => healthPromise),
    );

    render(<IntelApp />, { wrapper: providerWrapper() });
    expect(screen.getByText("Checking local API…")).toBeInTheDocument();
  });

  it("transitions to workspace when API becomes ready", async () => {
    mswServer.use(
      http.get(`${testBase}/api/v1/health/ready`, () => HttpResponse.json(readyHealth)),
      http.get(`${testBase}/api/v1/days`, () => HttpResponse.json({ days: ["2026-10-29"], revision: "rev-1" })),
    );

    render(<IntelApp />, { wrapper: providerWrapper() });
    // Wait for the connection to succeed
    await waitFor(() => expect(screen.getByText("API connected")).toBeInTheDocument(), { timeout: 5000 });
    // Should show the workspace content (Bulletin tab, categories, etc.)
    await waitFor(() => expect(screen.getByText("Intelligence bulletin")).toBeInTheDocument(), { timeout: 5000 });
  });

  it("has accessible API endpoint input with default value", async () => {
    render(<IntelApp />, { wrapper: providerWrapper() });
    await waitFor(() => expect(screen.getByText(/offline · reconnecting/i)).toBeInTheDocument(), { timeout: 5000 });
    const input = screen.getByLabelText(/api endpoint/i) as HTMLInputElement;
    expect(input).toHaveValue("http://127.0.0.1:8080");
  });

  it("renders without crashing", async () => {
    const { unmount } = render(<IntelApp />, { wrapper: providerWrapper() });
    // Wait for initial render to settle
    await waitFor(() => expect(screen.getByText(/offline · reconnecting/i)).toBeInTheDocument(), { timeout: 5000 });
    unmount();
  });
});

describe("Web Desktop App - Error Boundary", () => {
  it("catches render errors in Workspace and displays fallback", async () => {
    // Force an error in the health endpoint to test error boundary
    // Use HttpResponse.error() instead of throwing to avoid corrupting MSW state
    mswServer.use(
      http.get(`${testBase}/api/v1/health/ready`, () => {
        return HttpResponse.error();
      }),
    );

    render(
      <ErrorBoundary label="Test">
        <IntelApp />
      </ErrorBoundary>,
      { wrapper: providerWrapper() }
    );

    // ErrorBoundary should catch the error
    await waitFor(() => expect(screen.getByText(/test error boundary/i)).toBeInTheDocument(), { timeout: 5000 });
  });
});