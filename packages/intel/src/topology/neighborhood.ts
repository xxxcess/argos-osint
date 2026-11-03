/** Ego-network roles for the topology page. Depth 0 is the unfocused graph. */

export type GraphEdge = { id: string; from: string; to: string };

export type Depth = 0 | 1 | 2;

export type NodeRole = "focus" | "first" | "second" | "ghost" | "full";

export function classifyNeighborhood(
  nodeIds: readonly string[],
  edges: readonly GraphEdge[],
  focusId: string | null,
  depth: Depth,
): { role: Map<string, NodeRole>; drawnEdgeIds: Set<string> } {
  const present = new Set(nodeIds);
  const near = new Map<string, Set<string>>();
  const link = (from: string, to: string) => {
    if (!present.has(from) || !present.has(to)) return;
    const next = near.get(from) ?? new Set<string>();
    next.add(to);
    near.set(from, next);
  };
  for (const edge of edges) {
    link(edge.from, edge.to);
    link(edge.to, edge.from);
  }
  const role = new Map<string, NodeRole>();
  const drawn = new Set<string>();
  const draw = (allowed: (edge: GraphEdge) => boolean) => {
    for (const edge of edges) {
      if (present.has(edge.from) && present.has(edge.to) && allowed(edge))
        drawn.add(edge.id);
    }
  };
  if (depth === 0 || !focusId || !present.has(focusId)) {
    for (const id of nodeIds) role.set(id, "full");
    draw(() => true);
    return { role, drawnEdgeIds: drawn };
  }
  const first = near.get(focusId) ?? new Set<string>();
  const second = new Set<string>();
  for (const id of first)
    for (const next of near.get(id) ?? [])
      if (next !== focusId && !first.has(next)) second.add(next);
  role.set(focusId, "focus");
  for (const id of first) role.set(id, "first");
  if (depth === 2) for (const id of second) role.set(id, "second");
  const core = new Set([focusId, ...first]);
  const shown = new Set([...core, ...second]);
  draw((edge) => {
    if (!shown.has(edge.from) || !shown.has(edge.to)) return false;
    if (depth === 1) return core.has(edge.from) && core.has(edge.to);
    return core.has(edge.from) || core.has(edge.to);
  });
  return { role, drawnEdgeIds: drawn };
}

export type ParticleLink = {
  id: string;
  touchesFocus: boolean;
  secondDegree: boolean;
};

/** Focus edges keep their dots; 2nd-degree dots are dropped first at the cap. */
export function assignParticles(
  links: readonly ParticleLink[],
  reduced: boolean,
  cap = 600,
): Map<string, number> {
  const counts = new Map<string, number>();
  if (reduced) {
    for (const link of links) counts.set(link.id, 0);
    return counts;
  }
  const ordered = [...links].sort((a, b) => {
    const rank = (link: ParticleLink) =>
      link.secondDegree ? 2 : link.touchesFocus ? 0 : 1;
    return rank(a) - rank(b);
  });
  let budget = cap;
  for (const link of ordered) {
    const want = link.secondDegree ? 1 : link.touchesFocus ? 3 : 2;
    const give = Math.min(want, Math.max(0, budget));
    counts.set(link.id, give);
    budget -= give;
  }
  return counts;
}

export function particleOffset(id: string): number {
  let hash = 0;
  for (const char of id) hash = (hash * 33 + char.charCodeAt(0)) >>> 0;
  return (hash % 1000) / 1000;
}
