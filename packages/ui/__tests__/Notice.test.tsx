import React from "react";
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { Notice } from "@argos/ui";

const applied = (el: Element) => getComputedStyle(el);

describe("Notice (real @argos/ui component)", () => {
  it("renders as a live alert region with the message text", () => {
    render(<Notice>API offline · retry pending</Notice>);
    expect(screen.getByRole("alert")).toBeTruthy();
    expect(screen.getByText("API offline · retry pending")).toBeVisible();
  });

  it("paints the danger rule: 2px red left border on red-soft ground", () => {
    const { container } = render(<Notice>Graph failed</Notice>);
    const box = screen.getByRole("alert") as HTMLElement;
    expect(applied(box).borderLeftWidth).toBe("2px");
    expect(applied(box).borderLeftColor).toBe("rgb(240, 138, 138)"); // danger
    expect(applied(box).backgroundColor).toBe("rgba(240, 138, 138, 0.1)");
  });

  it("keeps the message in danger-colored body text", () => {
    render(<Notice>Something failed</Notice>);
    const text = screen.getByText("Something failed");
    expect(applied(text).color).toBe("rgb(240, 138, 138)"); // danger token
  });

  it("announces error text without hiding sibling content", () => {
    render(
      <div>
        <p data-testid="status">Status bar retained</p>
        <Notice>Could not load more articles.</Notice>
      </div>,
    );
    expect(screen.getByTestId("status")).toBeVisible();
    expect(screen.getByRole("alert")).toBeVisible();
  });
});
