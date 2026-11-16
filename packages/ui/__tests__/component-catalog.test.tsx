import React from "react";
import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Button } from "@argos/ui";

// A state matrix over the production Button. Each state must read differently from the others
// that a person can reach, and each must stay exposed to assistive tech. Pressed and keyboard
// focus have their own cases in Button.test.tsx.
type Variant = "default" | "quiet" | "primary";
const paint = (el: Element) => {
  const css = getComputedStyle(el);
  return `${css.backgroundColor}|${css.borderTopColor}`;
};

async function states(variant: Variant) {
  const user = userEvent.setup();
  const out: Record<string, string> = {};
  const mount = (props: { active?: boolean; disabled?: boolean }) =>
    render(
      <Button onPress={() => {}} variant={variant} {...props}>
        Catalog
      </Button>,
    );
  const first = mount({});
  const button = first.container.querySelector('[role="button"]')!;
  out.default = paint(button);
  await user.hover(button);
  out.hover = paint(button);
  await user.unhover(button);
  first.unmount();
  const selected = mount({ active: true });
  const selectedButton = selected.container.querySelector('[role="button"]')!;
  out.selected = paint(selectedButton);
  expect(selectedButton).toHaveAttribute("aria-pressed", "true");
  selected.unmount();
  const disabled = mount({ disabled: true });
  const disabledButton = disabled.container.querySelector('[role="button"]')!;
  out.disabled = paint(disabledButton);
  expect(disabledButton).toBeDisabled();
  disabled.unmount();
  return out;
}

describe("Button state catalog", () => {
  it("gives the default variant four distinct looks", async () => {
    const seen = await states("default");
    expect(new Set(Object.values(seen)).size).toBe(4);
  });

  it("gives the primary variant distinct default, hover and disabled looks", async () => {
    const seen = await states("primary");
    expect(new Set([seen.default, seen.hover, seen.disabled]).size).toBe(3);
  });

  it("shows hover on the quiet variant", async () => {
    const seen = await states("quiet");
    expect(seen.hover).not.toBe(seen.default);
  });
});
