"use client";
import React, { useEffect, useState } from "react";
import { View, Text, Pressable, Animated } from "react-native";
import { usePresence, useReducedMotion } from "../../shell/motion";
import { intelColors as c } from "../../shell/intel-theme";
import { intelFonts } from "../../shell/intel-fonts";
export function BriefSection({
  title,
  children,
  color = c.muted,
  icon: _icon,
  count,
}: {
  title: string;
  children: React.ReactNode;
  color?: string;
  icon?: "actor" | "claim" | "inference" | "source" | "entities";
  count?: number;
}) {
  const [open, setOpen] = useState(true);
  const presence = usePresence(open);
  const reduced = useReducedMotion();
  const [extent, setExtent] = useState(0);
  const [height] = useState(() => new Animated.Value(0));
  useEffect(() => {
    const animation = Animated.timing(height, {
      toValue: open ? extent : 0,
      duration: reduced ? 0 : 200,
      useNativeDriver: false,
    });
    animation.start();
    return () => animation.stop();
  }, [open, extent, reduced, height]);
  return (
    <View style={{ gap: 12, minWidth: 0 }}>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={title}
        accessibilityState={{ expanded: open }}
        aria-expanded={open}
        onPress={() => setOpen((v) => !v)}
        style={{
          flexDirection: "row",
          alignItems: "center",
          gap: 12,
          borderBottomWidth: 1,
          borderColor: c.divider,
          paddingVertical: 12,
        }}
      >
        <Text
          accessibilityRole="header"
          style={{
            fontFamily: intelFonts.mono,
            fontSize: 12,
            fontWeight: "700",
            color,
            flex: 1,
          }}
        >
          {title}
        </Text>
        {count != null ? (
          <Text
            accessibilityLabel={`${count} items`}
            style={{ color: c.dim, fontSize: 12 }}
          >
            {count}
          </Text>
        ) : null}
        <Text style={{ color: c.dim }}>{open ? "−" : "+"}</Text>
      </Pressable>
      <Animated.View
        aria-hidden={!open}
        style={[
          presence,
          {
            height: extent ? height : open ? undefined : 0,
            overflow: "hidden",
          },
        ]}
      >
        <View
          onLayout={(event) => {
            const measured = event.nativeEvent.layout.height;
            if (!extent && open) height.setValue(measured);
            setExtent(measured);
          }}
        >
          {children}
        </View>
      </Animated.View>
    </View>
  );
}
