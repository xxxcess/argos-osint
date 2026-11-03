import React from "react";
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { HomeView } from "../src/views/HomeView";

describe("Home Intel launcher", () => {
  it("exposes exactly one launcher and supports Enter and Space activation", async () => {
    const onOpen = vi.fn();
    const user = userEvent.setup();
    render(<HomeView onOpen={onOpen} />);
    expect(screen.getAllByRole("button")).toHaveLength(1);
    const launcher = screen.getByRole("button", { name: "Open Intel" });
    await user.tab();
    expect(launcher).toHaveFocus();
    await user.keyboard("{Enter}");
    await user.keyboard(" ");
    expect(onOpen).toHaveBeenCalledTimes(2);
    expect(screen.getByText("INTEL")).toBeVisible();
  });
});
