import React from "react";
import { View, Text } from "react-native";
import { intelKindColors as kinds } from "../../shell/intel-theme";
import { briefStyles as s } from "./briefStyles";

/** Shared claim presentation without Brief lifecycle ownership. */
export function ClaimEvidenceRow({
  marker,
  text,
}: {
  marker: string;
  text: string;
}) {
  return (
    <View style={s.evidenceRow}>
      <Text style={[s.marker, { color: kinds.claim }]}>{marker}</Text>
      <Text style={s.evidenceText}>{text}</Text>
    </View>
  );
}
