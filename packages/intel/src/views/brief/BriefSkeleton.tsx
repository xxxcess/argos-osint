import React from "react";
import { View } from "react-native";
import { Skeleton } from "@argos/ui";
export function BriefSkeleton({
  label = "Loading Focus Brief…",
}: {
  label?: string;
}) {
  return (
    <View
      role="status"
      accessibilityLabel={label}
      style={{ gap: 12, minWidth: 0, width: "100%" }}
    >
      <Skeleton width="55%" height={12} />
      <Skeleton lines={2} height={28} />
      <Skeleton lines={4} height={22} />
      <Skeleton lines={3} height={22} />
      <View style={{ flexDirection: "row", gap: 8 }}>
        <Skeleton width={72} height={28} />
        <Skeleton width={72} height={28} />
        <Skeleton width={72} height={28} />
      </View>
    </View>
  );
}
