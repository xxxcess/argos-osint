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

  it("applies the default (spaced density) press target on the matte graphite surface", () => {
    const { container } = render(<Button onPress={() => {}}>Default</Button>);
    const button = buttonEl(container)!;
    // minHeight 40 keeps the default touch target; default surface is matte graphite.
    expect(applied(button).minHeight).toBe("40px");
    expect(applied(button).backgroundColor).toBe("rgb(18, 24, 32)");
    expect(applied(button).borderTopColor).toBe("rgb(100, 117, 135)");
  });

  it("applies the primary variant with accent tokens", () => {
    const { container } = render(
      <Button onPress={() => {}} variant="primary">
        Primary
      </Button>,
    );
    const button = buttonEl(container)!;
    expect(applied(button).backgroundColor).toBe(
      "rgb(36, 220, 227)", // flat accent body
    );
    expect(applied(button).borderTopColor).toBe("rgb(36, 220, 227)");
    expect(applied(screen.getByText("Primary")).color).toBe("rgb(6, 7, 9)");
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
    expect(applied(button).borderTopColor).toBe("rgb(100, 117, 135)"); // border, not accent
    expect(applied(screen.getByText("Quiet")).color).toBe("rgb(168, 182, 197)"); // dim
  });

  it("applies the active (selection) state: accent surface and contrasting text without glow", () => {
    const { container } = render(
      <Button onPress={() => {}} active>
        Active
      </Button>,
    );
    const button = buttonEl(container)!;
    expect(applied(button).backgroundColor).toBe("rgb(36, 220, 227)"); // accent
    expect(applied(button).boxShadow).toBe("");
    // Selection uses the bg token against the accent surface for contrast.
    expect(applied(screen.getByText("Active")).color).toBe("rgb(6, 7, 9)");
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
      "rgba(36, 220, 227, 0.1)", // accentSoft
    );
    expect(applied(button).borderTopColor).toBe("rgb(36, 220, 227)"); // accent
    await user.unhover(button);
    expect(applied(button).backgroundColor).toBe("rgb(18, 24, 32)");
  });

  it("changes the flat primary fill on hover without a glow", async () => {
    const user = userEvent.setup();
    const { container } = render(
      <Button onPress={() => {}} variant="primary">
        Primary hover
      </Button>,
    );
    const button = buttonEl(container)!;
    await user.hover(button);
    expect(applied(button).backgroundColor).toBe("rgb(104, 231, 235)");
    expect(applied(button).boxShadow).toBe("");
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
    await waitFor(
      () => expect(applied(button).backgroundColor).toBe("rgb(18, 24, 32)"), // glassStrong
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
    await waitFor(
      () =>
        expect(applied(button).backgroundColor).toBe("rgba(36, 220, 227, 0.1)"), // accentSoft
    );
    await user.unhover(button);
    await waitFor(() =>
      expect(applied(button).backgroundColor).toBe("rgb(18, 24, 32)"),
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
    expect(colors.accent).toBe("#24DCE3");
    expect(colors.glass).toBe("#0B0F14");
    expect(colors.glassStrong).toBe("#121820");
    expect(colors.accentSoft).toBe("rgba(36, 220, 227, 0.1)");
    expect(colors.border).toBe("#647587");
  });

  it("gives a submit action the theme base, cyan ink and a 44 px target", () => {
    const { container } = render(
      <Button onPress={() => {}} intent="submit">
        Connect
      </Button>,
    );
    const button = buttonEl(container)!;
    expect(applied(button).backgroundColor).toBe("rgb(16, 38, 44)");
    expect(applied(button).minHeight).toBe("44px");
    expect(applied(button).borderTopColor).toBe("rgb(100, 117, 135)");
    expect(applied(screen.getByText("Connect")).color).toBe(
      "rgb(36, 220, 227)",
    );
  });

  it("tints confirm mint and delete rose, and a disabled action returns to the raised base", () => {
    const { container, rerender } = render(
      <Button onPress={() => {}} intent="confirm">
        Confirm
      </Button>,
    );
    expect(applied(buttonEl(container)).backgroundColor).toBe(
      "rgb(16, 40, 31)",
    );
    expect(applied(screen.getByText("Confirm")).color).toBe(
      "rgb(35, 223, 160)",
    );
    rerender(
      <Button onPress={() => {}} intent="delete">
        Delete
      </Button>,
    );
    expect(applied(buttonEl(container)).backgroundColor).toBe(
      "rgb(43, 24, 35)",
    );
    expect(applied(screen.getByText("Delete")).color).toBe(
      "rgb(242, 128, 160)",
    );
    rerender(
      <Button onPress={() => {}} intent="delete" disabled>
        Delete
      </Button>,
    );
    expect(applied(buttonEl(container)).backgroundColor).toBe(
      "rgb(18, 24, 32)",
    );
    expect(applied(screen.getByText("Delete")).color).toBe(
      "rgb(129, 146, 164)",
    );
  });

  it("leaves a button without an intent smooth", () => {
    const { container } = render(<Button onPress={() => {}}>Plain</Button>);
    expect(applied(buttonEl(container)).minHeight).toBe("40px");
  });

  it("draws the rock overlay at the state opacity, and none on a smooth button", () => {
    const layer = (container: HTMLElement) =>
      container.querySelector('[aria-hidden="true"]') as HTMLElement | null;
    const { container, rerender } = render(
      <Button onPress={() => {}} intent="submit">
        Go
      </Button>,
    );
    expect(applied(layer(container)!).opacity).toBe("0.12");
    rerender(
      <Button onPress={() => {}} intent="submit" disabled>
        Go
      </Button>,
    );
    expect(applied(layer(container)!).opacity).toBe("0.04");
    rerender(<Button onPress={() => {}}>Go</Button>);
    expect(layer(container)).toBeNull();
  });
});
