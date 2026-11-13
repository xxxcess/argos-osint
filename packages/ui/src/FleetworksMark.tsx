"use client";
import { Image } from "react-native";

// Canonical assets/brand/fleetworks-mark.svg geometry. Embedding the vector keeps
// shared RN consumers independent of the web host's base path and asset loader.
const MARK = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" role="img" aria-label="Fleetworks">
  <g fill="none" stroke="#C6A56E" stroke-width="2.4" stroke-linejoin="round">
    <polygon points="128,28 140,50 116,50"/>
    <polygon points="78,52 90,74 66,74"/>
    <polygon points="178,52 190,74 166,74"/>
    <polygon points="46,92 58,114 34,114"/>
    <polygon points="210,92 222,114 198,114"/>
  </g>
  <defs>
    <mask id="crescent" maskUnits="userSpaceOnUse" x="0" y="0" width="256" height="256">
      <rect width="256" height="256" fill="white"/>
      <circle cx="128" cy="132" r="50" fill="black"/>
    </mask>
  </defs>
  <circle cx="128" cy="168" r="62" fill="#C4A265" mask="url(#crescent)"/>
  <circle cx="128" cy="124" r="40" fill="#F3EEE4"/>
</svg>`;
const SOURCE = { uri: `data:image/svg+xml,${encodeURIComponent(MARK)}` };

/** Decorative five outlined rays, ivory disc and transparent gold crescent. */
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
