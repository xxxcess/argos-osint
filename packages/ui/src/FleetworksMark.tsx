"use client";
import { View } from "react-native";

const CREAM = "#F3EEE4";
const GOLD = "#C4A265";

/** Sun, rays, and crescent used by Fleetworks apps. */
export function FleetworksMark({ size = 18 }: { size?: number }) {
  const ray = Math.max(3, Math.round(size * 0.28));
  const disk = Math.round(size * 0.62);
  const moon = Math.round(size * 0.9);
  return (
    <View
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={{ width: size, height: size + ray, alignItems: "center" }}
    >
      <View style={{ flexDirection: "row", alignItems: "flex-end", gap: 1 }}>
        <Ray size={ray - 1} />
        <Ray size={ray} />
        <Ray size={ray - 1} />
      </View>
      <View style={{ width: size, height: disk + 4, alignItems: "center" }}>
        <View
          style={{
            position: "absolute",
            bottom: 0,
            width: moon,
            height: Math.round(moon * 0.46),
            borderBottomLeftRadius: moon,
            borderBottomRightRadius: moon,
            backgroundColor: GOLD,
          }}
        />
        <View
          style={{
            width: disk,
            height: disk,
            borderRadius: disk / 2,
            backgroundColor: CREAM,
          }}
        />
      </View>
    </View>
  );
}

function Ray({ size }: { size: number }) {
  return (
    <View
      style={{
        width: 0,
        height: 0,
        borderLeftWidth: size / 2,
        borderRightWidth: size / 2,
        borderBottomWidth: size,
        borderLeftColor: "transparent",
        borderRightColor: "transparent",
        borderBottomColor: GOLD,
      }}
    />
  );
}
