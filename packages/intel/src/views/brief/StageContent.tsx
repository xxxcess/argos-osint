import React from "react";
import { Notice } from "@argos/ui";
import type { BriefStage } from "@argos/api-client";
import { BriefSkeleton } from "./BriefSkeleton";

/** A generation's stage state takes precedence over older retained output. */
export function StageContent({
  stage,
  kind,
  children,
}: {
  stage?: BriefStage;
  kind: "claims" | "inferences";
  children: React.ReactNode;
}) {
  if (
    stage &&
    (/failed|cancelled|superseded|blocked/.test(stage.state) ||
      /blocked/i.test(stage.row))
  )
    return (
      <Notice>
        {stage.label}: {stage.detail || stage.row || stage.state}
      </Notice>
    );
  if (stage && stage.state !== "completed")
    return (
      <BriefSkeleton
        variant={kind}
        label={`${stage.label}: ${stage.row || stage.state}`}
      />
    );
  return <>{children}</>;
}
