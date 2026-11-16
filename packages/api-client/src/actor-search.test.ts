import { describe, expect, it } from "vitest";
import {
  actorAvailability,
  actorHits,
  actorHitViews,
  actorSearchProblem,
  canonicalActorId,
  type ActorSearchHit,
} from "./actor-search";

const hit = (actor_id: string, label: string, extra = {}): ActorSearchHit => ({
  actor_id,
  label,
  ...extra,
});

describe("actor discovery results", () => {
  it("deduplicates by canonical id, keeps rank order and drops unopenable hits", () => {
    const rows = [
      hit("actor:b", "Beta"),
      hit("actor:a", "Alpha"),
      hit("actor:b", "Beta alias row"),
      hit("", "No id"),
      hit("actor:\u0000", "Control id"),
      hit("x".repeat(257), "Long id"),
      hit("actor:c", "  "),
    ];
    expect(actorHits(rows).map((row) => row.actor_id)).toEqual([
      "actor:b",
      "actor:a",
    ]);
    expect(actorHits(undefined)).toEqual([]);
  });

  it("keeps ids opaque: no trimming or case folding", () => {
    expect(canonicalActorId(" Actor:A ")).toBe(" Actor:A ");
    expect(canonicalActorId("   ")).toBeNull();
    expect(canonicalActorId(7)).toBeNull();
  });

  it("keeps same-name actors distinct and shows their ids only then", () => {
    const views = actorHitViews(
      actorHits([
        hit("actor:1", "Orion Group", { countries: ["NO"], actor_type: "org" }),
        hit("actor:2", "orion group", {
          matched_alias: "Orion",
        }),
        hit("actor:3", "Vega"),
      ]),
    );
    expect(views.map((view) => view.hit.actor_id)).toEqual([
      "actor:1",
      "actor:2",
      "actor:3",
    ]);
    expect(views.map((view) => view.id)).toEqual([
      "actor:1",
      "actor:2",
      undefined,
    ]);
    expect(views[0].detail).toBe("org · NO");
    expect(views[1].match).toBe("Matched alias “Orion”");
  });

  it("reports index availability on its own", () => {
    expect(actorAvailability({ index: { state: "ready" } })).toEqual({
      kind: "ready",
    });
    expect(actorAvailability({ index: { state: "stale" } })).toEqual({
      kind: "partial",
      state: "stale",
    });
    for (const state of ["not_built", "unavailable", "bogus"])
      expect(actorAvailability({ index: { state } }).kind).toBe("unavailable");
    expect(actorAvailability(undefined).kind).toBe("unavailable");
  });

  it("applies the gateway's plain-query bounds before sending", () => {
    expect(actorSearchProblem("orion group")).toBeNull();
    expect(actorSearchProblem("  ")).not.toBeNull();
    expect(actorSearchProblem("a".repeat(257))).not.toBeNull();
    expect(actorSearchProblem("bad\u0007text")).not.toBeNull();
    expect(actorSearchProblem(Array(17).fill("a").join(" "))).not.toBeNull();
  });
});
