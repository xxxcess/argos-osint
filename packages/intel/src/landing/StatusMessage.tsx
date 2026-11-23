"use client";
import { Text, View } from "react-native";
import { intelFonts } from "../shell/intel-fonts";

export function StatusMessage({
  message,
  color,
}: {
  message: string;
  color: string;
}) {
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
