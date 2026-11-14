"use client";

import { useState } from "react";
import { StyleSheet, Text, TextInput, View } from "react-native";
import { Button, colors, Copy, fonts, Notice, Panel } from "@argos/ui";
import { parseConnectId } from "../../connect-id.mjs";
import { announceConnectionChange } from "../connection-events";

async function resolveConnection(basePath: string, connectId: string) {
  const response = await fetch(`${basePath}/api/connect`, {
    method: "POST",
    credentials: "same-origin",
    cache: "no-store",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ connect_id: connectId }),
  });
  const body = await response.json();
  if (!response.ok) {
    throw new Error(
      typeof body.error === "string"
        ? body.error
        : "Connection could not be verified",
    );
  }
}

function failureMessage(failure: unknown, fallback: string) {
  return failure instanceof Error ? failure.message : fallback;
}

/** This prompt resolves an existing registration; it never creates a node/folder. */
export function ConnectForm({
  returnPath,
  basePath,
}: {
  returnPath: string;
  basePath: string;
}) {
  const [connectId, setConnectId] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string>();

  const connect = async () => {
    if (pending) return;
    let canonical: string;
    try {
      canonical = parseConnectId(connectId).connectId;
    } catch (failure) {
      setError(failureMessage(failure, "Enter a valid registered Connect ID"));
      return;
    }
    setPending(true);
    setError(undefined);
    try {
      await resolveConnection(basePath, canonical);
      announceConnectionChange();
      // Full navigation discards the previous Query client and cached RSC payloads.
      window.location.replace(returnPath);
    } catch (failure) {
      setError(
        failureMessage(failure, "Connection authority unavailable; retry"),
      );
      setPending(false);
    }
  };

  return (
    <View style={styles.page}>
      <View style={styles.content}>
        <Text accessibilityRole="header" style={styles.heading}>
          Connect to Intel Web
        </Text>
        <Copy>
          Enter the Connect ID returned by Fleetworks for an existing registered
          workstation. Its identity stays the same when its IP changes.
        </Copy>
        <Panel title="Registered context">
          <Text nativeID="connect-id-label" style={styles.label}>
            Connect ID
          </Text>
          <TextInput
            accessibilityLabel="Connect ID"
            accessibilityLabelledBy="connect-id-label"
            autoCapitalize="none"
            autoCorrect={false}
            editable={!pending}
            maxLength={128}
            placeholder="Workstation ID from the native app"
            placeholderTextColor={colors.muted}
            value={connectId}
            onChangeText={setConnectId}
            onSubmitEditing={() => {
              void connect();
            }}
            returnKeyType="go"
            style={styles.input}
          />
          <View style={styles.actions}>
            <Button
              onPress={() => {
                void connect();
              }}
              disabled={pending || !connectId}
              label="Connect registered workstation"
              variant="primary"
            >
              {pending ? "Verifying…" : "Connect"}
            </Button>
          </View>
          {error ? <Notice>{error}</Notice> : null}
          <Text accessibilityLiveRegion="polite" style={styles.help}>
            {pending
              ? "Checking the registration in this web deployment."
              : "Unknown, revoked or other-backend IDs cannot unlock this deployment. Configure and register in the native app first."}
          </Text>
        </Panel>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  page: {
    minHeight: "100%",
    backgroundColor: colors.bg,
    justifyContent: "center",
    alignItems: "center",
    padding: 24,
  },
  content: { width: "100%", maxWidth: 560, gap: 20 },
  heading: {
    color: colors.text,
    fontFamily: fonts.body,
    fontSize: 28,
    lineHeight: 36,
    fontWeight: "600",
  },
  label: {
    color: colors.text,
    fontFamily: fonts.mono,
    fontSize: 14,
    lineHeight: 22,
    marginBottom: 8,
  },
  input: {
    color: colors.text,
    backgroundColor: colors.bg,
    borderWidth: 1,
    borderColor: colors.border,
    borderRadius: 6,
    minHeight: 44,
    paddingHorizontal: 12,
    fontFamily: fonts.mono,
    fontSize: 15,
  },
  actions: { flexDirection: "row", marginVertical: 16 },
  help: {
    color: colors.dim,
    fontFamily: fonts.body,
    fontSize: 14,
    lineHeight: 22,
  },
});
