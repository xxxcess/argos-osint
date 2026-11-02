import React from "react";
import { View, useWindowDimensions } from "react-native";
import { Skeleton } from "@argos/ui";
import { briefStyles as s } from "./briefStyles";
import { BriefAction } from "./BriefAction";

type EvidenceKind = "actors" | "claims" | "inferences";

function EvidenceSkeleton({ kind }: { kind: EvidenceKind }) {
  return (
    <View style={{ gap: 20, minWidth: 0 }}>
      {[0, 1].map((row) => (
        <View key={row} style={{ gap: 12 }}>
          <View style={s.evidenceRow}>
            <Skeleton width={24} height={22} />
            <View style={{ flex: 1, minWidth: 0 }}>
              <Skeleton
                lines={kind === "actors" ? 1 : 2}
                width={row === 0 ? "92%" : "76%"}
                height={22}
              />
            </View>
          </View>
          {kind === "claims" ? <Skeleton width={96} height={12} /> : null}
        </View>
      ))}
    </View>
  );
}

export function BriefSkeleton({
  label = "Loading Focus Brief…",
  variant = "page",
  onBack,
}: {
  label?: string;
  variant?: "page" | EvidenceKind;
  onBack?: () => void;
}) {
  const narrow = useWindowDimensions().width < 900;
  return (
    <View
      role="status"
      accessibilityLabel={label}
      style={variant === "page" ? s.shell : { minWidth: 0, width: "100%" }}
    >
      {variant !== "page" ? (
        <EvidenceSkeleton kind={variant} />
      ) : (
        <>
          <View style={s.header}>
            <View style={s.toolbar}>
              {onBack ? (
                <BriefAction
                  variant="back"
                  label="Return to Bulletin"
                  onPress={onBack}
                >
                  ‹ Bulletin
                </BriefAction>
              ) : (
                <Skeleton width={96} height={32} />
              )}
              <View style={s.metaLine}>
                <Skeleton width={80} height={12} />
                <Skeleton width={104} height={12} />
              </View>
            </View>
            <Skeleton width="88%" height={24} lines={2} />
          </View>
          <View
            style={{ flexDirection: narrow ? "column" : "row", minWidth: 0 }}
          >
            <View style={[s.body, { flex: 2 }, narrow && s.narrowPadding]}>
              <Skeleton lines={3} height={22} />
              {(["actors", "claims", "inferences"] as const).map((kind) => (
                <View key={kind} style={s.section}>
                  <View style={s.sectionHeading}>
                    <Skeleton width="45%" height={12} />
                  </View>
                  <View style={s.evidenceBox}>
                    <EvidenceSkeleton kind={kind} />
                  </View>
                </View>
              ))}
            </View>
            <View style={{ flex: 1, minWidth: 0, padding: 20, gap: 20 }}>
              <Skeleton width="70%" height={12} />
              <Skeleton lines={4} height={12} />
              <Skeleton lines={3} height={32} />
              <Skeleton width="65%" height={12} />
              <Skeleton lines={2} height={22} />
              <Skeleton width="80%" height={12} />
              {[0, 1].map((row) => (
                <View key={row} style={s.evidenceBox}>
                  <Skeleton width="85%" height={18} lines={2} />
                  <Skeleton width="60%" height={12} />
                </View>
              ))}
            </View>
          </View>
          <View style={s.foot}>
            <Skeleton width="55%" height={12} />
            <Skeleton width={112} height={28} />
          </View>
          <View style={s.lower}>
            <Skeleton height={44} />
          </View>
        </>
      )}
    </View>
  );
}
