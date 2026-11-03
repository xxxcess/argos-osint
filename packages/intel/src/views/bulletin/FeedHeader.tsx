"use client";
// @tag:react-ui — bulletin feed date filter overlays cards instead of expanding the list.
import React, { useState } from "react";
import { View, Text, Pressable, StyleSheet } from "react-native";
import { DeskButton } from "../desk-ui";
import { intelColors as c, intelRadius } from "../../shell/intel-theme";
import { intelFonts } from "../../shell/intel-fonts";

export function FeedHeader({
  sort,
  onSort,
  days,
  selectedDay,
  onDay,
  refreshing,
  onRefresh,
}: {
  sort: "latest" | "hottest";
  onSort: (sort: "latest" | "hottest") => void;
  days: string[];
  selectedDay: string;
  onDay: (day: string) => void;
  refreshing: boolean;
  onRefresh: () => void;
}) {
  const [daysOpen, setDaysOpen] = useState(false);
  return (
    <View nativeID="intel-feed-header" style={s.header}>
      <View style={s.filters}>
        {days.length > 0 ? (
          <View style={s.dayMenu}>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={`Retained day ${selectedDay}`}
              accessibilityState={{ expanded: daysOpen }}
              aria-expanded={daysOpen}
              onPress={() => setDaysOpen((open) => !open)}
              style={s.dayTrigger}
            >
              <View style={s.dayValue}>
                <Text style={s.dayCaption}>Day</Text>
                <Text style={s.dayLabel}>{selectedDay}</Text>
              </View>
              <Text style={s.dayLabel}>{daysOpen ? "▴" : "▾"}</Text>
            </Pressable>
            {daysOpen ? (
              <View
                nativeID="intel-day-overlay"
                accessibilityRole="menu"
                style={s.dayOverlay}
              >
                {days.map((day) => (
                  <Pressable
                    key={day}
                    accessibilityRole="button"
                    accessibilityLabel={`Day ${day}`}
                    accessibilityState={{ selected: day === selectedDay }}
                    onPress={() => {
                      onDay(day);
                      setDaysOpen(false);
                    }}
                    style={[
                      s.dayOption,
                      day === selectedDay && {
                        borderColor: c.accentBorder,
                        backgroundColor: c.accentSoft,
                      },
                    ]}
                  >
                    <Text style={s.dayLabel}>{day}</Text>
                  </Pressable>
                ))}
              </View>
            ) : null}
          </View>
        ) : (
          <View />
        )}
        <View style={{ flexDirection: "row", gap: 8 }}>
          {(["latest", "hottest"] as const).map((value) => (
            <DeskButton
              key={value}
              active={sort === value}
              onPress={() => onSort(value)}
            >
              {value === "latest" ? "Latest" : "Hottest"}
            </DeskButton>
          ))}
        </View>
      </View>
      <DeskButton disabled={refreshing} onPress={onRefresh}>
        Refresh feed
      </DeskButton>
    </View>
  );
}

const s = StyleSheet.create({
  header: {
    position: "relative",
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
    backgroundColor: c.inset,
    zIndex: 4,
    overflow: "visible",
  },
  filters: {
    // Keep the day menu above Refresh when the header wraps to a second row.
    zIndex: 1,
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    gap: 12,
    flexShrink: 1,
  },
  dayMenu: {
    position: "relative",
    width: 240,
    maxWidth: "100%",
    zIndex: 5,
  },
  dayTrigger: {
    minHeight: 44,
    paddingHorizontal: 16,
    borderRadius: intelRadius.control,
    borderWidth: 1,
    borderColor: c.borderStrong,
    backgroundColor: c.inset,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  dayOverlay: {
    position: "absolute",
    top: 48,
    left: 0,
    right: 0,
    zIndex: 20,
    elevation: 20,
    gap: 4,
    padding: 8,
    borderRadius: intelRadius.control,
    borderWidth: 1,
    borderColor: c.borderStrong,
    backgroundColor: c.panelRaised,
  },
  dayOption: {
    minHeight: 40,
    paddingHorizontal: 16,
    borderRadius: intelRadius.control,
    borderWidth: 1,
    borderColor: c.border,
    backgroundColor: c.panel,
    justifyContent: "center",
  },
  dayValue: { flexDirection: "row", alignItems: "center", gap: 8, minWidth: 0 },
  dayCaption: { fontFamily: intelFonts.mono, fontSize: 13, color: c.muted },
  dayLabel: { fontFamily: intelFonts.mono, fontSize: 15, color: c.text },
});
