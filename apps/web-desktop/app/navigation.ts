import type { IntelNavigation, IntelRoute } from "@argos/intel/src/navigation";

const initial: IntelRoute = { screen: "home" };
let snapshot: IntelRoute = initial;
let href = "";
const listeners = new Set<() => void>();

export function routeFromUrl(url: URL): IntelRoute {
  const path = url.pathname.replace(/\/$/, "");
  const run_id = url.searchParams.get("run") || undefined;
  const article_id = url.searchParams.get("id") || undefined;
  if (path === "/bulletin") return { screen: "bulletin" };
  if (path === "/brief") return { screen: "brief", run_id, article_id };
  if (path === "/topology") {
    const seed = url.searchParams.get("seed");
    const focus = url.searchParams.get("focus");
    return {
      screen: "topology",
      seed: seed && seed.length <= 256 && !/^[{\[]/.test(seed) ? seed : null,
      ...(focus &&
      focus.length <= 256 &&
      /^(claim|inference|actor|article):/.test(focus)
        ? { focus }
        : {}),
    };
  }
  return initial;
}

export function routeUrl(route: IntelRoute): string {
  const params = new URLSearchParams();
  if (route.screen === "brief" && route.run_id && route.article_id) {
    params.set("run", route.run_id);
    params.set("id", route.article_id);
  }
  if (route.screen === "topology") {
    if (route.seed) params.set("seed", route.seed);
    if (route.focus) params.set("focus", route.focus);
  }
  const query = params.toString();
  return `/${route.screen}${query ? `?${query}` : ""}`;
}

function changed() {
  href = "";
  listeners.forEach((listener) => listener());
}
// Next supports native History API updates; the shared workspace stays mounted.
export const webNavigation: IntelNavigation = {
  subscribe(listener) {
    const current = new URL(location.href);
    if (current.searchParams.has("api")) {
      current.searchParams.delete("api");
      window.history.replaceState(null, "", current);
      href = "";
    }
    listeners.add(listener);
    if (listeners.size === 1) window.addEventListener("popstate", changed);
    return () => {
      listeners.delete(listener);
      if (!listeners.size) window.removeEventListener("popstate", changed);
    };
  },
  getSnapshot() {
    if (href !== location.href) {
      href = location.href;
      snapshot = routeFromUrl(new URL(href));
      const from = history.state?.argosFrom;
      if (
        snapshot.screen === "topology" &&
        from &&
        typeof from.run_id === "string" &&
        typeof from.article_id === "string"
      )
        snapshot = { ...snapshot, from };
    }
    return snapshot;
  },
  getServerSnapshot: () => initial,
  navigate(route) {
    const next = new URL(routeUrl(route), location.origin);
    if (next.href !== location.href)
      window.history.pushState(
        route.screen === "topology" && route.from
          ? { argosFrom: route.from }
          : null,
        "",
        next,
      );
    changed();
  },
};
