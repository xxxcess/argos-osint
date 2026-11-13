import React, { act } from "react";
import { describe, expect, it } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Button, StatusTag, colors } from "@argos/ui";
import { paint, normColor } from "../../../vitest.test-support";

const applied = (el: Element) => getComputedStyle(el);
const buttonEl = (container: HTMLElement, index = 0) =>
  container.querySelectorAll('[role="button"]')[index] as HTMLElement;

/**
 * Matte state contract: hover and selection have distinct solid fills/borders.
 * Selection remains semantic (aria-pressed); glow is not required for state.
 * Focus or press must never hide other content.
 */
describe("Focus vs selection states (real components)", () => {
  it("hover lane paints accent border on accentSoft ground without glow", async () => {
    const user = userEvent.setup();
    const { container } = render(
      <Button onPress={() => {}}>Lane probe</Button>,
    );
    const button = buttonEl(container)!;
    await user.hover(button);
    expect(paint(button, "borderTopColor")).toBe(colors.accent.toLowerCase());
    expect(applied(button).backgroundColor).toBe(colors.accentSoft);
    expect(applied(button).boxShadow).toBe(""); // no glow in the hover lane
  });

  it("the primary variant changes its solid fill without glow on hover", async () => {
    const user = userEvent.setup();
    const { container } = render(
      <Button onPress={() => {}} variant="primary">
        Glow lane
      </Button>,
    );
    const button = buttonEl(container)!;
    await user.hover(button);
    expect(paint(button, "backgroundColor")).toBe(
      colors.accentHover.toLowerCase(),
    );
    expect(applied(button).boxShadow).toBe("");
    await user.unhover(button);
    expect(applied(button).boxShadow).toBe("");
  });

  it("selection (`active`) is distinct from hover: flat accent surface without glow", () => {
    render(
      <div>
        <Button onPress={() => {}} active>
          Selected
        </Button>
        <Button onPress={() => {}}>Plain</Button>
      </div>,
    );
    const selected = document.querySelector(
      '[aria-pressed="true"]',
    ) as HTMLElement;
    const plain = buttonEl(document.body, 1);
    expect(applied(selected).backgroundColor).toBe("rgb(198, 165, 110)"); // accent fill
    expect(applied(selected).boxShadow).toBe("");
    expect(paint(plain, "backgroundColor")).toBe(colors.raised.toLowerCase());
  });

  it("aria-pressed marks only the selected control; hover alone does not", async () => {
    const user = userEvent.setup();
    const { container } = render(
      <div>
        <Button onPress={() => {}} active>
          Active row
        </Button>
        <Button onPress={() => {}}>Idle row</Button>
      </div>,
    );
    await user.hover(buttonEl(container, 1)!);
    const rows = [...container.querySelectorAll('[role="button"]')];
    expect(rows[0]).toHaveAttribute("aria-pressed", "true");
    // RNW renders the attribute only for stateful (true) presses.
    expect(rows[1].getAttribute("aria-pressed")).toBeNull();
  });

  it("keyboard press uses the pressed lane (glassStrong), distinct from hover", async () => {
    const user = userEvent.setup();
    const { container } = render(
      <Button onPress={() => {}}>Press lane</Button>,
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
    await waitFor(() =>
      expect(normColor(applied(button).backgroundColor)).toBe(
        colors.glassStrong.toLowerCase(),
      ),
    );
  });

  it("focus never hides other panels or content", async () => {
    const user = userEvent.setup();
    render(
      <div>
        <Button onPress={() => {}} variant="primary" label="Focus target">
          x
        </Button>
        <p>Neighboring evidence panel content.</p>
      </div>,
    );
    const target = document.querySelector('[role="button"]') as HTMLElement;
    await act(async () => {
      target.focus();
    });
    await user.hover(target);
    expect(target).toBeVisible();
    expect(
      screen.getByText("Neighboring evidence panel content."),
    ).toBeVisible();
  });

  it("StatusTag tone stays stable while a sibling button is hovered (tones ≠ focus)", async () => {
    const user = userEvent.setup();
    const { container } = render(
      <div>
        <Button onPress={() => {}}>Hover source</Button>
        <StatusTag tone="accent">ALERT</StatusTag>
      </div>,
    );
    await user.hover(buttonEl(container)!);
    expect(paint(screen.getByText("[ALERT]"), "color")).toBe(
      colors.accent.toLowerCase(),
    );
  });
});
