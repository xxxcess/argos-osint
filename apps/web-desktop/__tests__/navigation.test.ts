import { afterEach, expect, it, vi } from "vitest";
import { createMemoryNavigation } from "../../../packages/intel/src/navigation";
import { routeFromUrl, routeUrl, webNavigation } from "../app/navigation";
import type { IntelRoute } from "../../../packages/intel/src/navigation";

afterEach(() => {
  history.replaceState(null, "", "/");
  vi.restoreAllMocks();
});

it("isolates native navigators, keeps the server snapshot stable and unsubscribes", () => {
  const navigation = createMemoryNavigation();
  const other = createMemoryNavigation();
  const changed = vi.fn();
  const stop = navigation.subscribe(changed);
  const route: IntelRoute = { screen: "brief", run_id: "r", article_id: "a" };
  navigation.navigate(route);
  expect(navigation.getSnapshot()).toBe(route);
  expect(navigation.getSnapshot()).toBe(navigation.getSnapshot());
  expect(navigation.getServerSnapshot()).toEqual({ screen: "home" });
  expect(other.getSnapshot()).toEqual({ screen: "home" });
  expect(changed).toHaveBeenCalledTimes(1);
  stop();
  navigation.navigate({ screen: "bulletin" });
  expect(changed).toHaveBeenCalledTimes(1);
});

it.each<IntelRoute>([
  { screen: "home" },
  { screen: "bulletin" },
  { screen: "brief", run_id: "run /&?", article_id: "article /&?" },
  { screen: "topology", seed: "memory /&?" },
  { screen: "topology", seed: "brief:r/a", focus: "claim:local /&?" },
  { screen: "topology", seed: "brief:run/art", focus: "claim:v2:abc" },
])(
  "round-trips $screen routes without corrupting encoded identifiers",
  (route) => {
    expect(routeFromUrl(new URL(routeUrl(route), location.origin))).toEqual(
      route,
    );
  },
);

it.each(['{"Actor":"id"}', '["id"]', "x".repeat(257), ""])(
  "rejects unsupported topology seeds (%s)",
  (seed) => {
    const url = new URL("/topology/", location.origin);
    url.searchParams.set("seed", seed);
    expect(routeFromUrl(url)).toEqual({ screen: "topology", seed: null });
  },
);

it("retains topology return context in history without leaking it into the URL", () => {
  history.replaceState(null, "", "/brief?run=r&id=a&api=http://old.invalid");
  const changed = vi.fn();
  const stop = webNavigation.subscribe(changed);
  try {
    expect(location.search).toBe("?run=r&id=a");
    const from = { run_id: "r", article_id: "a" };
    webNavigation.navigate({ screen: "topology", seed: "memory", from });
    expect(location.pathname + location.search).toBe("/topology?seed=memory");
    expect(webNavigation.getSnapshot()).toEqual({
      screen: "topology",
      seed: "memory",
      from,
    });
    const push = vi.spyOn(history, "pushState");
    webNavigation.navigate({ screen: "topology", seed: "memory", from });
    expect(push).not.toHaveBeenCalled();
    history.replaceState(null, "", "/bulletin");
    window.dispatchEvent(new PopStateEvent("popstate"));
    expect(webNavigation.getSnapshot()).toEqual({ screen: "bulletin" });
    const count = changed.mock.calls.length;
    stop();
    window.dispatchEvent(new PopStateEvent("popstate"));
    expect(changed).toHaveBeenCalledTimes(count);
  } finally {
    stop();
  }
});

it.each(["unknown:row", "claim:" + "x".repeat(257), '{"claim":1}'])(
  "rejects unsupported extracted focus %s",
  (focus) => {
    const url = new URL("/topology?seed=memory", location.origin);
    url.searchParams.set("focus", focus);
    expect(routeFromUrl(url)).toEqual({ screen: "topology", seed: "memory" });
  },
);
