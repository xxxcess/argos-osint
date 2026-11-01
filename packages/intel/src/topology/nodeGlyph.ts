/** Solid glyph tiles for CSS2D topology nodes. Chrome icons stay outside this module. */

import { fonts } from "@argos/ui";
import {
  intelColors,
  intelKindColors,
  topologyColors,
} from "../shell/intel-theme";

const icon: Record<string, string> = {
  actor:
    "M12 12a4 4 0 1 0-4-4 4 4 0 0 0 4 4zm0 2c-3.3 0-8 1.7-8 5v1h16v-1c0-3.3-4.7-5-8-5z",
  claim:
    "M4 4h16a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H9l-5 4v-4H4a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z",
  inference: "M12 3 1 21h22L12 3zm1 13h-2v-4h2zm0 3h-2v-2h2z",
  link: "M10 13a5 5 0 0 0 7.1.1l2.8-2.8a5 5 0 0 0-7.1-7.1l-1.6 1.5M14 11a5 5 0 0 0-7.1-.1L4.1 13.7a5 5 0 0 0 7.1 7.1l1.5-1.6",
  location:
    "M12 2a7 7 0 0 0-7 7c0 5.3 7 13 7 13s7-7.7 7-13a7 7 0 0 0-7-7zm0 9.5A2.5 2.5 0 1 1 14.5 9 2.5 2.5 0 0 1 12 11.5z",
  article: "M5 3h10l4 4v14H5V3zm9 1.5V8h3.5M8 12h8M8 16h8",
};

export function kindHue(kind: string): string {
  kind =
    kind === "entity" || kind === "actor_group"
      ? "actor"
      : kind === "ioc"
        ? "link"
        : kind;
  return (
    intelKindColors[kind as keyof typeof intelKindColors] ?? intelColors.accent
  );
}

export function kindIconSvg(kind: string, color: string, size = "44%"): string {
  kind =
    kind === "entity" || kind === "actor_group"
      ? "actor"
      : kind === "ioc"
        ? "link"
        : kind;
  const path = icon[kind] ?? icon.article;
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="${size}" height="${size}" aria-hidden="true"><path fill="${kind === "link" ? "none" : color}" stroke="${color}" stroke-width="1" stroke-linecap="round" d="${path}"/></svg>`;
}

export type GlyphNode = {
  label: string;
  kind: string;
  role?: string;
  imageUrl?: string | null;
};

export function createNodeGlyph(node: GlyphNode): HTMLElement {
  const color = kindHue(node.kind);
  const focused = node.role === "focus";
  const ghost = node.role === "ghost";
  const root = document.createElement("div");
  root.className = "intel-topology-node";
  root.style.setProperty("--topology-node-color", color);
  root.dataset.kind = node.kind;
  root.dataset.role = node.role || "full";
  root.style.display = "flex";
  root.style.flexDirection = "column";
  root.style.alignItems = "center";
  root.style.pointerEvents = "none";
  root.style.opacity = ghost ? "0.35" : node.role === "second" ? "0.7" : "1";
  const tile = document.createElement("div");
  const size = focused
    ? 56
    : node.role === "ghost" || node.role === "second"
      ? 32
      : 40;
  tile.dataset.tile = "node";
  tile.style.width = `${size}px`;
  tile.style.height = `${size}px`;
  tile.style.borderRadius = "9px";
  tile.style.border = `${focused ? 3 : 2}px solid ${color}`;
  tile.style.background = ghost ? "rgba(14,22,34,0.9)" : `${color}18`;
  tile.style.display = "flex";
  tile.style.alignItems = "center";
  tile.style.justifyContent = "center";
  tile.style.position = "relative";
  tile.style.overflow = "hidden";
  if (!ghost && node.role !== "second")
    tile.style.boxShadow = focused
      ? `0 0 6px ${color}, 0 0 28px ${color}66`
      : `0 0 16px ${color}33`;
  const paintIcon = () => {
    tile.innerHTML = kindIconSvg(node.kind, color);
  };
  if (node.imageUrl) {
    const image = document.createElement("img");
    image.alt = node.label;
    image.decoding = "async";
    image.src = node.imageUrl;
    image.style.width = "100%";
    image.style.height = "100%";
    image.style.objectFit = "cover";
    if (ghost) image.style.filter = "grayscale(1)";
    image.addEventListener("error", () => {
      image.remove();
      paintIcon();
    });
    tile.appendChild(image);
    const badge = document.createElement("span");
    badge.dataset.badge = "article";
    badge.style.position = "absolute";
    badge.style.right = "4px";
    badge.style.bottom = "4px";
    badge.style.width = "22px";
    badge.style.height = "22px";
    badge.innerHTML = kindIconSvg("article", topologyColors.accent);
    tile.appendChild(badge);
  } else paintIcon();
  const label = document.createElement("div");
  label.className = "intel-topology-node-label";
  label.textContent = node.label;
  label.title = node.label;
  label.style.maxWidth = "200px";
  label.style.overflow = "hidden";
  label.style.textOverflow = "ellipsis";
  label.style.whiteSpace = "nowrap";
  label.style.position = "absolute";
  label.style.top = "100%";
  label.style.marginTop = "12px";
  label.style.padding = "3px 8px";
  label.style.borderRadius = "3px";
  label.style.border = `1px solid ${color}77`;
  label.style.background = topologyColors.labelBg;
  label.style.color = color;
  label.style.font = `${focused ? 700 : 500} ${focused ? 14 : 12}px ${fonts.mono}`;
  root.append(tile, label);
  return root;
}
