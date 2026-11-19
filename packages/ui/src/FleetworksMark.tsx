"use client";
import { Image } from "react-native";

// Canonical assets/brand/fleetworks-mark.svg geometry. Embedding the vector keeps
// shared RN consumers independent of the web host's base path and asset loader.
const MARK = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 144 176" role="img" aria-label="Fleetworks">
  <defs>
    <radialGradient id="core" cx=".36" cy=".66" r=".69">
      <stop stop-color="#F3DBAA"/>
      <stop offset=".46" stop-color="#A87D57"/>
      <stop offset=".77" stop-color="#453B39"/>
      <stop offset="1" stop-color="#15151B"/>
    </radialGradient>
    <linearGradient id="obsidian" x2=".74" y2="1">
      <stop stop-color="#66717F"/>
      <stop offset=".47" stop-color="#252D38"/>
      <stop offset="1" stop-color="#101319"/>
    </linearGradient>
    <mask id="crescent" maskUnits="userSpaceOnUse" x="0" y="0" width="144" height="176">
      <rect width="144" height="176" fill="white"/>
      <circle cx="72" cy="69" r="50" fill="black"/>
    </mask>
  </defs>
  <circle cx="72" cy="105" r="62" fill="url(#obsidian)" stroke="#566170" stroke-width="2" mask="url(#crescent)"/>
  <circle cx="72" cy="61" r="40" fill="url(#core)" stroke="#8F8372" stroke-width="2"/>
  <g fill="url(#obsidian)" stroke="#9AA9BD" stroke-width="1.5" stroke-linejoin="round">
    <polygon points="72,3 65,17 79,17"/>
    <polygon points="41,8 33,24 48,23"/>
    <polygon points="103,8 96,23 111,24"/>
    <polygon points="17,23 10,40 26,38"/>
    <polygon points="127,23 118,38 134,40"/>
  </g>
</svg>`;
const SOURCE = { uri: `data:image/svg+xml,${encodeURIComponent(MARK)}` };

/** Decorative glass core, obsidian crescent and fixed five-satellite crown. */
export function FleetworksMark({ size = 18 }: { size?: number }) {
  return (
    <Image
      accessible={false}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      source={SOURCE}
      resizeMode="contain"
      style={{
        width: size,
        height: size + Math.max(3, Math.round(size * 0.28)),
      }}
    />
  );
}
