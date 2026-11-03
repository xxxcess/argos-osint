import React, { act } from "react";
import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Button, colors } from "@argos/ui";

// RNW 0.21 applies styles through atomic CSS classes, so visual assertions read
// computed styles on the real elements.
const applied = (el: Element) => getComputedStyle(el);
const buttonEl = (container: HTMLElement, index = 0) =>
  container.querySelectorAll('[role="button"]')[index] as HTMLElement;

describe("Button (real @argos/ui component)", () => {
  it("renders children as an accessible button", () => {
    render(<Button onPress={() => {}}>Click me</Button>);
    expect(screen.getByRole("button", { name: /click me/i })).toBeVisible();
  });

  it("uses accessibilityLabel as the accessible name", () => {
    render(
      <Button onPress={() => {}} label="Custom label">
        Accessible
      </Button>,
    );
    expect(screen.getByRole("button", { name: "Custom label" })).toBeTruthy();
  });

  it("calls onPress when pressed", () => {
    const handlePress = vi.fn();
    render(<Button onPress={handlePress}>Click me</Button>);
    fireEvent.click(screen.getByRole("button", { name: /click me/i }));
    expect(handlePress).toHaveBeenCalledTimes(1);
  });

  it("does not call onPress when disabled and reports disabled state", () => {
    const handlePress = vi.fn();
    render(
      <Button onPress={handlePress} disabled>
        Click me
      </Button>,
    );
    const button = screen.getByRole("button", { name: /click me/i });
    expect(button).toBeDisabled();
    fireEvent.click(button);
    expect(handlePress).not.toHaveBeenCalled();
  });

  it("applies the default (spaced density) press target on the glass surface", () => {
    const { container } = render(<Button onPress={() => {}}>Default</Button>);
    const button = buttonEl(container)!;
    // minHeight 40 keeps the default touch target; default surface is glass.
    expect(applied(button).minHeight).toBe("40px");
    expect(applied(button).backgroundColor).toBe("rgba(30, 30, 30, 0.55)");
    expect(applied(button).borderTopColor).toBe("rgb(83, 83, 83)");
  });

  it("applies the primary variant with accent tokens", () => {
    const { container } = render(
      <Button onPress={() => {}} variant="primary">
        Primary
      </Button>,
    );
    const button = buttonEl(container)!;
    expect(applied(button).backgroundColor).toBe(
      "rgba(135, 191, 255, 0.14)", // accentSoft
    );
    expect(applied(button).borderTopColor).toBe("rgb(135, 191, 255)"); // accent
    expect(applied(screen.getByText("Primary")).color).toBe("rgb(135, 191, 255)");
  });

  it("applies the quiet variant with a transparent surface", () => {
    const { container } = render(
      <Button onPress={() => {}} variant="quiet">
        Quiet
      </Button>,
    );
    const button = buttonEl(container)!;
    // jsdom serializes `transparent` as rgba(0, 0, 0, 0).
    expect(applied(button).backgroundColor).toBe("rgba(0, 0, 0, 0)");
    expect(applied(button).borderTopColor).toBe("rgb(83, 83, 83)"); // border, not accent
    expect(applied(screen.getByText("Quiet")).color).toBe("rgb(181, 181, 181)"); // dim
  });

  it("applies the active (selection) state: accent surface, bg text token, glow", () => {
    const { container } = render(
      <Button onPress={() => {}} active>
        Active
      </Button>,
    );
    const button = buttonEl(container)!;
    expect(applied(button).backgroundColor).toBe("rgb(135, 191, 255)"); // accent
    expect(applied(button).boxShadow).toContain("12px"); // glow
    // Selection uses the bg token against the accent surface for contrast.
    expect(applied(screen.getByText("Active")).color).toBe("rgb(20, 20, 20)");
  });

  it("renders mono uppercase telemetry text styling", () => {
    render(<Button onPress={() => {}}>Telemetry</Button>);
    const text = screen.getByText("Telemetry");
    expect(applied(text).textTransform).toBe("uppercase");
    expect(applied(text).letterSpacing.replace(/^0\./, "0.")).toContain("0.8");
  });

  it("reflects the active boolean through aria-pressed", () => {
    const { rerender } = render(
      <Button onPress={() => {}} active>
        A
      </Button>,
    );
    expect(screen.getByRole("button", { name: "A" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    rerender(
      <Button onPress={() => {}} active={false}>
        A
      </Button>,
    );
    expect(screen.getByRole("button", { name: "A" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  });
});

describe("Button focus/selection lanes (hover + pressed)", () => {
  it("shows accent hover styling for the default variant and restores glass on leave", async () => {
    const user = userEvent.setup();
    const { container } = render(<Button onPress={() => {}}>Hover me</Button>);
    const button = buttonEl(container)!;
    await user.hover(button);
    expect(applied(button).backgroundColor).toBe(
      "rgba(135, 191, 255, 0.14)", // accentSoft
    );
    expect(applied(button).borderTopColor).toBe("rgb(135, 191, 255)"); // accent
    await user.unhover(button);
    expect(applied(button).backgroundColor).toBe("rgba(30, 30, 30, 0.55)");
  });

  it("adds the accent glow on primary hover", async () => {
    const user = userEvent.setup();
    const { container } = render(
      <Button onPress={() => {}} variant="primary">
        Primary hover
      </Button>,
    );
    const button = buttonEl(container)!;
    await user.hover(button);
    expect(applied(button).boxShadow).toContain("12px");
    await user.unhover(button);
    expect(applied(button).boxShadow).toBe("");
  });

  it("swaps the surface to glassStrong while pressed via keyboard", async () => {
    const user = userEvent.setup();
    const { container } = render(<Button onPress={() => {}}>Press</Button>);
    const button = buttonEl(container)!;
    await user.hover(button);
    await act(async () => {
      button.focus();
      button.dispatchEvent(
        new window.KeyboardEvent("keydown", {
          bubbles: true,
          key: " ",
          code: "Space",
        }),
      );
    });
    // React/pressability flushes after the event loop turns, so wait for it.
    await waitFor(() =>
      expect(applied(button).backgroundColor).toBe("rgba(48, 48, 48, 0.72)"), // glassStrong
    );
    await act(async () => {
      button.dispatchEvent(
        new window.KeyboardEvent("keyup", {
          bubbles: true,
          key: " ",
          code: "Space",
        }),
      );
    });
    // Releasing press returns to the active hover lane (still hovered).
    await waitFor(() =>
      expect(applied(button).backgroundColor).toBe(
        "rgba(135, 191, 255, 0.14)",
      ), // accentSoft
    );
    await user.unhover(button);
    await waitFor(() =>
      expect(applied(button).backgroundColor).toBe("rgba(30, 30, 30, 0.55)"),
    );
  });

  it("hides no content during hover or press", async () => {
    const user = userEvent.setup();
    const { container } = render(
      <div>
        <Button onPress={() => {}}>Focus target</Button>
        <p>Neighbouring telemetry stays visible.</p>
      </div>,
    );
    const button = buttonEl(container)!;
    await user.hover(button);
    await act(async () => {
      button.focus();
      button.dispatchEvent(
        new window.KeyboardEvent("keydown", {
          bubbles: true,
          key: " ",
          code: "Space",
        }),
      );
    });
    // Focus/press treatments never remove sibling content from the tree.
    expect(
      screen.getByText("Neighbouring telemetry stays visible."),
    ).toBeVisible();
  });
});

// Token sanity: the button palette stays on the terminal-desk neutrals.
describe("Button token bindings", () => {
  it("binds the glass, glassStrong, accentSoft and border tokens", () => {
    expect(colors.accent).toBe("#87bfff");
    expect(colors.glass).toBe("rgba(30, 30, 30, 0.55)");
    expect(colors.glassStrong).toBe("rgba(48, 48, 48, 0.72)");
    expect(colors.accentSoft).toBe("rgba(135, 191, 255, 0.14)");
    expect(colors.border).toBe("#535353");
  });
});
