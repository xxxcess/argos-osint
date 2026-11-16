// @vitest-environment node
import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { WEB_UI_SCALE } from "../app/ui-scale";

const css = readFileSync(new URL("../app/style.css", import.meta.url), "utf8");
const layout = readFileSync(
  new URL("../app/layout.tsx", import.meta.url),
  "utf8",
);
const workspace = readFileSync(
  new URL("../app/workspace.tsx", import.meta.url),
  "utf8",
);

describe("web UI scale", () => {
  it("has one number in the stylesheet and it equals the TypeScript constant", () => {
    const declared = [...css.matchAll(/--intel-ui-scale:\s*([0-9.]+)\s*;/g)];
    expect(declared).toHaveLength(1);
    expect(Number(declared[0][1])).toBe(WEB_UI_SCALE);
    expect(WEB_UI_SCALE).toBeGreaterThan(0);
    expect(WEB_UI_SCALE).toBeLessThanOrEqual(1);
  });

  it("derives the page zoom, content width and the graph's inverse zoom from that variable", () => {
    expect(css).toMatch(/html\s*{\s*zoom:\s*var\(--intel-ui-scale\);/);
    expect(css).toContain("calc(1440px / var(--intel-ui-scale))");
    expect(css).toContain("zoom: calc(1 / var(--intel-ui-scale))");
    // No second hard-coded copy of the number anywhere else in the stylesheet.
    expect(css.match(/0\.75/g)).toHaveLength(1);
  });

  it("feeds the same constant to the layout variable and the graph viewport", () => {
    expect(layout).toContain('"--intel-ui-scale": WEB_UI_SCALE');
    expect(workspace).toContain("displayScale={WEB_UI_SCALE}");
  });
});
