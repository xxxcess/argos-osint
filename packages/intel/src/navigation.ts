export type ArticleLocation = { run_id: string; article_id: string };
export type ActorLocation = {
  id: string | null;
  source?: ArticleLocation & { actor_id: string };
  from?: ArticleLocation;
};
/** @tag:web-routing — Shared screen routes; each host supplies its navigation/history implementation. */
export type IntelRoute =
  | { screen: "home" }
  | { screen: "bulletin" }
  | ({ screen: "brief" } & Partial<ArticleLocation>)
  | ({ screen: "actor" } & ActorLocation)
  | {
      screen: "topology";
      actorFrom?: ActorLocation;
      seed: string | null;
      /** Node to select inside an extracted brief graph. */
      focus?: string;
      from?: ArticleLocation;
    };
export interface IntelNavigation {
  subscribe: (listener: () => void) => () => void;
  getSnapshot: () => IntelRoute;
  getServerSnapshot: () => IntelRoute;
  navigate: (route: IntelRoute) => void;
}

/** Native hosts without an external router retain the same screen contract. */
export function createMemoryNavigation(): IntelNavigation {
  let route: IntelRoute = { screen: "home" };
  const initial = route;
  const listeners = new Set<() => void>();
  return {
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    getSnapshot: () => route,
    getServerSnapshot: () => initial,
    navigate(next) {
      route = next;
      listeners.forEach((listener) => listener());
    },
  };
}
