/**
 * Shared test support helpers for React Native Web component tests.
 *
 * jsdom's `getComputedStyle` cannot fully resolve RNW 0.21's atomic CSS
 * (the `font` shorthand resets font-size/line-height and `backdrop-filter` is
 * unsupported). The real rendered contract is still deterministic: RNW writes
 * one atomic class per declaration into `document.styleSheets`; tests resolve
 * styles from those real rules (inline style last), which mirrors the cascade
 * the browser applies.
 */

type DeclarationRecorder = Record<string, string>;

/** Resolve the visual declarations that actually target `el`. */
export function sheetStyle(el: Element): DeclarationRecorder {
  const out: DeclarationRecorder = {};
  const classes = new Set(Array.from(el.classList));
  // Any inline style wins over atomic classes (same as cascade order).
  const inline = el.getAttribute("style");
  for (const sheet of document.styleSheets) {
    if (!("cssRules" in sheet)) continue;
    for (const rule of (sheet as CSSStyleSheet).cssRules) {
      const typed = rule as CSSStyleRule;
      if (!typed.selectorText) continue;
      // Atomic classes are single-class selectors (.r-*, .css-*).
      if (!typed.selectorText.startsWith(".")) continue;
      const cls = typed.selectorText.slice(1).replace(/\\(.)/g, "$1");
      if (cls.includes(":") || cls.includes(" ") || cls.includes(",")) continue;
      if (!classes.has(cls)) continue;
      const style = typed.style;
      for (let i = 0; i < style.length; i++) {
        const prop = style[i] as string;
        const value = style.getPropertyValue(prop);
        if (value !== "") out[prop as string] = value;
      }
    }
  }
  if (inline) {
    for (const decl of inline.split(";")) {
      const m = decl.match(/^\s*([-A-Za-z]+)\s*:\s*(.+)\s*$/);
      if (m) out[m[1].toLowerCase()] = m[2];
    }
  }
  return out;
}

/** Build the computed visual record for `el` including token colors.
 * Compare tokens via a canonical form: `#87bfff`, `rgba(83, 83, 83, 0.6)`.
 */
export function normColor(value: string): string {
  const v = value.replace(/\s+/g, "");
  const m = v.match(/^rgba?\((\d+),(\d+),(\d+)(?:,([\d.]+))?\)$/);
  if (!m) return v;
  const [, r, g, b, a] = m;
  if (a === undefined || Number(a) === 1) return `#${hex(r)}${hex(g)}${hex(b)}`;
  return `rgba(${Number(r)}, ${Number(g)}, ${Number(b)}, ${Number(a)})`;
}

function hex(channel: string): string {
  return Number(channel).toString(16).padStart(2, "0");
}

/** Normalized color value of the given camelCase property on `el`. */
export function paint(el: Element, camelProp: string): string {
  return normColor(sheetProp(el, camelProp));
}

/** Convenience accessor with camelCase conversion (backgroundColor etc.). */
export function sheetProp(el: Element, camelProp: string): string {
  const kebab = camelProp.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`);
  return sheetStyle(el)[kebab] ?? "";
}
