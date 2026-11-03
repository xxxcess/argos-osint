import React from "react";
import { View } from "react-native";
import { Skeleton } from "@argos/ui";
import { intelColors as c, intelRadius } from "../../shell/intel-theme";
import { bulletinCardMinHeight, bulletinThumb } from "./ArticleCard";
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
            padding: 16,
            minHeight: bulletinCardMinHeight,
            flexDirection: "row",
            gap: 12,
            backgroundColor: c.panel,
            borderWidth: 1,
            borderColor: c.border,
            borderRadius: intelRadius.card,
          }}
        >
          <View style={{ flex: 1, gap: 6 }}>
            <Skeleton width="45%" height={17} />
            <Skeleton height={25} />
            <Skeleton height={23} />
            <Skeleton width="60%" height={21} />
          </View>
          <Skeleton width={bulletinThumb} height={bulletinThumb} radius={8} />
        </View>
      ))}
    </View>
  );
}
