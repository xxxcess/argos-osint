import type React from "react";

/** Web-only keyboard adapter; native Pressables retain their own activation. */
export function moveCategoryFocus(event: React.KeyboardEvent<HTMLElement>) {
  const keys = [
    "ArrowDown",
    "ArrowUp",
    "ArrowLeft",
    "ArrowRight",
    "Home",
    "End",
  ];
  if (!keys.includes(event.key)) return;
  const buttons = Array.from(
    event.currentTarget.querySelectorAll<HTMLElement>('[role="button"]'),
  );
  const index = buttons.indexOf(event.target as HTMLElement);
  if (index < 0) return;
  event.preventDefault();
  const next =
    event.key === "Home"
      ? 0
      : event.key === "End"
        ? buttons.length - 1
        : (index +
            (["ArrowUp", "ArrowLeft"].includes(event.key) ? -1 : 1) +
            buttons.length) %
          buttons.length;
  buttons[next]?.focus();
}
