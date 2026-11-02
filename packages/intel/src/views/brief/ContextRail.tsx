"use client";
import React, { useEffect, useState } from "react";
import { View, Text, Image, Linking } from "react-native";
import {
  Button,
  Label,
  Copy,
  Notice,
  EmptyState,
  LoadingState,
} from "@argos/ui";
import type {
  BriefStage,
  Article,
  Brief,
  useRead,
  useAction,
  useRelated,
} from "@argos/api-client";
import { StageContent } from "./StageContent";
import { JobStatusBeacon } from "../../shell/JobStatusBeacon";
import { indexingJobState, type JobState } from "../../shell/jobState";
import { intelStatus, intelColors } from "../../shell/intel-theme";
const s = { small: { color: intelColors.dim, fontSize: 13 } };
function JsonLines({ value }: { value: unknown }) {
  function lines(v: unknown): string[] {
    if (Array.isArray(v)) return v.flatMap(lines);
    if (typeof v === "string") return [v];
    if (!v || typeof v !== "object") return [];
    const object = v as Record<string, unknown>;
    const own = [
      "text",
      "claim",
      "inference_text",
      "why",
      "verification_needed",
      "interpretation",
      "empty_reason",
    ].flatMap((key) =>
      typeof object[key] === "string" && object[key]
        ? [String(object[key])]
        : [],
    );
    return own.length
      ? own
      : Object.entries(object)
          .filter(([key]) =>
            ["claims", "inferences", "leads", "links"].includes(key),
          )
          .flatMap(([, v]) => lines(v));
  }
  const items = lines(value);
  return items.length ? (
    <View style={{ gap: 12 }}>
      {items.map((line, i) => (
        <Copy key={i}>{line}</Copy>
      ))}
    </View>
  ) : (
    <EmptyState title="No retained output" />
  );
}

function safeLink(raw: string) {
  try {
    return ["http:", "https:"].includes(new URL(raw).protocol);
  } catch {
    return false;
  }
}
function BriefSourceImage({ uri, title }: { uri: string; title: string }) {
  const [aspectRatio, setAspectRatio] = useState(16 / 9);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let active = true;
    Image.getSize(
      uri,
      (width, height) => {
        if (active && width > 0 && height > 0) setAspectRatio(width / height);
      },
      () => {},
    );
    return () => {
      active = false;
    };
  }, [uri]);
  if (failed) return null;
  return (
    <Image
      accessibilityLabel={title}
      source={{ uri }}
      style={{ width: "100%", aspectRatio, maxHeight: 520 }}
      resizeMode="contain"
      onError={() => setFailed(true)}
    />
  );
}

export function ContextRail({
  a,
  d,
  shown,
  detail,
  related,
  ensure,
  reload,
  backend,
  jobState,
  linksStage,
}: {
  a: Article;
  d?: Brief;
  shown: Article;
  detail: ReturnType<typeof useRead<Brief>>;
  related: ReturnType<typeof useRelated>;
  ensure: ReturnType<typeof useAction<{ job_id: string }>>;
  reload: ReturnType<typeof useAction<{ job_id: string }>>;
  backend: { createRequestId: () => string };
  jobState: JobState;
  linksStage?: BriefStage;
}) {
  const source = (
    <View style={{ gap: 16 }}>
      {safeLink(shown.image_url) && (
        <BriefSourceImage
          key={shown.image_url}
          uri={shown.image_url}
          title={shown.title}
        />
      )}
      {safeLink(shown.url) && (
        <Button
          variant="quiet"
          onPress={() => {
            void Linking.openURL(shown.url);
          }}
        >
          Read original source ↗
        </Button>
      )}
    </View>
  );
  const context = (
    <View style={{ gap: 16, minWidth: 0 }}>
      <Text accessibilityRole="header" style={s.small}>
        RETAINED BRIEF CONTROLS
      </Text>
      {!d && detail.isLoading ? (
        <LoadingState label="Loading brief status…" compact />
      ) : d ? (
        <>
          <JobStatusBeacon state={jobState} />
          <Label>Brief lifecycle</Label>
          <Copy>{d.status || "Status unavailable"}</Copy>
          <View
            role="list"
            accessibilityLabel="Brief lifecycle timeline"
            style={{ gap: 12 }}
          >
            {[
              {
                label: "Extraction",
                value: d.lifecycle || "unavailable",
                state: d.publication ? "succeeded" : jobState,
              },
              {
                label: "Publication",
                value: d.publication
                  ? "Retained canonical brief"
                  : "Not published",
                state: d.publication ? "succeeded" : "idle",
              },
              {
                label: "Indexing",
                value: `${d.indexing?.state || "Not indexed"}${d.indexing?.reason ? ` · ${d.indexing.reason}` : ""}`,
                state: indexingJobState(d.indexing?.state),
              },
            ].map((step) => (
              <View
                role="listitem"
                key={step.label}
                style={{
                  flexDirection: "row",
                  gap: 8,
                  alignItems: "center",
                  borderLeftWidth: 1,
                  borderColor: intelColors.border,
                  paddingLeft: 8,
                }}
              >
                <View
                  aria-hidden
                  style={{
                    width: 6,
                    height: 6,
                    borderRadius: 3,
                    backgroundColor: intelStatus[step.state as JobState].color,
                  }}
                />
                <Text style={s.small}>
                  {step.label}: {step.value}
                </Text>
              </View>
            ))}
          </View>
        </>
      ) : (
        <EmptyState title="Brief status unavailable" />
      )}
      <Button
        variant="primary"
        disabled={ensure.isPending}
        onPress={() => ensure.mutate({ run: a.run_id, id: a.id })}
      >
        {ensure.isPending ? "Admitting…" : "Ensure / resume brief"}
      </Button>
      <Button
        variant="quiet"
        disabled={reload.isPending}
        onPress={() =>
          reload.mutate({
            run: a.run_id,
            id: a.id,
            request_id: backend.createRequestId(),
          })
        }
      >
        Reload article / new generation
      </Button>
      {reload.error && <Notice>{reload.error.message}</Notice>}
      <Button
        variant="quiet"
        onPress={() => {
          void detail.refetch();
          void related.refetch();
        }}
      >
        Refresh retained brief
      </Button>
      <Label>Links</Label>
      <StageContent stage={linksStage} kind="inferences">
        {d ? (
          <JsonLines value={d.links} />
        ) : !detail.isLoading ? (
          <EmptyState title="Links unavailable" />
        ) : null}
      </StageContent>
    </View>
  );
  return (
    <View style={{ gap: 20, minWidth: 0 }}>
      {context}
      {source}
    </View>
  );
}
