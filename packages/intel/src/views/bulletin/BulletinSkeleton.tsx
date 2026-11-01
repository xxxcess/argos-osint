import React from "react";
import { View } from "react-native";
import { Skeleton } from "@argos/ui";
import { intelColors as c, intelRadius } from "../../shell/intel-theme";
export function BulletinSkeleton() {
  return (
    <View
      role="status"
      accessibilityLabel="Loading bulletin…"
      style={{ gap: 12 }}
    >
      {[0, 1, 2].map((id) => (
        <View
          key={id}
          testID="bulletin-skeleton"
          style={{
            padding: 20,
            flexDirection: "row",
            gap: 12,
            backgroundColor: c.panel,
            borderWidth: 1,
            borderColor: c.border,
            borderRadius: intelRadius.card,
          }}
        >
          <View style={{ flex: 1, gap: 12 }}>
            <Skeleton width="45%" height={12} />
            <Skeleton lines={2} height={28} />
            <Skeleton lines={3} height={22} />
            <Skeleton width="60%" height={20} />
          </View>
          <Skeleton width={72} height={72} />
        </View>
      ))}
    </View>
  );
}
