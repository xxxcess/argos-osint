import React, { act } from "react";
import { renderToString } from "react-dom/server";
import { hydrateRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { IntelScrollFab, IntelStatusBar } from "../src/shell/IntelShell";
import { fontFamilies } from "../src/shell/intel-fonts";
import { intelMetricColors } from "../src/shell/intel-theme";
import { paint, sheetProp } from "../../../vitest.test-support";

// @tag:testing — only the host viewport is controlled; render real RN components.
const viewport = vi.hoisted(() => ({ width: 1440 }));
vi.mock("react-native", async (importOriginal) => ({
  ...(await importOriginal<typeof import("react-native")>()),
  useWindowDimensions: () => ({
    width: viewport.width,
    height: 900,
    scale: 1,
    fontScale: 1,
  }),
}));
afterEach(() => {
  cleanup();
  viewport.width = 1440;
});

describe("Intel status bar", () => {
  it("reserves a non-shrinking 64px outer bar", () => {
    const { container } = render(<IntelStatusBar connected activeApp />);
    const bar = container.querySelector("#intel-status-bar")!;
    expect(sheetProp(bar, "height")).toBe("64px");
    expect(sheetProp(bar, "flexShrink")).toBe("0");
  });
  it("hydrates the server's narrow status tree at a wide browser viewport without recovery", async () => {
    viewport.width = 1440;
    const container = document.createElement("div");
    document.body.append(container);
    const element = <IntelStatusBar connected={false} activeApp={false} />;
    container.innerHTML = renderToString(element);
    expect(container.textContent).not.toContain("CPU");
    const errors: unknown[] = [];
    let root: Root | undefined;
    try {
      await act(async () => {
        root = hydrateRoot(container, element, {
          onRecoverableError: (error) => errors.push(error),
        });
      });
      expect(container.textContent).toContain("CPU");
      expect(container.textContent).toContain("OFFLINE");
      expect(errors).toEqual([]);
    } finally {
      await act(async () => root?.unmount());
      container.remove();
    }
  });
  it("formats measured metrics, preserving zero and distinguishing unavailable values", () => {
    render(
      <IntelStatusBar
        connected
        activeApp
        metrics={{
          cpuPercent: 0,
          memoryPercent: 45.4,
          diskPercent: null,
          networkKBps: 3376,
        }}
        clock="10:00:54 AM"
      />,
    );
    expect(screen.getByText("0%")).toBeVisible();
    expect(screen.getByText("45%")).toBeVisible();
    expect(screen.getByText("—")).toBeVisible();
    expect(screen.getByRole("group", { name: "NET 3376 KB/s" })).toBeVisible();
    expect(screen.getByText("3376")).toBeVisible();
    expect(screen.getByText("KB/s")).toBeVisible();
    expect(paint(screen.getByText("0%"), "color")).toBe(
      intelMetricColors.cpu.toLowerCase(),
    );
    expect(paint(screen.getByText("45%"), "color")).toBe(
      intelMetricColors.memory.toLowerCase(),
    );
    expect(paint(screen.getByText("3376"), "color")).toBe(
      intelMetricColors.network.toLowerCase(),
    );
    expect(screen.getByLabelText("API connected")).toHaveTextContent("API_OK");
    expect(screen.getByText("10:00:54 AM")).toBeVisible();
  });

  it("does not invent telemetry and keeps the app tab hidden on Home", () => {
    render(<IntelStatusBar connected={false} activeApp={false} />);
    expect(
      screen.queryByRole("button", { name: "Bulletin" }),
    ).not.toBeInTheDocument();
    expect(screen.getByLabelText("API offline")).toHaveTextContent("OFFLINE");
    expect(screen.queryByText(/\d+%/)).not.toBeInTheDocument();
  });
  it("stacks network units for a measured zero but does not imply a measurement when unavailable", () => {
    const { rerender } = render(
      <IntelStatusBar
        connected
        activeApp={false}
        metrics={{ networkKBps: 0 }}
      />,
    );
    expect(screen.getByRole("group", { name: "NET 0 KB/s" })).toBeVisible();
    expect(screen.getByText("KB/s")).toBeVisible();
    rerender(<IntelStatusBar connected activeApp={false} />);
    expect(screen.getByRole("group", { name: "NET —" })).toBeVisible();
    expect(screen.queryByText("KB/s")).not.toBeInTheDocument();
  });

  it.each([768, 390, 320])(
    "keeps navigation and connection status at %ipx without metric clutter",
    async (width) => {
      viewport.width = width;
      const home = vi.fn();
      const intel = vi.fn();
      const user = userEvent.setup();
      render(
        <IntelStatusBar
          connected
          activeApp
          onHome={home}
          onIntel={intel}
          metrics={{ cpuPercent: 51 }}
        />,
      );
      expect(screen.queryByText("51%")).not.toBeInTheDocument();
      expect(screen.getByLabelText("API connected")).toBeVisible();
      const tab = screen.getByRole("button", { name: "Bulletin" });
      expect(tab).toHaveTextContent(width < 600 ? "Intel" : "intelligence");
      expect(tab).toHaveAttribute("aria-pressed", "true");
      await user.click(screen.getByRole("button", { name: "server" }));
      await user.click(screen.getByRole("button", { name: "Bulletin" }));
      expect(home).toHaveBeenCalledOnce();
      expect(intel).toHaveBeenCalledOnce();
    },
  );
});

describe("Intel scroll-to-bottom control", () => {
  it("appears only beyond 200px and activates the supplied scroll action", async () => {
    const onPress = vi.fn();
    const user = userEvent.setup();
    const { rerender } = render(
      <IntelScrollFab remaining={200} onPress={onPress} />,
    );
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
    rerender(<IntelScrollFab remaining={201} onPress={onPress} />);
    await user.click(screen.getByRole("button", { name: "Scroll to bottom" }));
    expect(onPress).toHaveBeenCalledOnce();
    rerender(<IntelScrollFab remaining={0} onPress={onPress} />);
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });
});

describe("Intel font platform boundary", () => {
  it("uses app-loaded CSS font variables only on web", () => {
    expect(fontFamilies("web").mono).toContain("var(--intel-mono");
    expect(fontFamilies("web").body).toContain("var(--intel-body");
  });

  it.each(["ios", "android"] as const)(
    "uses installed native families on %s",
    (platform) => {
      const families = fontFamilies(platform);
      expect(families.mono).toBe(platform === "ios" ? "Menlo" : "monospace");
      expect(families.body).toBe(platform === "ios" ? "System" : "sans-serif");
      expect(Object.values(families).join(" ")).not.toMatch(/var\(|,/);
    },
  );
});
