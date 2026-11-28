"use client";
import { Text, View } from "react-native";
import { Skeleton } from "@argos/ui";
import { intelLoadingTints } from "../shell/intel-theme";
import { intelFonts } from "../shell/intel-fonts";

export function StatusMessage({
  message,
  color,
  loading = false,
}: {
  message: string;
  color: string;
  loading?: boolean;
}) {
  if (loading) {
    return (
      <View
        nativeID="intel-landing-status"
        role="status"
        aria-live="polite"
        accessibilityLabel={message}
        accessibilityState={{ busy: true }}
      >
        <Skeleton width={160} height={13} tint={intelLoadingTints.search} />
      </View>
    );
  }
  return (
    <View nativeID="intel-landing-status">
      <Text
        role="status"
        aria-live="polite"
        accessibilityLabel={message}
        style={{
          color,
          fontFamily: intelFonts.mono,
          fontSize: 13,
          textAlign: "center",
        }}
      >
        {message}
      </Text>
    </View>
  );
}
