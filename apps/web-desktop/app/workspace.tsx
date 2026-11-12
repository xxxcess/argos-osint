"use client";
import { BackendProvider } from "@argos/api-client";
import type { RegisteredWebContext } from "@argos/api-client";
import { IntelApp } from "@argos/intel";
import { useEffect, useState } from "react";
import { StyleSheet, Text, View } from "react-native";
import { Button, colors, fonts, Notice } from "@argos/ui";
import { createWebBackendSettings } from "./backend-settings";
import { createWebNavigation } from "./navigation";
import { createLiveTransport } from "./live-transport";
import { WEB_UI_SCALE } from "./ui-scale";
import {
  announceConnectionChange,
  subscribeConnectionChanges,
} from "./connection-events";

/** Persistent shell across the four screen routes; history never resets feed state. */
export function WebWorkspace({
  connection,
  expiresAt,
  basePath,
}: {
  connection: RegisteredWebContext;
  expiresAt: string;
  basePath: string;
}) {
  const [stale, setStale] = useState(false);
  const [disconnecting, setDisconnecting] = useState(false);
  const [error, setError] = useState<string>();
  const [settings] = useState(() => createWebBackendSettings(basePath));
  const [navigation] = useState(() => createWebNavigation(basePath));
  const [live] = useState(() =>
    createLiveTransport({
      basePath,
      contextRevision: connection.context_revision,
      onInvalidated: () => setStale(true),
    }),
  );
  useEffect(() => {
    const invalidate = () => {
      live.close();
      setStale(true);
    };
    const unsubscribe = subscribeConnectionChanges(invalidate);
    const timer = setTimeout(
      invalidate,
      Math.max(0, Math.min(2_147_483_647, Date.parse(expiresAt) - Date.now())),
    );
    return () => {
      unsubscribe();
      clearTimeout(timer);
      live.close();
    };
  }, [live, expiresAt]);

  const disconnect = async () => {
    if (disconnecting) return;
    setDisconnecting(true);
    try {
      const response = await fetch(`${basePath}/api/connect/logout`, {
        method: "POST",
        credentials: "same-origin",
        cache: "no-store",
        headers: { "content-type": "application/json" },
        body: "{}",
      });
      announceConnectionChange();
      live.close();
      setStale(true);
      if (!response.ok)
        setError(
          "Local connection cleared; backend revocation could not be confirmed. Reconnect to retry safely.",
        );
    } catch {
      setError(
        "Could not confirm logout; retry while the connection authority is available.",
      );
      setDisconnecting(false);
    }
  };

  if (stale)
    return (
      <View style={styles.reconnect}>
        <Text accessibilityRole="header" style={styles.title}>
          Reconnect this tab
        </Text>
        <Text style={styles.text}>
          The connection changed, expired or was revoked. Protected data and
          live subscriptions have been cleared.
        </Text>
        {error ? <Notice>{error}</Notice> : null}
        <Button
          onPress={() => window.location.assign(`${basePath}/connect`)}
          label="Reconnect registered context"
        >
          Reconnect
        </Button>
      </View>
    );

  return (
    <div
      id="intel-web-workspace"
      style={{ display: "flex", flexDirection: "column" }}
    >
      <View style={styles.bar}>
        <Text style={styles.context}>
          Workstation: {connection.workstation_ip} · Workspace:{" "}
          {connection.workspace_name}
        </Text>
        <Button
          onPress={() => window.location.assign(`${basePath}/connect`)}
          label="Change registered context"
          variant="quiet"
        >
          Change connection
        </Button>
        <Button
          onPress={() => {
            void disconnect();
          }}
          disabled={disconnecting}
          label="Disconnect Intel Web"
          variant="quiet"
        >
          {disconnecting ? "Disconnecting…" : "Disconnect"}
        </Button>
      </View>
      {error ? <Notice>{error}</Notice> : null}
      <div style={{ flex: 1, minHeight: 0 }}>
        <BackendProvider
          settings={settings}
          live={live}
          connection={connection}
          onConnectionInvalidated={() => {
            live.close();
            setStale(true);
          }}
        >
          <IntelApp navigation={navigation} displayScale={WEB_UI_SCALE} />
        </BackendProvider>
      </div>
    </div>
  );
}

const styles = StyleSheet.create({
  bar: {
    flexDirection: "row",
    alignItems: "center",
    flexWrap: "wrap",
    gap: 8,
    paddingHorizontal: 12,
    paddingVertical: 4,
    backgroundColor: colors.panel,
    borderBottomColor: colors.border,
    borderBottomWidth: 1,
  },
  context: {
    flex: 1,
    minWidth: 180,
    color: colors.dim,
    fontFamily: fonts.mono,
    fontSize: 12,
    lineHeight: 18,
  },
  reconnect: {
    minHeight: "100%",
    alignItems: "center",
    justifyContent: "center",
    gap: 20,
    padding: 24,
    backgroundColor: colors.bg,
  },
  title: {
    color: colors.text,
    fontFamily: fonts.body,
    fontSize: 24,
    lineHeight: 32,
  },
  text: {
    color: colors.dim,
    fontFamily: fonts.body,
    fontSize: 16,
    lineHeight: 24,
    maxWidth: 600,
  },
});
